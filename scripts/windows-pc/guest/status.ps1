<#
.SYNOPSIS
What `pc status` reads over SSH: Windows, automatic logon, who is logged on,
the lock screen, a pending restart, sleep, and the runners' processes.

.DESCRIPTION
Sent inline by `pc status` (it works before the setup has run) and printed
as "name: value" lines. It changes nothing.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'

function Get-Value([string] $Path, [string] $Name) {
    $item = Get-ItemProperty -LiteralPath $Path -Name $Name -ErrorAction SilentlyContinue
    if ($null -eq $item) { return $null }
    return $item.$Name
}

# Seconds until the PC sleeps on AC power; 0 means never, $null unknown.
function Get-SleepMinutes {
    $scheme = (Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerPlan -Filter 'IsActive = True').InstanceID -replace '^.*\\', ''
    $index = Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerSettingDataIndex |
        Where-Object { $_.InstanceID -like "*$scheme\AC\{29f6c1db-86da-48c5-9fdb-f2b67b1f44da}" }
    if (@($index).Count -ne 1) { return $null }
    return [int] $index.SettingIndexValue
}

# Whether $Process is the runner that wrote $Ready: a PowerShell that
# started before it, not another process that has the pid now.
function Test-RunnerProcess($Process, $Ready) {
    if ($null -eq $Process -or $Process.ProcessName -ne 'powershell') { return $false }
    return $Process.StartTime -le $Ready.LastWriteTime
}

# The runner of queue $Queue as runners_ready checks it, in words.
function Get-RunnerState([string] $Queue, [datetime] $Boot) {
    $ready = Get-Item -LiteralPath "C:\wn01\queue\$Queue\ready" -ErrorAction SilentlyContinue
    if ($null -eq $ready) { return 'not set up' }
    if ($ready.LastWriteTime -lt $Boot) { return 'not started since the boot' }
    $process = Get-Process -Id ([int] (Get-Content -LiteralPath $ready.FullName)) -ErrorAction SilentlyContinue
    if (-not (Test-RunnerProcess $process $ready)) { return 'not running' }
    return "running since $($process.StartTime.ToString('yyyy-MM-dd HH:mm'))"
}

$os = Get-CimInstance Win32_OperatingSystem
$ubr = Get-Value 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion' 'UBR'
Write-Output "windows:    $($os.Caption), build $($os.Version).$ubr"
Write-Output "booted:     $($os.LastBootUpTime.ToString('yyyy-MM-dd HH:mm'))"
$autologon = if ((Get-Value $Winlogon 'AutoAdminLogon') -eq '1') { "on, as $(Get-Value $Winlogon 'DefaultUserName')" } else { 'off' }
if ($null -ne (Get-Value $Winlogon 'AutoLogonCount')) { $autologon += " (ends after $(Get-Value $Winlogon 'AutoLogonCount') more)" }
Write-Output "autologon:  $autologon"
$user = (Get-CimInstance Win32_ComputerSystem).UserName
Write-Output "logged on:  $(if ($user) { $user } else { 'nobody' })"
$locked = @(Get-Process -Name LogonUI -ErrorAction SilentlyContinue).Count -gt 0
Write-Output "screen:     $(if ($locked) { 'locked, or at the sign-in screen: injected input goes nowhere' } else { 'unlocked' })"
$restart = (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\WindowsUpdate\Auto Update\RebootRequired') -or
    (Test-Path 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Component Based Servicing\RebootPending')
Write-Output "restart:    $(if ($restart) { 'pending (Windows Update may restart the PC)' } else { 'none pending' })"
$sleep = Get-SleepMinutes
Write-Output "sleep (AC): $(if ($null -eq $sleep) { 'unknown' } elseif ($sleep -eq 0) { 'never' } else { "after $sleep s idle" })"
foreach ($queue in 'normal', 'high') {
    Write-Output ("{0,-12}{1}" -f "${queue}:", (Get-RunnerState $queue $os.LastBootUpTime))
}
