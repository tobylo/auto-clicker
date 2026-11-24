// Runs one runner job: a command line in its own Win32 job object, so the
// whole process tree ends when the job does, at its deadline, or when the
// runner itself dies (the job is kill-on-close and only the runner holds it).
// Loaded by runner.ps1 with Add-Type.

using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;

namespace Wn01
{
    /// What to run: a command line, where, where its output goes, and its deadline.
    public sealed class JobSpec
    {
        public string CommandLine;
        public string WorkDir;
        public string StdoutPath;
        public string StderrPath;
        public uint TimeoutMs;
    }

    public sealed class JobResult
    {
        public int ExitCode;
        public bool TimedOut;
    }

    public static class JobRunner
    {
        const uint CREATE_SUSPENDED = 0x00000004;
        const uint CREATE_NO_WINDOW = 0x08000000;
        const uint CREATE_UNICODE_ENVIRONMENT = 0x00000400;
        const int STARTF_USESTDHANDLES = 0x00000100;
        const uint WAIT_TIMEOUT = 0x00000102;
        const uint WAIT_FAILED = 0xFFFFFFFF;
        const int JobObjectExtendedLimitInformation = 9;
        const uint JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000;
        const int TIMED_OUT_EXIT_CODE = 124;

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        struct STARTUPINFO
        {
            public int cb;
            public string lpReserved, lpDesktop, lpTitle;
            public int dwX, dwY, dwXSize, dwYSize, dwXCountChars, dwYCountChars, dwFillAttribute, dwFlags;
            public short wShowWindow, cbReserved2;
            public IntPtr lpReserved2, hStdInput, hStdOutput, hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct PROCESS_INFORMATION
        {
            public IntPtr hProcess, hThread;
            public int dwProcessId, dwThreadId;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct IO_COUNTERS
        {
            public ulong ReadOperationCount, WriteOperationCount, OtherOperationCount;
            public ulong ReadTransferCount, WriteTransferCount, OtherTransferCount;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_BASIC_LIMIT_INFORMATION
        {
            public long PerProcessUserTimeLimit, PerJobUserTimeLimit;
            public uint LimitFlags;
            public UIntPtr MinimumWorkingSetSize, MaximumWorkingSetSize;
            public uint ActiveProcessLimit;
            public UIntPtr Affinity;
            public uint PriorityClass, SchedulingClass;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION
        {
            public JOBOBJECT_BASIC_LIMIT_INFORMATION BasicLimitInformation;
            public IO_COUNTERS IoInfo;
            public UIntPtr ProcessMemoryLimit, JobMemoryLimit, PeakProcessMemoryUsed, PeakJobMemoryUsed;
        }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        static extern SafeFileHandle CreateJobObject(IntPtr attributes, string name);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetInformationJobObject(SafeFileHandle job, int infoClass,
            ref JOBOBJECT_EXTENDED_LIMIT_INFORMATION info, int length);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool AssignProcessToJobObject(SafeFileHandle job, IntPtr process);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool TerminateJobObject(SafeFileHandle job, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        static extern bool CreateProcess(string application, System.Text.StringBuilder commandLine,
            IntPtr processAttributes, IntPtr threadAttributes, bool inheritHandles, uint flags,
            IntPtr environment, string currentDirectory, ref STARTUPINFO startupInfo,
            out PROCESS_INFORMATION processInformation);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool TerminateProcess(IntPtr process, uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint ResumeThread(IntPtr thread);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool GetExitCodeProcess(IntPtr process, out uint exitCode);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool CloseHandle(IntPtr handle);

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool SetHandleInformation(IntPtr handle, uint mask, uint flags);

        /// Runs the job's command line with stdout and stderr written to its
        /// two files, and ends its whole tree at the deadline or once the
        /// command itself exits.
        public static JobResult Run(JobSpec spec)
        {
            using (SafeFileHandle job = CreateKillOnCloseJob())
            using (FileStream stdout = InheritableFile(spec.StdoutPath))
            using (FileStream stderr = InheritableFile(spec.StderrPath))
            {
                PROCESS_INFORMATION process = StartSuspended(spec.CommandLine, spec.WorkDir, stdout, stderr);
                try
                {
                    return RunInJob(job, process, spec.TimeoutMs);
                }
                finally
                {
                    // Whatever the command left running ends with it.
                    TerminateJobObject(job, TIMED_OUT_EXIT_CODE);
                    CloseHandle(process.hThread);
                    CloseHandle(process.hProcess);
                }
            }
        }

        static JobResult RunInJob(SafeFileHandle job, PROCESS_INFORMATION process, uint timeoutMs)
        {
            if (!AssignProcessToJobObject(job, process.hProcess))
            {
                // Outside the job, the suspended process would outlive it.
                int error = Marshal.GetLastWin32Error();
                TerminateProcess(process.hProcess, 1);
                throw new Win32Exception(error, "AssignProcessToJobObject failed");
            }
            Check(ResumeThread(process.hThread) != 0xFFFFFFFF, "ResumeThread");
            uint waited = WaitForSingleObject(process.hProcess, timeoutMs);
            Check(waited != WAIT_FAILED, "WaitForSingleObject");
            if (waited == WAIT_TIMEOUT)
            {
                return new JobResult { ExitCode = TIMED_OUT_EXIT_CODE, TimedOut = true };
            }
            uint code;
            Check(GetExitCodeProcess(process.hProcess, out code), "GetExitCodeProcess");
            return new JobResult { ExitCode = unchecked((int)code), TimedOut = false };
        }

        static SafeFileHandle CreateKillOnCloseJob()
        {
            SafeFileHandle job = CreateJobObject(IntPtr.Zero, null);
            Check(!job.IsInvalid, "CreateJobObject");
            var info = new JOBOBJECT_EXTENDED_LIMIT_INFORMATION();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            Check(SetInformationJobObject(job, JobObjectExtendedLimitInformation, ref info,
                Marshal.SizeOf(typeof(JOBOBJECT_EXTENDED_LIMIT_INFORMATION))), "SetInformationJobObject");
            return job;
        }

        static FileStream InheritableFile(string path)
        {
            var stream = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read);
            const uint HANDLE_FLAG_INHERIT = 1;
            Check(SetHandleInformation(stream.SafeFileHandle.DangerousGetHandle(), HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT), "SetHandleInformation");
            return stream;
        }

        static PROCESS_INFORMATION StartSuspended(string commandLine, string workDir, FileStream stdout,
            FileStream stderr)
        {
            var startup = new STARTUPINFO();
            startup.cb = Marshal.SizeOf(typeof(STARTUPINFO));
            startup.dwFlags = STARTF_USESTDHANDLES;
            startup.hStdOutput = stdout.SafeFileHandle.DangerousGetHandle();
            startup.hStdError = stderr.SafeFileHandle.DangerousGetHandle();
            PROCESS_INFORMATION process;
            uint flags = CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT;
            Check(CreateProcess(null, new System.Text.StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero,
                true, flags, IntPtr.Zero, workDir, ref startup, out process), "CreateProcess");
            return process;
        }

        static void Check(bool ok, string call)
        {
            if (!ok)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(), call + " failed");
            }
        }
    }
}
