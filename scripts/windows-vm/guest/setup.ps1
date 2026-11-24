<#
.SYNOPSIS
First-logon setup of the Windows test VM, run once from the setup stick.

.DESCRIPTION
Runs elevated as tester, from autounattend.xml's FirstLogonCommands, with the
stick's root one folder up. It leaves the VM reachable by `vm build` over SSH:
the proxy configured, OpenSSH installed from the stick (key logon only),
automatic logon kept on, nothing that locks the screen or sleeps, and the two
runner tasks registered and started. The build does the rest over SSH
(provision.ps1).
#>
param(
    [Parameter(Mandatory)] [string] $Password
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Stick = Split-Path $PSScriptRoot
$Root = 'C:\wn01'
$Proxy = 'http://10.0.2.100:3128'
$User = 'tester'

function Set-Reg([string] $Path, [string] $Name, $Value, [string] $Type = 'DWord') {
    # New-Item -Force on an existing key would try to recreate it.
    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType $Type -Force | Out-Null
}

function Install-GuestFiles {
    New-Item -ItemType Directory -Path $Root, 'C:\work' -Force | Out-Null
    & icacls.exe $Root /inheritance:r /grant:r 'Administrators:(OI)(CI)F' 'SYSTEM:(OI)(CI)F' "${User}:(OI)(CI)F" | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls $Root failed" }
    Copy-Item -Path (Join-Path $PSScriptRoot '*') -Destination $Root -Recurse -Force
}

function Set-ProxyEverywhere {
    # WinHTTP: Windows Update, activation, services.
    & netsh.exe winhttp set proxy proxy-server='10.0.2.100:3128' bypass-list='<local>' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'netsh winhttp set proxy failed' }
    # WinINet: tester's programs, the Store.
    $inet = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings'
    Set-Reg $inet 'ProxyEnable' 1
    Set-Reg $inet 'ProxyServer' '10.0.2.100:3128' 'String'
    Set-Reg $inet 'ProxyOverride' '<local>' 'String'
    # curl, rustup and cargo.
    foreach ($name in 'HTTP_PROXY', 'HTTPS_PROXY') {
        [Environment]::SetEnvironmentVariable($name, $Proxy, 'Machine')
    }
}

function Install-OpenSsh {
    $msi = Join-Path $Stick 'openssh.msi'
    $signature = Get-AuthenticodeSignature -FilePath $msi
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
        throw "openssh.msi is not signed by Microsoft: $($signature.Status)"
    }
    $p = Start-Process msiexec.exe -ArgumentList '/i', "`"$msi`"", '/qn', '/norestart', 'ADDLOCAL=Server' -Wait -PassThru
    if ($p.ExitCode -notin 0, 3010) { throw "OpenSSH install failed: $($p.ExitCode)" }
    $ssh = 'C:\ProgramData\ssh'
    New-Item -ItemType Directory -Path $ssh -Force | Out-Null
    Copy-Item (Join-Path $PSScriptRoot 'sshd_config') (Join-Path $ssh 'sshd_config') -Force
    $keys = Join-Path $ssh 'administrators_authorized_keys'
    Copy-Item (Join-Path $Stick 'authorized_keys') $keys -Force
    # sshd ignores this file unless only SYSTEM and Administrators can write it.
    & icacls.exe $keys /inheritance:r /grant:r 'Administrators:F' 'SYSTEM:F' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "icacls $keys failed" }
    Set-Reg 'HKLM:\SOFTWARE\OpenSSH' 'DefaultShell' 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' 'String'
    Get-NetFirewallRule -Name 'wn01-sshd' -ErrorAction SilentlyContinue | Remove-NetFirewallRule
    New-NetFirewallRule -Name 'wn01-sshd' -DisplayName 'OpenSSH (wn01)' -Direction Inbound -Protocol TCP -LocalPort 22 -Action Allow -Profile Any | Out-Null
    Set-Service sshd -StartupType Automatic
    Restart-Service sshd
}

function Disable-Interruptions {
    # Lets SSH sessions run the scripts in C:\wn01. Set-ExecutionPolicy would
    # throw here, since this process runs with -ExecutionPolicy Bypass.
    Set-Reg 'HKLM:\SOFTWARE\Microsoft\PowerShell\1\ShellIds\Microsoft.PowerShell' 'ExecutionPolicy' 'RemoteSigned' 'String'
    foreach ($setting in 'monitor-timeout-ac', 'standby-timeout-ac', 'hibernate-timeout-ac', 'disk-timeout-ac') {
        & powercfg.exe /change $setting 0
    }
    & powercfg.exe /hibernate off
    Set-Reg 'HKCU:\Control Panel\Desktop' 'ScreenSaveActive' '0' 'String'
    Set-Reg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\Personalization' 'NoLockScreen' 1
    Set-Reg 'HKLM:\SOFTWARE\Policies\Microsoft\Power\PowerSettings\0e796bdb-100d-47d6-a2d5-f7d2daa51f51' 'ACSettingIndex' 0
    # No automatic updates or their restarts: the build runs Windows Update itself.
    Set-Reg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' 'NoAutoUpdate' 1
    Set-Reg 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsUpdate\AU' 'NoAutoRebootWithLoggedOnUsers' 1
    # Builds are slow under real-time scanning.
    Add-MpPreference -ExclusionPath 'C:\work', "C:\Users\$User\.cargo", "C:\Users\$User\.rustup"
}

function Enable-PermanentAutoLogon {
    # Setup's AutoLogon counts down; these keep tester logging in at every boot.
    $winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
    Set-Reg $winlogon 'AutoAdminLogon' '1' 'String'
    Set-Reg $winlogon 'DefaultUserName' $User 'String'
    Set-Reg $winlogon 'DefaultDomainName' $env:COMPUTERNAME 'String'
    Set-Reg $winlogon 'DefaultPassword' $Password 'String'
    Remove-ItemProperty -Path $winlogon -Name 'AutoLogonCount' -ErrorAction SilentlyContinue
}

function Register-Runner([string] $Queue, [string] $RunLevel) {
    $name = "wn01-runner-$Queue"
    # No window at all: Windows Terminal ignores -WindowStyle Hidden, and an
    # elevated runner's window in the foreground would make Windows drop every
    # injected input (UIPI).
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Root\runner.ps1 -Queue $Queue"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
    $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel $RunLevel
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $name -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Start-ScheduledTask -TaskName $name
}

Start-Transcript -Path 'C:\wn01-setup.log' -Force | Out-Null
try {
    Install-GuestFiles
    Set-ProxyEverywhere
    Disable-Interruptions
    Enable-PermanentAutoLogon
    Register-Runner 'normal' 'Limited'
    Register-Runner 'high' 'Highest'
    # Last, so SSH answering means everything above is in place.
    Install-OpenSsh
    Write-Output 'setup finished'
} catch {
    Write-Output "setup failed: $_"
    Write-Output $_.ScriptStackTrace
    throw
} finally {
    Stop-Transcript | Out-Null
}
