<#
.SYNOPSIS
Sets the test PC up for `pc`: the desktop runner, automatic logon to tester,
the toolchain, and no sleep on AC power.

.DESCRIPTION
Run elevated over SSH by `pc setup`, from a staging folder that holds this
script, Autologon.cs and the runner's files; tester's password arrives as the
first line on stdin, never on a command line. Each item is checked first and
changed only when it is not already right. One line per item ("ok" or "set"),
then "setup: nothing changed" or "setup: changed ...". Windows settings it
changes are recorded the first time in C:\wn01\restore.json, and
uninstall.ps1 puts them back.
#>
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = 'C:\wn01'
$User = 'tester'
$Admins = 'S-1-5-32-544'
$System = 'S-1-5-18'
# The runner's files from scripts/windows-vm/guest, and the PC's own jobs.
$RunnerFiles = 'runner.ps1', 'JobRunner.cs', 'run-job.ps1', 'click-test.ps1', 'screen.ps1', 'focus.ps1'
$Winlogon = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion\Winlogon'
# Each install's own bound; the dev box bounds the whole setup too.
$InstallSeconds = 2400
$script:Changed = New-Object Collections.Generic.List[string]
# The files a running runner has loaded: a change to them needs a restart.
$RunnerCode = 'runner.ps1', 'JobRunner.cs'

# Reports go to the host, not the pipeline, so they never become part of a
# function's result.
function Report-Ok([string] $What) { Write-Host "ok   $What" }

function Report-Set([string] $What) {
    $script:Changed.Add($What)
    Write-Host "set  $What"
}

function New-Rule([string] $Sid, [string] $Rights, [string] $Inherit = 'ContainerInherit, ObjectInherit', [string] $Propagate = 'None') {
    New-Object Security.AccessControl.FileSystemAccessRule (
        (New-Object Security.Principal.SecurityIdentifier $Sid),
        [Security.AccessControl.FileSystemRights] $Rights,
        [Security.AccessControl.InheritanceFlags] $Inherit,
        [Security.AccessControl.PropagationFlags] $Propagate, 'Allow')
}

function Get-RuleText($Acl) {
    $rules = $Acl.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]) | ForEach-Object {
        '{0}|{1}|{2}|{3}|{4}' -f $_.IdentityReference.Value, [int] $_.FileSystemRights, [int] $_.InheritanceFlags,
        [int] $_.PropagationFlags, $_.AccessControlType
    }
    return (@($rules) | Sort-Object) -join ';'
}

# Whether the security $Have of a folder or file is owned by the
# administrators or SYSTEM, and protects and lists its explicit rules as
# $Want does.
function Test-Security($Have, $Want) {
    $owner = $Have.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($owner -notin $Admins, $System) { return $false }
    if ($Have.AreAccessRulesProtected -ne $Want.AreAccessRulesProtected) { return $false }
    return (Get-RuleText $Have) -eq (Get-RuleText $Want)
}

function Assert-NoReparsePoint([string] $Path) {
    if ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$Path is a junction or link: remove it, then run the setup again"
    }
}

# Folder (or file) security with these explicit rules alone.
function New-Security([bool] $Protected, [object[]] $Rules, [bool] $File) {
    $security = if ($File) { New-Object Security.AccessControl.FileSecurity } else { New-Object Security.AccessControl.DirectorySecurity }
    $security.SetAccessRuleProtection($Protected, $false)
    foreach ($rule in $Rules) { $security.AddAccessRule($rule) }
    return $security
}

# Gives folder (or, with -File, file) $Path exactly these explicit rules,
# owned by the administrators; $Protected stops it inheriting from its
# parent. A missing folder is made. A junction or link is refused: another
# account may have made it, to point the runner elsewhere.
function Set-Security([string] $Path, [bool] $Protected, [object[]] $Rules = @(), [switch] $File) {
    $made = -not $File -and -not (Test-Path -LiteralPath $Path -PathType Container)
    if ($made) { New-Item -ItemType Directory -Path $Path | Out-Null }
    Assert-NoReparsePoint $Path
    $want = New-Security $Protected $Rules $File
    if (-not $made -and (Test-Security (Get-Acl -LiteralPath $Path) $want)) {
        Report-Ok "security of $Path"
        return
    }
    $want.SetOwner((New-Object Security.Principal.SecurityIdentifier $Admins))
    Set-Acl -LiteralPath $Path -AclObject $want
    Report-Set "security of $Path"
}

# The runner's folders, each owned by the administrators, whoever made it.
# Only administrators and SYSTEM write the runner's files and hand in jobs,
# so no other account can change the runner or give it a job, and a job in
# the normal queue cannot hand one to the elevated runner. The unelevated
# runner writes its own queue's ready, log, work and outbox, and claims a job
# by moving it out of the inbox, which it can read and delete from but not
# write to. A moved job keeps the inbox's rules, and the runner removes it
# with Remove-Item -Force, which writes its attributes first.
function Install-Folders([string] $TesterSid) {
    $full = 'FullControl'
    Set-Security $Root $true @((New-Rule $Admins $full), (New-Rule $System $full), (New-Rule $TesterSid 'ReadAndExecute'))
    foreach ($folder in 'queue', 'queue\high', 'queue\high\inbox', 'queue\high\work', 'queue\high\outbox') {
        Set-Security "$Root\$folder" $false
    }
    Set-Security "$Root\queue\normal" $false @(New-Rule $TesterSid 'Modify')
    Set-Security "$Root\queue\normal\inbox" $true @(
        (New-Rule $Admins $full), (New-Rule $System $full),
        (New-Rule $TesterSid 'ReadAndExecute' 'None'),
        (New-Rule $TesterSid 'ReadAndExecute, Delete, WriteAttributes' 'ObjectInherit' 'InheritOnly'))
    foreach ($folder in 'queue\normal\work', 'queue\normal\outbox') {
        Set-Security "$Root\$folder" $false
    }
}

# Copies each runner file whose content differs, stamped with the time it
# was installed (a copy keeps its source's), and gives every one the
# folder's rules alone.
function Install-RunnerFiles {
    foreach ($name in $RunnerFiles) {
        $source = Join-Path $PSScriptRoot $name
        $target = Join-Path $Root $name
        if ((Test-Path -LiteralPath $target) -and
            (Get-FileHash -LiteralPath $target).Hash -eq (Get-FileHash -LiteralPath $source).Hash) {
            Report-Ok "file $target"
        } else {
            Copy-Item -LiteralPath $source -Destination $target -Force
            (Get-Item -LiteralPath $target).LastWriteTime = Get-Date
            Report-Set "file $target"
        }
        Set-Security $target $false -File
    }
}

# Writes $Text to $Path whole or not at all, on disk before it returns.
function Write-DurableFile([string] $Path, [string] $Text) {
    $tmp = "$Path.tmp"
    $bytes = (New-Object Text.UTF8Encoding $false).GetBytes($Text)
    $stream = New-Object IO.FileStream $tmp, 'Create', 'Write', 'None', 4096, 'WriteThrough'
    try {
        $stream.Write($bytes, 0, $bytes.Length)
        $stream.Flush($true)
    } finally {
        $stream.Dispose()
    }
    if (Test-Path -LiteralPath $Path) { [IO.File]::Replace($tmp, $Path, $null) } else { [IO.File]::Move($tmp, $Path) }
}

# Records a setting's value from before the first setup, once, before the
# setting is changed.
function Save-Original([string] $Name, $Value) {
    $path = Join-Path $Root 'restore.json'
    $saved = @{}
    if (Test-Path -LiteralPath $path) {
        (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $saved[$_.Name] = $_.Value }
    }
    if ($saved.ContainsKey($Name)) { return }
    $saved[$Name] = $Value
    Write-DurableFile $path ($saved | ConvertTo-Json -Compress)
}

function Stop-Runner([string] $Queue) {
    Stop-ScheduledTask -TaskName "wn01-runner-$Queue" -ErrorAction SilentlyContinue
    # Ending the task can leave its PowerShell running.
    Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
        Where-Object { $_.CommandLine -like "*$Root\runner.ps1 -Queue $Queue*" } |
        ForEach-Object { Invoke-CimMethod -InputObject $_ -MethodName Terminate | Out-Null }
}

function Test-TesterLoggedOn {
    return (Get-CimInstance Win32_ComputerSystem).UserName -eq "$env:COMPUTERNAME\$User"
}

# Whether queue $Queue's runner runs the installed code: the process its
# `ready` names is a PowerShell that started after the runner's files were
# installed. False when it does not run.
function Test-RunnerCurrent([string] $Queue) {
    $ready = Get-Item -LiteralPath (Join-Path $Root "queue\$Queue\ready") -ErrorAction SilentlyContinue
    if ($null -eq $ready) { return $false }
    $process = Get-Process -Id ([int] (Get-Content -LiteralPath $ready.FullName)) -ErrorAction SilentlyContinue
    if ($null -eq $process -or $process.ProcessName -ne 'powershell') { return $false }
    $installed = ($RunnerCode | ForEach-Object { (Get-Item -LiteralPath (Join-Path $Root $_)).LastWriteTime } |
            Measure-Object -Maximum).Maximum
    return $process.StartTime -gt $installed -and $process.StartTime -le $ready.LastWriteTime
}

# What matters of a runner task, as text, for a registered task and for one
# only built, so the two compare. Names lose their computer's prefix.
function Get-TaskText($Actions, $Triggers, $Principal, $Settings) {
    $action = @($Actions | ForEach-Object { "$($_.Execute) $($_.Arguments)" }) -join ';'
    $trigger = @($Triggers | ForEach-Object { "$($_.CimClass.CimClassName):$(($_.UserId -split '\\')[-1]):$($_.Enabled)" }) -join ';'
    $principal = '{0}:{1}:{2}' -f ($Principal.UserId -split '\\')[-1], $Principal.LogonType, $Principal.RunLevel
    $limits = '{0}|{1}|{2}|{3}|{4}|{5}|{6}' -f $Settings.Enabled, $Settings.ExecutionTimeLimit, $Settings.MultipleInstances,
        $Settings.RestartCount, $Settings.RestartInterval, $Settings.DisallowStartIfOnBatteries, $Settings.StopIfGoingOnBatteries
    return "$action|$trigger|$principal|$limits"
}

# The VM's runner task (scripts/windows-vm/guest/setup.ps1), registered
# again when what is registered differs; returns whether it was.
function Install-RunnerTask([string] $Queue, [string] $RunLevel) {
    $name = "wn01-runner-$Queue"
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Root\runner.ps1 -Queue $Queue"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
    $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel $RunLevel
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    if ($null -ne $task -and (Get-TaskText $task.Actions $task.Triggers $task.Principal $task.Settings) -eq
        (Get-TaskText $action $trigger $principal $settings)) {
        Report-Ok "task $name"
        return $false
    }
    Stop-Runner $Queue
    Register-ScheduledTask -TaskName $name -Description "Desktop runner for pc run ($Queue queue)" -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Report-Set "task $name"
    return $true
}

# The runner task, and its runner started again unless it already runs the
# installed files under the current task; a setup stopped half-way leaves a
# runner older than its files, which the next run restarts.
function Install-Runner([string] $Queue, [string] $RunLevel) {
    $changed = Install-RunnerTask $Queue $RunLevel
    if (-not (Test-TesterLoggedOn)) {
        Write-Host "     wn01-runner-$Queue starts at tester's next logon (tester is not logged on)"
        return
    }
    if (-not $changed -and (Test-RunnerCurrent $Queue)) {
        Report-Ok "runner $Queue runs the installed files"
        return
    }
    Stop-Runner $Queue
    Start-ScheduledTask -TaskName "wn01-runner-$Queue"
    Report-Set "runner $Queue started"
}

function Set-RegistryString([string] $Name, [string] $Value) {
    $have = (Get-ItemProperty -LiteralPath $Winlogon -Name $Name -ErrorAction SilentlyContinue)
    if ($null -ne $have -and $have.$Name -eq $Value) {
        Report-Ok "Winlogon $Name"
        return
    }
    New-ItemProperty -LiteralPath $Winlogon -Name $Name -Value $Value -PropertyType String -Force | Out-Null
    Report-Set "Winlogon $Name"
}

function Remove-RegistryValue([string] $Name) {
    if ($null -eq (Get-ItemProperty -LiteralPath $Winlogon -Name $Name -ErrorAction SilentlyContinue)) {
        Report-Ok "Winlogon has no $Name"
        return
    }
    Remove-ItemProperty -LiteralPath $Winlogon -Name $Name
    Report-Set "Winlogon $Name removed"
}

# tester logs in at every boot, with the password from the dev box. The
# account's password is set first: should storing it for Winlogon fail, the
# old one still logs in until a second run.
function Enable-AutoLogon([string] $Password) {
    if ([Wn13.Autologon]::IsPassword($User, $Password)) {
        Report-Ok "tester's password"
    } else {
        Set-LocalUser -Name $User -Password (ConvertTo-SecureString $Password -AsPlainText -Force)
        Report-Set "tester's password"
    }
    $account = Get-LocalUser -Name $User
    if ($null -eq $account.PasswordExpires) {
        Report-Ok "tester's password never expires"
    } else {
        Save-Original 'passwordNeverExpires' $false
        Set-LocalUser -Name $User -PasswordNeverExpires $true
        Report-Set "tester's password never expires"
    }
    if ([Wn13.Autologon]::ReadPassword() -ceq $Password) {
        Report-Ok 'the password Winlogon logs on with'
    } else {
        [Wn13.Autologon]::WritePassword($Password)
        Report-Set 'the password Winlogon logs on with'
    }
    Set-RegistryString 'AutoAdminLogon' '1'
    Set-RegistryString 'DefaultUserName' $User
    $domain = (Get-ItemProperty -LiteralPath $Winlogon -Name 'DefaultDomainName' -ErrorAction SilentlyContinue)
    if ($null -eq $domain -or $domain.DefaultDomainName -notin '', $env:COMPUTERNAME) {
        Set-RegistryString 'DefaultDomainName' $env:COMPUTERNAME
    } else {
        Report-Ok 'Winlogon DefaultDomainName'
    }
    # A count turns automatic logon off once it runs out; the registry's
    # plain-text copy of a password is not needed beside the LSA secret.
    Remove-RegistryValue 'AutoLogonCount'
    Remove-RegistryValue 'DefaultPassword'
}

# The active power plan's GUID.
function Get-ActivePlan {
    $plan = Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerPlan -Filter 'IsActive = True'
    if (@($plan).Count -ne 1) { throw 'could not read the active power plan' }
    return $plan.InstanceID -replace '^.*\\\{(.*)\}$', '$1'
}

# The AC value in seconds of SUB_SLEEP setting $Guid in power plan $Plan,
# read through CIM, whose names do not depend on the language.
function Get-SleepSetting([string] $Plan, [string] $Guid) {
    $index = Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerSettingDataIndex |
        Where-Object { $_.InstanceID -like "*{$Plan}\AC\{$Guid}" }
    # One instance, or the read failed: never taken as "never sleeps".
    if (@($index).Count -ne 1) { throw "could not read power setting $Guid" }
    return [int] $index.SettingIndexValue
}

# Nobody wakes the PC: it never sleeps or hibernates on AC while set up.
# The value found is recorded under its plan, which uninstall puts back.
function Disable-Sleep {
    $plan = Get-ActivePlan
    foreach ($setting in @{ name = 'STANDBYIDLE'; guid = '29f6c1db-86da-48c5-9fdb-f2b67b1f44da' },
        @{ name = 'HIBERNATEIDLE'; guid = '9d7815a6-7ee4-497e-8888-515a05f02364' }) {
        $value = Get-SleepSetting $plan $setting.guid
        if ($value -eq 0) {
            Report-Ok "power $($setting.name) on AC: never"
            continue
        }
        Save-Original "power:${plan}:$($setting.name)" $value
        & powercfg.exe /setacvalueindex $plan SUB_SLEEP $setting.name 0
        if ($LASTEXITCODE -ne 0) { throw "powercfg /setacvalueindex $($setting.name) failed" }
        & powercfg.exe /setactive $plan
        if ($LASTEXITCODE -ne 0) { throw 'powercfg /setactive failed' }
        Report-Set "power $($setting.name) on AC: never (was $value s)"
    }
}

# Runs an installer, ended after $InstallSeconds, and fails unless it exits
# with one of $Ok.
function Invoke-Installer([string] $Path, [string[]] $Arguments, [int[]] $Ok = @(0)) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -NoNewWindow
    if (-not $process.WaitForExit($InstallSeconds * 1000)) {
        # winget and Visual Studio's setup start installers of their own.
        & taskkill.exe /T /F /PID $process.Id | Out-Null
        throw "$Path did not finish within $InstallSeconds s"
    }
    if ($process.ExitCode -notin $Ok) { throw "$Path exited with $($process.ExitCode)" }
}

# winget has no alias in an SSH session; it is called by its full path.
function Get-Winget {
    $found = Get-ChildItem -Path 'C:\Program Files\WindowsApps\Microsoft.DesktopAppInstaller_*_x64__8wekyb3d8bbwe\winget.exe' -ErrorAction SilentlyContinue |
        Sort-Object { [version] ($_.Directory.Name -split '_')[1] } | Select-Object -Last 1
    if ($null -eq $found) { throw 'winget is not installed' }
    return $found.FullName
}

function Install-WingetPackage([string] $Id, [string[]] $Extra = @()) {
    Invoke-Installer (Get-Winget) (@('install', '--id', $Id, '--exact', '--source', 'winget', '--silent',
            '--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity') + $Extra)
}

# Visual Studio Build Tools with the C++ tools (the MSVC linker) and a
# Windows SDK.
function Install-BuildTools {
    $installer = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer"
    $sdk = Test-Path 'C:\Program Files (x86)\Windows Kits\10\Lib\*\um\x64\kernel32.lib'
    $vc = $null
    if (Test-Path "$installer\vswhere.exe") {
        $vc = & "$installer\vswhere.exe" -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    }
    if ($vc -and $sdk) {
        Report-Ok 'Build Tools with the C++ tools and a Windows SDK'
        return
    }
    $tools = $null
    if (Test-Path "$installer\vswhere.exe") {
        $tools = & "$installer\vswhere.exe" -products Microsoft.VisualStudio.Product.BuildTools -property installationPath
    }
    $add = '--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    if ($tools) {
        Invoke-Installer "$installer\setup.exe" @('modify', '--installPath', "`"$tools`"", $add, '--quiet', '--norestart') @(0, 3010)
    } else {
        Install-WingetPackage 'Microsoft.VisualStudio.2022.BuildTools' @('--override', "`"--wait --quiet --norestart $add`"")
    }
    Report-Set 'Build Tools with the C++ tools and a Windows SDK'
}

# Runs rustup with these arguments and fails when it does.
function Invoke-Rustup([string[]] $Arguments) {
    & (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe') @Arguments
    if ($LASTEXITCODE -ne 0) { throw "rustup $($Arguments -join ' ') failed" }
}

# What Rust lacks of stable as tester's default with clippy and rustfmt:
# 'rustup', 'default' or 'components', in the order they are fixed.
function Get-MissingRust {
    if (-not (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'))) {
        return @('rustup', 'default', 'components')
    }
    # It fails when no default is set: that is a missing default too. Its
    # stderr goes nowhere through cmd.exe, since PowerShell 5.1 would turn
    # it into a terminating error here.
    $default = & cmd.exe /d /c "$env:USERPROFILE\.cargo\bin\rustup.exe default 2>nul"
    if ($LASTEXITCODE -ne 0 -or "$default" -notmatch '^stable-') {
        # Stable may not be installed either; adding it brings its components.
        return @('default', 'components')
    }
    $installed = Invoke-Rustup @('component', 'list', '--installed', '--toolchain', 'stable')
    $components = @(@($installed) -match '^(clippy|rustfmt)-')
    if ($components.Count -ne 2) { return @('components') }
    return @()
}

function Install-Rust {
    $missing = @(Get-MissingRust)
    if ($missing.Count -eq 0) {
        Report-Ok 'Rust stable with clippy and rustfmt'
        return
    }
    if ('rustup' -in $missing) { Install-WingetPackage 'Rustlang.Rustup' }
    if ('default' -in $missing) { Invoke-Rustup @('default', 'stable') | Out-Host }
    if ('components' -in $missing) { Invoke-Rustup @('component', 'add', 'clippy', 'rustfmt', '--toolchain', 'stable') | Out-Host }
    Report-Set 'Rust stable with clippy and rustfmt'
}

$Password = [Console]::In.ReadLine()
if ([string]::IsNullOrEmpty($Password)) { throw 'no password on stdin' }
try {
    Add-Type -Path (Join-Path $PSScriptRoot 'Autologon.cs')
    $testerSid = (Get-LocalUser -Name $User).SID.Value
    Install-Folders $testerSid
    Install-RunnerFiles
    Install-Runner 'normal' 'Limited'
    Install-Runner 'high' 'Highest'
    Enable-AutoLogon $Password
    Disable-Sleep
    Install-BuildTools
    Install-Rust
    if ($script:Changed.Count -eq 0) {
        Write-Output 'setup: nothing changed'
    } else {
        Write-Output "setup: changed $($script:Changed.Count): $($script:Changed -join ', ')"
    }
} finally {
    Set-Location C:\
    Remove-Item -LiteralPath $PSScriptRoot -Recurse -Force -ErrorAction SilentlyContinue
}
