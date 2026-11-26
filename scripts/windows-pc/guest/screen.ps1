<#
.SYNOPSIS
Captures tester's desktop as a PNG, printed as base64 on stdout.

.DESCRIPTION
Run through the runner by `pc screen`, which decodes the output into a file
on the dev box. It captures every screen at its real resolution (the process
declares itself DPI-aware first, or a scaled display would be cut off).
Exits 0 with the image, 3 when the desktop cannot be captured: the lock
screen or a UAC prompt is in front.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

Add-Type -AssemblyName System.Drawing, System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class Wn13Dpi
{
    [DllImport("user32.dll")]
    static extern bool SetProcessDpiAwarenessContext(IntPtr context);

    // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
    public static bool Declare() { return SetProcessDpiAwarenessContext(new IntPtr(-4)); }
}
'@

if (-not [Wn13Dpi]::Declare()) { throw 'could not make the capture DPI-aware' }
$area = [Windows.Forms.SystemInformation]::VirtualScreen
$bitmap = New-Object Drawing.Bitmap $area.Width, $area.Height
$graphics = [Drawing.Graphics]::FromImage($bitmap)
$stream = New-Object IO.MemoryStream
try {
    try {
        $graphics.CopyFromScreen($area.Left, $area.Top, 0, 0, $bitmap.Size)
    } catch {
        [Console]::Error.WriteLine("the desktop cannot be captured (the lock screen or a UAC prompt is in front): $_")
        exit 3
    }
    $bitmap.Save($stream, [Drawing.Imaging.ImageFormat]::Png)
    [Console]::Out.Write([Convert]::ToBase64String($stream.ToArray()))
} finally {
    $stream.Dispose()
    $graphics.Dispose()
    $bitmap.Dispose()
}
