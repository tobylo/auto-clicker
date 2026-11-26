<#
.SYNOPSIS
Names the window in front of tester's desktop and whether it runs elevated.

.DESCRIPTION
Run through the elevated runner by `pc status`, which can read any
process's integrity level. Windows drops input injected by an unelevated
program into a window whose process has a higher integrity level (UIPI), so
an elevated window left in front makes every click test fail. Prints one
line, "<process> (<pid>) '<title>' <level>", or "no window in front" (as
after a window closes, until another is activated), and exits 0.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class Wn13Focus
{
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint pid);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetWindowText(IntPtr window, StringBuilder text, int size);
    [DllImport("kernel32.dll", SetLastError = true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)] static extern bool GetTokenInformation(IntPtr token, int cls, IntPtr info, int size, out int needed);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthority(IntPtr sid, uint index);
    [DllImport("advapi32.dll")] static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);

    const uint PROCESS_QUERY_LIMITED_INFORMATION = 0x1000, TOKEN_QUERY = 0x0008;
    const int TokenIntegrityLevel = 25;

    public static uint Pid;
    public static string Title;

    /// False when no window is in front.
    public static bool Read()
    {
        IntPtr window = GetForegroundWindow();
        if (window == IntPtr.Zero) return false;
        GetWindowThreadProcessId(window, out Pid);
        StringBuilder text = new StringBuilder(512);
        GetWindowText(window, text, text.Capacity);
        Title = text.ToString();
        return true;
    }

    /// The integrity level's RID of process pid: 0x2000 medium, 0x3000
    /// high (elevated), 0x4000 system; -1 when it cannot be read.
    public static int Integrity(uint pid)
    {
        IntPtr process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid);
        if (process == IntPtr.Zero) return -1;
        IntPtr token;
        bool opened = OpenProcessToken(process, TOKEN_QUERY, out token);
        CloseHandle(process);
        if (!opened) return -1;
        IntPtr info = Marshal.AllocHGlobal(256);
        try
        {
            int needed;
            if (!GetTokenInformation(token, TokenIntegrityLevel, info, 256, out needed)) return -1;
            IntPtr sid = Marshal.ReadIntPtr(info);
            int count = Marshal.ReadByte(GetSidSubAuthorityCount(sid));
            return Marshal.ReadInt32(GetSidSubAuthority(sid, (uint)(count - 1)));
        }
        finally
        {
            Marshal.FreeHGlobal(info);
            CloseHandle(token);
        }
    }
}
'@

if (-not [Wn13Focus]::Read()) {
    Write-Output 'no window in front'
    exit 0
}
$process = Get-Process -Id ([Wn13Focus]::Pid) -ErrorAction SilentlyContinue
$name = if ($process) { $process.ProcessName } else { 'gone' }
$level = [Wn13Focus]::Integrity([Wn13Focus]::Pid)
$text = switch ($level) {
    { $_ -lt 0 } { 'integrity unknown'; break }
    { $_ -ge 0x3000 } { 'elevated: Windows drops input injected by unelevated programs into it'; break }
    default { 'not elevated' }
}
Write-Output ("{0} ({1}) '{2}' {3}" -f $name, [Wn13Focus]::Pid, [Wn13Focus]::Title, $text)
