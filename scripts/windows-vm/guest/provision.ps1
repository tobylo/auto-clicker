<#
.SYNOPSIS
The build's steps after first logon, each run by `vm build` over SSH with its
own deadline.

.PARAMETER Step
build-tools     Visual Studio Build Tools: the MSVC linker and Windows SDK.
rust            rustup and the stable MSVC toolchain, for tester.
update-start    Starts one Windows Update pass as SYSTEM (windows-update.ps1).
update-status   Prints that pass's status: running, done, reboot or failed.
activate        Activates the evaluation online.
info            Prints the Windows build and the evaluation's expiry, as JSON.
#>
param(
    [Parameter(Mandatory)]
    [ValidateSet('build-tools', 'rust', 'update-start', 'update-status', 'activate', 'info')]
    [string] $Step
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = 'C:\wn01'
$Proxy = 'http://10.0.2.100:3128'
$Downloads = 'C:\wn01\downloads'
$WindowsAppId = '55c92734-d682-4d71-983e-d6ec3f16059f'

function Get-File([string] $Url, [string] $Name) {
    New-Item -ItemType Directory -Path $Downloads -Force | Out-Null
    $path = Join-Path $Downloads $Name
    # Best effort: a revocation host the proxy does not list must not fail the
    # download; what is downloaded is checked by hash or signature anyway.
    & curl.exe --fail --location --silent --show-error --ssl-revoke-best-effort --proxy $Proxy --max-time 900 --output $path $Url
    if ($LASTEXITCODE -ne 0) { throw "download of $Url failed: curl exit $LASTEXITCODE" }
    return $path
}

function Assert-MicrosoftSigned([string] $Path) {
    $signature = Get-AuthenticodeSignature -FilePath $Path
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw "$Path is not signed by Microsoft: $($signature.Status)"
    }
}

function Install-BuildTools {
    $installer = Get-File 'https://aka.ms/vs/17/release/vs_BuildTools.exe' 'vs_BuildTools.exe'
    Assert-MicrosoftSigned $installer
    $arguments = '--quiet', '--wait', '--norestart', '--nocache',
        '--add', 'Microsoft.VisualStudio.Workload.VCTools', '--includeRecommended'
    $p = Start-Process -FilePath $installer -ArgumentList $arguments -Wait -PassThru
    # 3010: installed, a restart is due (the build restarts after Windows Update).
    if ($p.ExitCode -notin 0, 3010) { throw "Build Tools install failed: $($p.ExitCode)" }
}

function Install-Rust {
    $base = 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe'
    $init = Get-File $base 'rustup-init.exe'
    $expected = ((Get-Content -Raw (Get-File "$base.sha256" 'rustup-init.exe.sha256')) -split '\s+')[0]
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $init).Hash
    if ($actual -ne $expected) { throw "rustup-init.exe SHA-256 $actual does not match $expected" }
    $env:HTTPS_PROXY = $Proxy
    & $init -y --default-toolchain stable --profile minimal -c clippy -c rustfmt
    if ($LASTEXITCODE -ne 0) { throw "rustup-init failed: $LASTEXITCODE" }
    # rustup puts cargo on the user's PATH, which SSH sessions do not get.
    $bin = "$env:USERPROFILE\.cargo\bin"
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    if (($machine -split ';') -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', "$machine;$bin", 'Machine')
    }
    & "$env:USERPROFILE\.cargo\bin\rustc.exe" --version
}

function Start-WindowsUpdate {
    $name = 'wn01-windows-update'
    Remove-Item -LiteralPath "$Root\windows-update.json" -Force -ErrorAction SilentlyContinue
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Root\windows-update.ps1"
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Hours 3) -MultipleInstances IgnoreNew
    Register-ScheduledTask -TaskName $name -Action $action -Principal $principal -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $name
}

function Get-WindowsUpdateStatus {
    $path = "$Root\windows-update.json"
    if (Test-Path -LiteralPath $path) {
        $status = Get-Content -Raw -LiteralPath $path | ConvertFrom-Json
        if ($status.error) { [Console]::Error.WriteLine($status.error) }
        return $status.state
    }
    $task = Get-ScheduledTask -TaskName 'wn01-windows-update'
    if ($task.State -eq 'Running' -or $task.State -eq 'Queued') { return 'running' }
    # The task may have written its status and ended since the first look.
    if (Test-Path -LiteralPath $path) { return Get-WindowsUpdateStatus }
    return 'failed'
}

function Get-WindowsLicense {
    Get-CimInstance SoftwareLicensingProduct -Filter "ApplicationID='$WindowsAppId' AND PartialProductKey IS NOT NULL"
}

function Invoke-Activation {
    & cscript.exe //nologo "$env:windir\System32\slmgr.vbs" /ato
    $license = Get-WindowsLicense
    # LicenseStatus 1 is licensed.
    if ($license.LicenseStatus -ne 1) { throw "Windows is not activated: license status $($license.LicenseStatus)" }
}

function Get-Info {
    $license = Get-WindowsLicense
    $os = Get-CimInstance Win32_OperatingSystem
    $ubr = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion').UBR
    @{
        build = "$($os.Version).$ubr"
        caption = $os.Caption
        licenseStatus = $license.LicenseStatus
        expires = (Get-Date).ToUniversalTime().AddMinutes($license.GracePeriodRemaining).ToString('yyyy-MM-dd')
    } | ConvertTo-Json -Compress
}

switch ($Step) {
    'build-tools' { Install-BuildTools }
    'rust' { Install-Rust }
    'update-start' { Start-WindowsUpdate }
    'update-status' { Get-WindowsUpdateStatus }
    'activate' { Invoke-Activation }
    'info' { Get-Info }
}
