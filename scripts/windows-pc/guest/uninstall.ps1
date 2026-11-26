<#
.SYNOPSIS
Removes what setup.ps1 added to the test PC.

.DESCRIPTION
Run elevated over SSH by `pc uninstall`, from a staging folder that holds
this script and Autologon.cs. It stops and removes both runner tasks and
their processes, puts back the Windows settings C:\wn01\restore.json
recorded, removes C:\wn01 and turns automatic logon off. Then it checks that
none of them is left, exits 1 if one is, and names what it left alone.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = 'C:\wn01'
$User = 'tester'
$Winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
$Tasks = 'wn01-runner-normal', 'wn01-runner-high'
# Administrators and SYSTEM.
$Trusted = 'S-1-5-32-544', 'S-1-5-18'

function Get-RunnerProcesses {
    Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
        Where-Object { $_.CommandLine -like "*-File $Root\runner.ps1 -Queue *" }
}

# Whether process $Id still runs. Windows keeps an exited process findable
# while another program holds a handle to it, so being found is not enough.
function Test-ProcessAlive([int] $Id) {
    $process = Get-Process -Id $Id -ErrorAction SilentlyContinue
    return $null -ne $process -and -not $process.HasExited
}

# Task $Name, or $null when Windows says there is none; any other failure
# to look it up stops the uninstall before C:\wn01 goes, since a task left
# registered would run whatever a new C:\wn01 held.
function Get-RunnerTask([string] $Name) {
    try {
        return Get-ScheduledTask -TaskName $Name -ErrorAction Stop
    } catch {
        if ($_.CategoryInfo.Category -eq 'ObjectNotFound') { return $null }
        throw
    }
}

function Remove-RunnerTask([string] $Name) {
    if ($null -eq (Get-RunnerTask $Name)) { return }
    Stop-ScheduledTask -TaskName $Name -ErrorAction SilentlyContinue
    Unregister-ScheduledTask -TaskName $Name -Confirm:$false
    Write-Output "removed task $Name"
}

# Ends runner process $Process, or stops the uninstall before C:\wn01 goes.
function Stop-RunnerProcess($Process) {
    Invoke-CimMethod -InputObject $Process -MethodName Terminate | Out-Null
    Wait-Process -Id $Process.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
    if (Test-ProcessAlive $Process.ProcessId) { throw "runner process $($Process.ProcessId) did not end" }
    Write-Output "ended runner process $($Process.ProcessId)"
}

# Ending a task can leave its PowerShell running; a job it ran ends with it,
# since the job object closes with the runner.
function Remove-Runners {
    foreach ($name in $Tasks) { Remove-RunnerTask $name }
    foreach ($process in @(Get-RunnerProcesses)) { Stop-RunnerProcess $process }
}

# Whether C:\wn01 is the setup's: a plain folder the administrators or
# SYSTEM own, inheriting nothing, that only they can write (tester may read).
# One another account made or can write is left alone, and nothing in it is
# read.
function Test-SetupRoot {
    $item = Get-Item -LiteralPath $Root -Force
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { return $false }
    $acl = Get-Acl -LiteralPath $Root
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $Trusted -or -not $acl.AreAccessRulesProtected) { return $false }
    # Synchronize comes with every rule Windows writes.
    $readOnly = [int] [Security.AccessControl.FileSystemRights]'ReadAndExecute, Synchronize'
    $writers = $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]) |
        Where-Object { $_.IdentityReference.Value -notin $Trusted -and ([int] $_.FileSystemRights -bor $readOnly) -ne $readOnly }
    return @($writers).Count -eq 0
}

function Restore-Settings {
    $path = Join-Path $Root 'restore.json'
    if (-not (Test-Path -LiteralPath $path)) { return }
    $saved = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    # Only for the account it was read from, by SID.
    $account = Get-LocalUser -Name $User -ErrorAction SilentlyContinue
    $expiry = if ($account) { $saved.PSObject.Properties["passwordNeverExpires:$($account.SID.Value)"] }
    if ($expiry) {
        Set-LocalUser -Name $User -PasswordNeverExpires ([bool] $expiry.Value)
        Write-Output "put back: tester's password never expires = $($expiry.Value)"
    }
    # Each power value under the plan it was read from: "power:<plan>:<name>".
    foreach ($property in @($saved.PSObject.Properties | Where-Object Name -like 'power:*')) {
        $plan, $name = $property.Name.Split(':')[1, 2]
        $exists = Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerPlan |
            Where-Object { $_.InstanceID -like "*{$plan}" }
        if ($null -eq $exists) {
            Write-Output "power plan $plan is gone: its $name has nothing to go back to"
            continue
        }
        & powercfg.exe /setacvalueindex $plan SUB_SLEEP $name ([int] $property.Value)
        if ($LASTEXITCODE -ne 0) { throw "powercfg /setacvalueindex $plan $name failed" }
        Write-Output "put back: power $name on AC in plan $plan = $($property.Value) s"
    }
    & powercfg.exe /setactive SCHEME_CURRENT
    if ($LASTEXITCODE -ne 0) { throw 'powercfg /setactive failed' }
}

function Disable-AutoLogon {
    New-ItemProperty -LiteralPath $Winlogon -Name 'AutoAdminLogon' -Value '0' -PropertyType String -Force | Out-Null
    foreach ($name in 'AutoLogonCount', 'DefaultPassword') {
        Remove-ItemProperty -LiteralPath $Winlogon -Name $name -ErrorAction SilentlyContinue
    }
    [Wn13.Autologon]::RemovePassword()
    Write-Output 'automatic logon is off'
}

function Test-WinlogonValue([string] $Name) {
    return $null -ne (Get-ItemProperty -LiteralPath $Winlogon -Name $Name -ErrorAction SilentlyContinue)
}

# Each check names what it finds left of the setup's.
$Leftovers = [ordered]@{
    'task wn01-runner-normal' = { $null -ne (Get-RunnerTask 'wn01-runner-normal') }
    'task wn01-runner-high' = { $null -ne (Get-RunnerTask 'wn01-runner-high') }
    'a runner process' = { @(Get-RunnerProcesses).Count -gt 0 }
    $Root = { Test-Path -LiteralPath $Root }
    'Winlogon AutoAdminLogon' = { (Get-ItemProperty -LiteralPath $Winlogon -Name 'AutoAdminLogon' -ErrorAction SilentlyContinue).AutoAdminLogon -ne '0' }
    'Winlogon AutoLogonCount' = { Test-WinlogonValue 'AutoLogonCount' }
    'Winlogon DefaultPassword' = { Test-WinlogonValue 'DefaultPassword' }
    'the LSA secret DefaultPassword' = { $null -ne [Wn13.Autologon]::ReadPassword() }
}

# What is still there of the setup's; empty when nothing.
function Get-Leftovers {
    return @($Leftovers.Keys | Where-Object { & $Leftovers[$_] })
}

try {
    Add-Type -Path (Join-Path $PSScriptRoot 'Autologon.cs')
    # Before anything else that can fail: a PC left half uninstalled must
    # not still log tester on at boot.
    Disable-AutoLogon
    Remove-Runners
    if ((Test-Path -LiteralPath $Root) -and -not (Test-SetupRoot)) {
        Write-Output "$Root was not made by the setup: left alone, and no setting put back from it"
    } elseif (Test-Path -LiteralPath $Root) {
        # restore.json goes with C:\wn01 only once every setting is back.
        Restore-Settings
        Remove-Item -LiteralPath $Root -Recurse -Force
        Write-Output "removed $Root"
    }
    $left = @(Get-Leftovers)
    if ($left.Count -gt 0) {
        Write-Output "uninstall: still there: $($left -join ', ')"
        exit 1
    }
    Write-Output 'uninstall: no task, runner file or automatic logon entry is left'
    Write-Output "left alone: SSH (sshd, its config and keys, the firewall rule devbox-ssh), Build Tools, Rust, C:\work, and tester's password, which stays the one in the dev box's password file"
} finally {
    Set-Location C:\
    Remove-Item -LiteralPath $PSScriptRoot -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
