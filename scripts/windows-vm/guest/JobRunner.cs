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
        /// What is left of the job's time, measured when Clock started: a
        /// monotonic budget, so neither a delay before the command starts nor
        /// a change to Windows' clock can stretch it.
        public uint TimeoutMs;
        public System.Diagnostics.Stopwatch Clock;
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

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
        static extern SafeFileHandle CreateFile(string name, uint access, uint share, IntPtr security,
            uint disposition, uint flags, IntPtr template);

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

        [StructLayout(LayoutKind.Sequential)]
        struct JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
        {
            public long TotalUserTime, TotalKernelTime, ThisPeriodTotalUserTime, ThisPeriodTotalKernelTime;
            public uint TotalPageFaultCount, TotalProcesses, ActiveProcesses, TotalTerminatedProcesses;
        }

        const int JobObjectBasicAccountingInformation = 1;
        const int TERMINATION_WAIT_MS = 10000;

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern bool QueryInformationJobObject(SafeFileHandle job, int infoClass,
            out JOBOBJECT_BASIC_ACCOUNTING_INFORMATION info, int length, IntPtr returnLength);

        /// Runs the job's command line with stdout and stderr written to its
        /// two files, and ends its whole tree at the deadline or once the
        /// command itself exits.
        public static JobResult Run(JobSpec spec)
        {
            if (MillisecondsLeft(spec) == 0)
            {
                // Its deadline passed while it waited in the queue: it never starts.
                return TimedOut();
            }
            using (SafeFileHandle job = CreateKillOnCloseJob())
            using (FileStream stdout = InheritableFile(spec.StdoutPath))
            using (FileStream stderr = InheritableFile(spec.StderrPath))
            {
                PROCESS_INFORMATION process = StartSuspended(spec.CommandLine, spec.WorkDir, stdout, stderr);
                try
                {
                    return RunInJob(job, process, spec);
                }
                finally
                {
                    // Whatever the command left running ends with it, before
                    // the result is written: a descendant still holding the
                    // output files would keep the client from reading them.
                    try
                    {
                        TerminateJobObject(job, TIMED_OUT_EXIT_CODE);
                        WaitForEmptyJob(job);
                    }
                    finally
                    {
                        CloseHandle(process.hThread);
                        CloseHandle(process.hProcess);
                    }
                }
            }
        }

        static JobResult TimedOut()
        {
            return new JobResult { ExitCode = TIMED_OUT_EXIT_CODE, TimedOut = true };
        }

        static uint MillisecondsLeft(JobSpec spec)
        {
            long left = spec.TimeoutMs - spec.Clock.ElapsedMilliseconds;
            return left <= 0 ? 0 : (uint)left;
        }

        static JobResult RunInJob(SafeFileHandle job, PROCESS_INFORMATION process, JobSpec spec)
        {
            if (!AssignProcessToJobObject(job, process.hProcess))
            {
                // Outside the job, the suspended process would outlive it.
                int error = Marshal.GetLastWin32Error();
                TerminateProcess(process.hProcess, 1);
                throw new Win32Exception(error, "AssignProcessToJobObject failed");
            }
            if (MillisecondsLeft(spec) == 0)
            {
                // Still suspended: the caller's TerminateJobObject ends it unrun.
                return TimedOut();
            }
            Check(ResumeThread(process.hThread) != 0xFFFFFFFF, "ResumeThread");
            // Measured again: the wait is relative, and time passed since the check.
            uint waited = WaitForSingleObject(process.hProcess, MillisecondsLeft(spec));
            Check(waited != WAIT_FAILED, "WaitForSingleObject");
            if (waited == WAIT_TIMEOUT)
            {
                return TimedOut();
            }
            uint code;
            Check(GetExitCodeProcess(process.hProcess, out code), "GetExitCodeProcess");
            return new JobResult { ExitCode = unchecked((int)code), TimedOut = false };
        }

        /// Waits, up to TERMINATION_WAIT_MS, until no process is left in the
        /// job. Termination is asynchronous and the job object signals no
        /// such event, so this checks its count every 50 ms.
        static void WaitForEmptyJob(SafeFileHandle job)
        {
            var deadline = DateTime.UtcNow.AddMilliseconds(TERMINATION_WAIT_MS);
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION info;
            int size = Marshal.SizeOf(typeof(JOBOBJECT_BASIC_ACCOUNTING_INFORMATION));
            while (QueryInformationJobObject(job, JobObjectBasicAccountingInformation, out info, size, IntPtr.Zero)
                && info.ActiveProcesses > 0)
            {
                if (DateTime.UtcNow >= deadline)
                {
                    throw new TimeoutException("the job's processes did not end within " + TERMINATION_WAIT_MS + " ms");
                }
                System.Threading.Thread.Sleep(50);
            }
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

        const uint HANDLE_FLAG_INHERIT = 1;

        static FileStream InheritableFile(string path)
        {
            var stream = new FileStream(path, FileMode.Create, FileAccess.Write, FileShare.Read);
            Check(SetHandleInformation(stream.SafeFileHandle.DangerousGetHandle(), HANDLE_FLAG_INHERIT,
                HANDLE_FLAG_INHERIT), "SetHandleInformation");
            return stream;
        }

        /// NUL for the job's stdin, so a program that reads it sees its end
        /// rather than an invalid handle. FileStream refuses devices.
        static SafeFileHandle InheritableNul()
        {
            const uint GENERIC_READ = 0x80000000, FILE_SHARE_READ_WRITE = 3, OPEN_EXISTING = 3;
            SafeFileHandle nul = CreateFile("NUL", GENERIC_READ, FILE_SHARE_READ_WRITE, IntPtr.Zero, OPEN_EXISTING,
                0, IntPtr.Zero);
            Check(!nul.IsInvalid, "CreateFile NUL");
            Check(SetHandleInformation(nul.DangerousGetHandle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT),
                "SetHandleInformation");
            return nul;
        }

        static PROCESS_INFORMATION StartSuspended(string commandLine, string workDir, FileStream stdout,
            FileStream stderr)
        {
            // CreateProcess copies the handle, so this one closes once it returns.
            using (SafeFileHandle stdin = InheritableNul())
            {
                var startup = new STARTUPINFO();
                startup.cb = Marshal.SizeOf(typeof(STARTUPINFO));
                startup.dwFlags = STARTF_USESTDHANDLES;
                startup.hStdInput = stdin.DangerousGetHandle();
                startup.hStdOutput = stdout.SafeFileHandle.DangerousGetHandle();
                startup.hStdError = stderr.SafeFileHandle.DangerousGetHandle();
                PROCESS_INFORMATION process;
                uint flags = CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT;
                Check(CreateProcess(null, new System.Text.StringBuilder(commandLine), IntPtr.Zero, IntPtr.Zero,
                    true, flags, IntPtr.Zero, workDir, ref startup, out process), "CreateProcess");
                return process;
            }
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
