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
$script:FilesChanged = $false

function Report-Ok([string] $What) { Write-Output "ok   $What" }

function Report-Set([string] $What) {
    $script:Changed.Add($What)
    Write-Output "set  $What"
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

# Whether folder security $Have is owned by the administrators or SYSTEM,
# and protects and lists its explicit rules as $Want does.
function Test-FolderSecurity($Have, $Want) {
    $owner = $Have.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($owner -notin $Admins, $System) { return $false }
    if ($Have.AreAccessRulesProtected -ne $Want.AreAccessRulesProtected) { return $false }
    return (Get-RuleText $Have) -eq (Get-RuleText $Want)
}

# Makes folder $Path with exactly these explicit rules, owned by the
# administrators; $Protected stops it inheriting from its parent.
function Set-Folder([string] $Path, [bool] $Protected, [object[]] $Rules) {
    $made = -not (Test-Path -LiteralPath $Path -PathType Container)
    if ($made) { New-Item -ItemType Directory -Path $Path | Out-Null }
    $want = New-Object Security.AccessControl.DirectorySecurity
    $want.SetAccessRuleProtection($Protected, $false)
    foreach ($rule in $Rules) { $want.AddAccessRule($rule) }
    if (-not $made -and (Test-FolderSecurity (Get-Acl -LiteralPath $Path) $want)) {
        Report-Ok "folder $Path"
        return
    }
    $want.SetOwner((New-Object Security.Principal.SecurityIdentifier $Admins))
    Set-Acl -LiteralPath $Path -AclObject $want
    Report-Set "folder $Path"
}

# The runner's folders. Only administrators and SYSTEM write the runner's
# files and hand in jobs, so nothing unelevated (a job in the normal queue
# included) can change the runner or give the elevated one a job. The
# unelevated runner writes its own queue's ready, log, work and outbox, and
# claims a job by moving it out of the inbox, which it can read and delete
# from but not write to. A moved job keeps the inbox's rules, and the runner
# removes it with Remove-Item -Force, which writes its attributes first.
function Install-Folders([string] $TesterSid) {
    $full = 'FullControl'
    Set-Folder $Root $true @((New-Rule $Admins $full), (New-Rule $System $full), (New-Rule $TesterSid 'ReadAndExecute'))
    Set-Folder "$Root\queue" $false @()
    Set-Folder "$Root\queue\high" $false @()
    Set-Folder "$Root\queue\normal" $false @(New-Rule $TesterSid 'Modify')
    Set-Folder "$Root\queue\normal\inbox" $true @(
        (New-Rule $Admins $full), (New-Rule $System $full),
        (New-Rule $TesterSid 'ReadAndExecute' 'None'),
        (New-Rule $TesterSid 'ReadAndExecute, Delete, WriteAttributes' 'ObjectInherit' 'InheritOnly'))
}

# Copies each runner file whose content differs, and notes in
# $script:FilesChanged whether any did.
function Install-RunnerFiles {
    foreach ($name in $RunnerFiles) {
        $source = Join-Path $PSScriptRoot $name
        $target = Join-Path $Root $name
        if ((Test-Path -LiteralPath $target) -and
            (Get-FileHash -LiteralPath $target).Hash -eq (Get-FileHash -LiteralPath $source).Hash) {
            Report-Ok "file $target"
            continue
        }
        Copy-Item -LiteralPath $source -Destination $target -Force
        Report-Set "file $target"
        $script:FilesChanged = $true
    }
}

# Records a setting's value from before the first setup, once.
function Save-Original([string] $Name, $Value) {
    $path = Join-Path $Root 'restore.json'
    $saved = @{}
    if (Test-Path -LiteralPath $path) {
        (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json).PSObject.Properties | ForEach-Object { $saved[$_.Name] = $_.Value }
    }
    if ($saved.ContainsKey($Name)) { return }
    $saved[$Name] = $Value
    [IO.File]::WriteAllText("$path.tmp", ($saved | ConvertTo-Json -Compress))
    Move-Item -LiteralPath "$path.tmp" -Destination $path -Force
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

# The VM's runner task (scripts/windows-vm/guest/setup.ps1), with a hash of
# its definition in its description so a second run can tell it is current.
# Re-registered when the definition differs; restarted when it was, when
# the runner's files changed, or when it is not running.
function Install-Runner([string] $Queue, [string] $RunLevel, [bool] $FilesChanged) {
    $name = "wn01-runner-$Queue"
    $arguments = "--headless powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $Root\runner.ps1 -Queue $Queue"
    $definition = "conhost.exe|$arguments|AtLogOn:$User|Interactive:$RunLevel|limit 0|IgnoreNew|restart 3 x 1 min|batteries"
    $sha = [Security.Cryptography.SHA256]::Create()
    $hash = -join ($sha.ComputeHash([Text.Encoding]::UTF8.GetBytes($definition)) | ForEach-Object { $_.ToString('x2') })
    $description = "Desktop runner for pc run ($Queue queue). Definition $hash"
    $task = Get-ScheduledTask -TaskName $name -ErrorAction SilentlyContinue
    $restart = $FilesChanged
    if ($null -eq $task -or $task.Description -ne $description) {
        Stop-Runner $Queue
        $action = New-ScheduledTaskAction -Execute 'conhost.exe' -Argument $arguments
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $User
        $principal = New-ScheduledTaskPrincipal -UserId $User -LogonType Interactive -RunLevel $RunLevel
        $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $name -Description $description -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
        Report-Set "task $name"
        $restart = $true
    } else {
        Report-Ok "task $name"
    }
    if (-not (Test-TesterLoggedOn)) {
        Write-Output "     $name starts at tester's next logon (tester is not logged on)"
        return
    }
    $running = (Get-ScheduledTask -TaskName $name).State -eq 'Running'
    if ($running -and -not $restart) {
        Report-Ok "runner $Queue running"
        return
    }
    if ($running) { Stop-Runner $Queue }
    Start-ScheduledTask -TaskName $name
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

# The AC value in seconds of power setting $Guid (SUB_SLEEP) in the
# active plan, read through CIM, whose names do not depend on the language.
function Get-SleepSetting([string] $Guid) {
    $scheme = (Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerPlan -Filter 'IsActive = True').InstanceID -replace '^.*\\', ''
    $index = Get-CimInstance -Namespace root\cimv2\power -ClassName Win32_PowerSettingDataIndex |
        Where-Object { $_.InstanceID -like "*$scheme\AC\{$Guid}" }
    # One instance, or the read failed: never taken as "never sleeps".
    if (@($index).Count -ne 1) { throw "could not read power setting $Guid" }
    return [int] $index.SettingIndexValue
}

# Nobody wakes the PC: it never sleeps or hibernates on AC while set up.
function Disable-Sleep {
    foreach ($setting in @{ name = 'STANDBYIDLE'; guid = '29f6c1db-86da-48c5-9fdb-f2b67b1f44da' },
        @{ name = 'HIBERNATEIDLE'; guid = '9d7815a6-7ee4-497e-8888-515a05f02364' }) {
        $value = Get-SleepSetting $setting.guid
        if ($value -eq 0) {
            Report-Ok "power $($setting.name) on AC: never"
            continue
        }
        Save-Original $setting.name $value
        & powercfg.exe /setacvalueindex SCHEME_CURRENT SUB_SLEEP $setting.name 0
        if ($LASTEXITCODE -ne 0) { throw "powercfg /setacvalueindex $($setting.name) failed" }
        & powercfg.exe /setactive SCHEME_CURRENT
        if ($LASTEXITCODE -ne 0) { throw 'powercfg /setactive failed' }
        Report-Set "power $($setting.name) on AC: never (was $value s)"
    }
}

# Runs an installer, ended after $InstallSeconds, and fails unless it exits
# with one of $Ok.
function Invoke-Installer([string] $Path, [string[]] $Arguments, [int[]] $Ok = @(0)) {
    $process = Start-Process -FilePath $Path -ArgumentList $Arguments -PassThru -NoNewWindow
    if (-not $process.WaitForExit($InstallSeconds * 1000)) {
        Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
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
    Install-Runner 'normal' 'Limited' $script:FilesChanged
    Install-Runner 'high' 'Highest' $script:FilesChanged
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
