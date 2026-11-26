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
$RustupSeconds = 1200
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

# Folder security with these explicit rules alone.
function New-Security([bool] $Protected, [object[]] $Rules) {
    $security = New-Object Security.AccessControl.DirectorySecurity
    $security.SetAccessRuleProtection($Protected, $false)
    foreach ($rule in $Rules) { $security.AddAccessRule($rule) }
    return $security
}

# C:\wn01 is the setup's own: made by it, with its rules, in one step, or
# found as it leaves it, a plain folder the administrators or SYSTEM own with
# exactly the setup's rules. One that another account made or could write
# (or a junction in its place) is refused, since what that account put
# inside could not be trusted; the setup never adopts it by tightening it.
# Once the root is the setup's, no other account can add anything to it,
# so nothing inside needs checking for its origin.
function Install-Root($Security) {
    if (-not (Test-Path -LiteralPath $Root)) {
        [IO.Directory]::CreateDirectory($Root, $Security) | Out-Null
        # Checked again: a folder another account put there in between is
        # taken by CreateDirectory as it is.
        Assert-SetupRoot $Security
        Report-Set "made $Root"
        return
    }
    Assert-SetupRoot $Security
    Report-Ok "$Root is the setup's"
}

function Assert-SetupRoot($Security) {
    $acl = Get-Acl -LiteralPath $Root
    $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if (-not (Test-SetupFolder (Get-Item -LiteralPath $Root -Force) $owner) -or -not (Test-Security $acl $Security)) {
        throw "$Root was not made by the setup, or others can write to it (owner $owner): remove it from an elevated prompt (Remove-Item -Recurse -Force $Root), then run the setup again"
    }
}

# Whether $Item, owned by $Owner, is a plain folder the administrators or
# SYSTEM own.
function Test-SetupFolder($Item, [string] $Owner) {
    if (-not $Item.PSIsContainer) { return $false }
    if ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) { return $false }
    return $Owner -in $Admins, $System
}

# Refuses a junction or link at $Path or at any folder between it and
# C:\wn01, before anything there is made or changed: tester may write in
# queue\normal, and a link there would take the setup's changes to wherever
# it points.
function Assert-NoLink([string] $Path) {
    for ($at = $Path; $at.Length -gt $Root.Length; $at = Split-Path $at) {
        $item = Get-Item -LiteralPath $at -Force -ErrorAction SilentlyContinue
        if ($item -and ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "$at is a junction or link: remove it, then run the setup again"
        }
    }
}

# Gives folder $Path exactly these explicit rules, owned by the
# administrators; $Protected stops it inheriting from its parent. A missing
# folder is made.
function Set-Security([string] $Path, [bool] $Protected, [object[]] $Rules = @()) {
    Assert-NoLink $Path
    $made = -not (Test-Path -LiteralPath $Path -PathType Container)
    if ($made) { New-Item -ItemType Directory -Path $Path | Out-Null }
    $want = New-Security $Protected $Rules
    if (-not $made -and (Test-Security (Get-Acl -LiteralPath $Path) $want)) {
        Report-Ok "security of $Path"
        return
    }
    $want.SetOwner((New-Object Security.Principal.SecurityIdentifier $Admins))
    Set-Acl -LiteralPath $Path -AclObject $want
    Report-Set "security of $Path"
}

# The runner's folders. Only administrators and SYSTEM write the runner's
# files and hand in jobs, so no other account can change the runner or give
# it a job, and a job in the normal queue cannot hand one to the elevated
# runner. The unelevated runner writes its own queue's ready, log, work and
# outbox, and claims a job by moving it out of the inbox, which it can read
# and delete from but not write to. A moved job keeps the inbox's rules, and
# the runner removes it with Remove-Item -Force, which writes its attributes
# first.
function Install-Folders([string] $TesterSid) {
    $full = 'FullControl'
    $rootRules = @((New-Rule $Admins $full), (New-Rule $System $full), (New-Rule $TesterSid 'ReadAndExecute'))
    Install-Root (New-Security $true $rootRules)
    foreach ($folder in 'queue', 'queue\high', 'queue\high\inbox', 'queue\high\work', 'queue\high\outbox') {
        Set-Security "$Root\$folder" $false
    }
    # The inbox first, made with its parent while tester can write in
    # neither, so nothing of tester's is in it when it is protected.
    Set-Security "$Root\queue\normal\inbox" $true @(
        (New-Rule $Admins $full), (New-Rule $System $full),
        (New-Rule $TesterSid 'ReadAndExecute' 'None'),
        (New-Rule $TesterSid 'ReadAndExecute, Delete, WriteAttributes' 'ObjectInherit' 'InheritOnly'))
    Set-Security "$Root\queue\normal" $false @(New-Rule $TesterSid 'Modify')
    foreach ($folder in 'queue\normal\work', 'queue\normal\outbox') {
        Set-Security "$Root\$folder" $false
    }
}

# Copies each runner file whose content differs, stamped with the time it
# was installed (a copy keeps its source's).
function Install-RunnerFiles {
    foreach ($name in $RunnerFiles) {
        $source = Join-Path $PSScriptRoot $name
        $target = Join-Path $Root $name
        if ((Test-Path -LiteralPath $target) -and
            (Get-FileHash -LiteralPath $target).Hash -eq (Get-FileHash -LiteralPath $source).Hash) {
            Report-Ok "file $target"
            continue
        }
        # Code a runner has loaded: both stop first, so none can start
        # between the copy and its time stamp and pass for current; the
        # setup starts them again.
        # Their tasks are disabled first, so no logon starts a runner on half
        # the new files; that changes their export, so Install-RunnerTask
        # registers them again, enabled, once every file is in.
        if ($name -in $RunnerCode) { Suspend-Runners }
        # Whole or not at all: a half-copied run-job.ps1 would run as empty.
        Copy-Item -LiteralPath $source -Destination "$target.new" -Force
        (Get-Item -LiteralPath "$target.new").LastWriteTime = Get-Date
        if (Test-Path -LiteralPath $target) {
            [IO.File]::Replace("$target.new", $target, [Management.Automation.Language.NullString]::Value)
        } else {
            [IO.File]::Move("$target.new", $target)
        }
        Report-Set "file $target"
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
    # Windows PowerShell passes $null to a string parameter as "", which
    # File.Replace refuses as a backup path.
    if (Test-Path -LiteralPath $Path) {
        [IO.File]::Replace($tmp, $Path, [Management.Automation.Language.NullString]::Value)
    } else {
        [IO.File]::Move($tmp, $Path)
    }
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

# Task $Name, or $null when Windows says there is none; any other failure
# to look it up stops the setup rather than reading as "no task".
function Get-RunnerTask([string] $Name) {
    try {
        return Get-ScheduledTask -TaskName $Name -ErrorAction Stop
    } catch {
        # The task query's own "no such task", not any ObjectNotFound (a
        # missing ScheduledTasks module reports that too).
        if ($_.FullyQualifiedErrorId -like 'CmdletizationQuery_NotFound*') { return $null }
        throw
    }
}

# Disables queue $Queue's task and ends its runner. The disabled task no
# longer matches its record, so Install-RunnerTask registers it again and
# the runner restarts, even after a setup stopped half-way. Only a task that
# does not exist yet is passed over.
function Suspend-Runner([string] $Queue) {
    if (Get-RunnerTask "wn01-runner-$Queue") {
        Disable-ScheduledTask -TaskName "wn01-runner-$Queue" -ErrorAction Stop | Out-Null
    }
    Stop-Runner $Queue
}

function Suspend-Runners { 'normal', 'high' | ForEach-Object { Suspend-Runner $_ } }

# Whether process $Id still runs. Windows keeps an exited process findable
# while another program holds a handle to it, so being found is not enough.
function Test-ProcessAlive([int] $Id) {
    $process = Get-Process -Id $Id -ErrorAction SilentlyContinue
    return $null -ne $process -and -not $process.HasExited
}

# Ends queue $Queue's runner and fails unless it is gone within 10 s.
function Stop-Runner([string] $Queue) {
    Stop-ScheduledTask -TaskName "wn01-runner-$Queue" -ErrorAction SilentlyContinue
    # Ending the task can leave its PowerShell running.
    foreach ($process in @(Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" |
                Where-Object { $_.CommandLine -like "*-File $Root\runner.ps1 -Queue $Queue" })) {
        Invoke-CimMethod -InputObject $process -MethodName Terminate | Out-Null
        Wait-Process -Id $process.ProcessId -Timeout 10 -ErrorAction SilentlyContinue
        if (Test-ProcessAlive $process.ProcessId) {
            throw "the $Queue runner (process $($process.ProcessId)) did not end"
        }
    }
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
    # A ready file that names no pid is stale.
    $id = 0
    if (-not [int]::TryParse((Get-Content -LiteralPath $ready.FullName -Raw), [ref] $id)) { return $false }
    if (-not (Test-ProcessAlive $id)) { return $false }
    $process = Get-Process -Id $id
    if ($process.ProcessName -ne 'powershell') { return $false }
    $installed = ($RunnerCode | ForEach-Object { (Get-Item -LiteralPath (Join-Path $Root $_)).LastWriteTime } |
            Measure-Object -Maximum).Maximum
    return $process.StartTime -gt $installed -and $process.StartTime -le $ready.LastWriteTime
}

# The definition the setup registers for a runner task, as text: a change to
# it in a later setup shows as a difference.
function Get-TaskText($Action, $Trigger, $Principal, $Settings) {
    $limits = '{0}|{1}|{2}|{3}|{4}' -f $Settings.ExecutionTimeLimit, $Settings.MultipleInstances, $Settings.RestartCount,
        $Settings.RestartInterval, $Settings.DisallowStartIfOnBatteries
    return '{0} {1}|{2}:{3}|{4}:{5}:{6}|{7}' -f $Action.Execute, $Action.Arguments, $Trigger.CimClass.CimClassName, $Trigger.UserId,
        $Principal.UserId, $Principal.LogonType, $Principal.RunLevel, $limits
}

# Whether task $Name is registered, as $Record says it was registered with
# definition $Want.
function Test-TaskRecorded([string] $Name, [string] $Record, [string] $Want) {
    if ($null -eq (Get-RunnerTask $Name)) { return $false }
    if (-not (Test-Path -LiteralPath $Record)) { return $false }
    return (Get-Content -LiteralPath $Record -Raw) -ceq "$Want`n$(Export-ScheduledTask -TaskName $Name)"
}

# The VM's runner task (scripts/windows-vm/guest/setup.ps1). Registering it
# records, in C:\wn01, the definition it was given and Windows' own export
# of the task as registered. The task is registered again when either
# differs now: the setup wants another definition, or anything in the task
# changed since (its account, triggers, conditions or settings, whatever
# changed them). Returns whether it was.
function Install-RunnerTask([string] $Queue, [string] $RunLevel) {
    $name = "wn01-runner-$Queue"
    $record = Join-Path $Root "task-$Queue.txt"
    $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument "--headless powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Root\runner.ps1 -Queue $Queue"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
    $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel $RunLevel
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    # tester's SID too: an account made again under the same name is another.
    $want = "$(Get-TaskText $action $trigger $principal $settings)|$((Get-LocalUser -Name $User).SID.Value)"
    if (Test-TaskRecorded $name $record $want) {
        Report-Ok "task $name"
        return $false
    }
    Stop-Runner $Queue
    Register-ScheduledTask -TaskName $name -Description "Desktop runner for pc run ($Queue queue)" -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Write-DurableFile $record "$want`n$(Export-ScheduledTask -TaskName $name)"
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

# tester's password is $Password, and Windows does not want it changed at
# the next logon, which would stop an automatic logon at a prompt.
function Set-TesterPassword([string] $Password) {
    $check = [Wn13.Autologon]::CheckPassword($User, $Password)
    if ($check -eq [Wn13.Autologon]::WrongPassword) {
        Set-LocalUser -Name $User -Password (ConvertTo-SecureString $Password -AsPlainText -Force)
        Report-Set "tester's password"
        $check = [Wn13.Autologon]::CheckPassword($User, $Password)
    } else {
        Report-Ok "tester's password"
    }
    if ($check -ne [Wn13.Autologon]::RightButMustChange) { return }
    & net.exe user $User /logonpasswordchg:no | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "net user $User /logonpasswordchg:no failed" }
    Report-Set "tester need not change its password at logon"
}

# tester logs in at every boot, with the password from the dev box. The
# account's password is set first: should storing it for Winlogon fail, the
# old one still logs in until a second run.
function Enable-AutoLogon([string] $Password) {
    Set-TesterPassword $Password
    $account = Get-LocalUser -Name $User
    if ($null -eq $account.PasswordExpires) {
        Report-Ok "tester's password never expires"
    } else {
        # Under tester's SID: an account made again under the same name is
        # another, whose setting the setup never changed.
        Save-Original "passwordNeverExpires:$($account.SID.Value)" $false
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
        Report-Set "power $($setting.name) on AC: never (was $value s)"
    }
    # Always: a setup stopped between a write and this has the value stored
    # but not in effect.
    & powercfg.exe /setactive $plan
    if ($LASTEXITCODE -ne 0) { throw 'powercfg /setactive failed' }
}

# Runs an installer, ended after $InstallSeconds, and fails unless it exits
# with one of $Ok.
function Invoke-Installer([string] $Path, [string[]] $Arguments, [int[]] $Ok = @(0)) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -NoNewWindow
    # Read once now: without it Windows PowerShell can lose the exit code.
    $null = $process.Handle
    if (-not $process.WaitForExit($InstallSeconds * 1000)) {
        # winget and Visual Studio's setup start installers of their own.
        Stop-Tree $process.Id
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
        $tools = & "$installer\vswhere.exe" -products Microsoft.VisualStudio.Product.BuildTools -latest -property installationPath
    }
    $add = '--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
    # A runner started before the toolchain changes hands its jobs an old PATH.
    Suspend-Runners
    if ($tools) {
        Invoke-Installer "$installer\setup.exe" @('modify', '--installPath', "`"$tools`"", $add, '--quiet', '--norestart') @(0, 3010)
    } else {
        Install-WingetPackage 'Microsoft.VisualStudio.2022.BuildTools' @('--override', "`"--wait --quiet --norestart $add`"")
    }
    Report-Set 'Build Tools with the C++ tools and a Windows SDK'
}

# Ends process $Id and everything it started, within 30 s, and fails when
# the process still runs after that. The setup's lock goes with the setup's
# own process, so the message tells the next run to wait.
function Stop-Tree([int] $Id) {
    $kill = Start-Process -FilePath taskkill.exe -ArgumentList '/T', '/F', '/PID', $Id -PassThru -NoNewWindow
    $null = $kill.Handle
    if (-not $kill.WaitForExit(30000)) { $kill.Kill() }
    Wait-Process -Id $Id -Timeout 10 -ErrorAction SilentlyContinue
    if (Test-ProcessAlive $Id) {
        throw "process $Id could not be ended and may still change the PC: wait for it to end before running the setup again"
    }
}

# Runs $Path with $Arguments, ending it and everything it started after
# $Seconds, and returns its exit code and its stdout's lines; its stderr is
# shown when it fails. Nothing the setup runs while it holds its lock may
# wait without a deadline.
function Invoke-Bounded([string] $Path, [string[]] $Arguments, [int] $Seconds) {
    $info = New-Object Diagnostics.ProcessStartInfo $Path
    $info.Arguments = $Arguments -join ' '
    $info.UseShellExecute = $false
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = [Diagnostics.Process]::Start($info)
    $out = $process.StandardOutput.ReadToEndAsync()
    $err = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit($Seconds * 1000)) {
        Stop-Tree $process.Id
        throw "$Path $($info.Arguments) did not finish within $Seconds s"
    }
    # Its output once the pipes close, which a child it left may delay.
    [void] [Threading.Tasks.Task]::WaitAll(@($out, $err), 10000)
    if ($process.ExitCode -ne 0 -and $err.IsCompleted) { Write-Host $err.Result }
    $lines = if ($out.IsCompleted) { @($out.Result -split "`r?`n" | Where-Object { $_ }) } else { @() }
    return [pscustomobject] @{ Code = $process.ExitCode; Lines = $lines }
}

# Runs rustup with these arguments, within $RustupSeconds, and returns its
# stdout's lines; fails when it does.
function Invoke-Rustup([string[]] $Arguments) {
    $run = Invoke-Bounded (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe') $Arguments $RustupSeconds
    if ($run.Code -ne 0) { throw "rustup $($Arguments -join ' ') failed" }
    return $run.Lines
}

# What Rust lacks of stable as tester's default with clippy and rustfmt:
# 'rustup', 'default' or 'components', in the order they are fixed.
function Get-MissingRust {
    if (-not (Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe'))) {
        return @('rustup', 'default', 'components')
    }
    # It fails when no default is set: that is a missing default too.
    $default = Invoke-Bounded (Join-Path $env:USERPROFILE '.cargo\bin\rustup.exe') @('default') $RustupSeconds
    if ($default.Code -ne 0 -or "$($default.Lines)" -notmatch '^stable-') {
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
    # A runner started before the toolchain changes hands its jobs an old PATH.
    Suspend-Runners
    if ('rustup' -in $missing) { Install-WingetPackage 'Rustlang.Rustup' }
    if ('default' -in $missing) { Invoke-Rustup @('default', 'stable') | Out-Host }
    if ('components' -in $missing) { Invoke-Rustup @('component', 'add', 'clippy', 'rustfmt', '--toolchain', 'stable') | Out-Host }
    Report-Set 'Rust stable with clippy and rustfmt'
}

$Password = [Console]::In.ReadLine()
if ([string]::IsNullOrEmpty($Password)) { throw 'no password on stdin' }
# One setup or uninstall at a time on the PC itself. The dev box's lock is
# not enough: when the dev box gives up on one, ending its SSH client, the
# script goes on running here. The lock is a file in tester's own profile,
# which no other account can reach, held open without sharing; Windows lets
# it go when the process ends, however it ends.
function Lock-Setup {
    $path = Join-Path $env:LOCALAPPDATA 'wn13-pc-setup.lock'
    $deadline = (Get-Date).AddSeconds(60)
    while ($true) {
        try {
            return [IO.File]::Open($path, 'OpenOrCreate', 'ReadWrite', 'None')
        } catch [IO.IOException] {
            if ((Get-Date) -gt $deadline) {
                throw 'another setup or uninstall still runs on the PC (one the dev box gave up on, maybe): let it finish, then run this again'
            }
            Start-Sleep -Seconds 1
        }
    }
}

# Removes staging folders other runs left (one whose script never started)
# once a day old: a run waits at most a minute for the lock, so none that
# old is in use. Housekeeping that never fails the run.
function Remove-OldStages {
    $cutoff = (Get-Date).AddDays(-1)
    Get-ChildItem -LiteralPath (Split-Path $PSScriptRoot) -Directory -Filter 'wn13-setup-*' -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -ne $PSScriptRoot -and $_.LastWriteTime -lt $cutoff } |
        ForEach-Object { Remove-Item -LiteralPath $_.FullName -Recurse -Force -ErrorAction SilentlyContinue }
}

$SetupLock = $null
try {
    $SetupLock = Lock-Setup
    Remove-OldStages
    Add-Type -Path (Join-Path $PSScriptRoot 'Autologon.cs')
    $testerSid = (Get-LocalUser -Name $User).SID.Value
    Install-Folders $testerSid
    Install-RunnerFiles
    # No sleep during the long installs that follow.
    Disable-Sleep
    # The toolchain before the runners: a runner started earlier would hand
    # its jobs a PATH without a newly installed cargo.
    Install-BuildTools
    Install-Rust
    Install-Runner 'normal' 'Limited'
    Install-Runner 'high' 'Highest'
    Enable-AutoLogon $Password
    if ($script:Changed.Count -eq 0) {
        Write-Output 'setup: nothing changed'
    } else {
        Write-Output "setup: changed $($script:Changed.Count): $($script:Changed -join ', ')"
    }
} finally {
    Set-Location C:\
    Remove-Item -LiteralPath $PSScriptRoot -Recurse -Force -ErrorAction SilentlyContinue
    if ($SetupLock) { $SetupLock.Dispose() }
}
# Set explicitly: $LASTEXITCODE would be whatever a tolerated native
# command last left.
exit 0
