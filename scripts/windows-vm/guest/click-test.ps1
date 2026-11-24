<#
.SYNOPSIS
Proves injected input reaches a window in this desktop.

.DESCRIPTION
Run through the runner by `vm selftest`. Opens a window, brings it to the
front, sends one left click at its centre with SendInput, and exits 0 when the
window received that click, 1 when it did not within 10 seconds, 2 when
SendInput refused the input.
#>

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class Wn01Input
{
    [StructLayout(LayoutKind.Sequential)]
    struct MOUSEINPUT { public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo; }

    [StructLayout(LayoutKind.Explicit, Size = 40)]
    struct INPUT { [FieldOffset(0)] public uint type; [FieldOffset(8)] public MOUSEINPUT mi; }

    const uint INPUT_MOUSE = 0;
    const uint MOVE = 0x0001, LEFTDOWN = 0x0002, LEFTUP = 0x0004, ABSOLUTE = 0x8000, VIRTUALDESK = 0x4000;

    [DllImport("user32.dll", SetLastError = true)]
    static extern uint SendInput(uint count, INPUT[] inputs, int size);

    [DllImport("user32.dll")]
    static extern int GetSystemMetrics(int index);

    /// Clicks at a screen point; returns how many events Windows accepted.
    public static uint ClickAt(int x, int y)
    {
        int left = GetSystemMetrics(76), top = GetSystemMetrics(77);
        int width = GetSystemMetrics(78), height = GetSystemMetrics(79);
        int ax = (int)(((long)(x - left) * 65535) / (width - 1));
        int ay = (int)(((long)(y - top) * 65535) / (height - 1));
        uint flags = ABSOLUTE | VIRTUALDESK;
        INPUT[] inputs = new INPUT[3];
        inputs[0] = Mouse(ax, ay, flags | MOVE);
        inputs[1] = Mouse(ax, ay, flags | LEFTDOWN);
        inputs[2] = Mouse(ax, ay, flags | LEFTUP);
        return SendInput(3, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    static INPUT Mouse(int x, int y, uint flags)
    {
        INPUT input = new INPUT();
        input.type = INPUT_MOUSE;
        input.mi.dx = x;
        input.mi.dy = y;
        input.mi.dwFlags = flags;
        return input;
    }
}
'@

$form = New-Object Windows.Forms.Form
$form.Text = 'wn01 click test'
$form.StartPosition = 'CenterScreen'
$form.Size = New-Object Drawing.Size 400, 300
$form.TopMost = $true
$script:clicked = $false
$script:sent = 0
$form.Add_MouseClick({ $script:clicked = $true; $form.Close() })

$send = New-Object Windows.Forms.Timer
$send.Interval = 500
$send.Add_Tick({
    $send.Stop()
    $form.Activate()
    $centre = $form.PointToScreen((New-Object Drawing.Point ($form.ClientSize.Width / 2), ($form.ClientSize.Height / 2)))
    $script:sent = [Wn01Input]::ClickAt($centre.X, $centre.Y)
})
$giveUp = New-Object Windows.Forms.Timer
$giveUp.Interval = 10000
$giveUp.Add_Tick({ $form.Close() })
$form.Add_Shown({ $send.Start(); $giveUp.Start() })
[Windows.Forms.Application]::Run($form)

if ($script:sent -ne 3) {
    Write-Output "SendInput accepted $($script:sent) of 3 events"
    exit 2
}
if (-not $script:clicked) {
    Write-Output 'the window did not receive the click'
    exit 1
}
Write-Output 'the window received the injected click'
exit 0
