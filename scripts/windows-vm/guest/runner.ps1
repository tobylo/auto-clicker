<#
.SYNOPSIS
Runs jobs in tester's desktop session, one at a time, each with a deadline.

.DESCRIPTION
Started at tester's logon by a scheduled task (setup.ps1 registers two: the
"normal" queue unelevated, the "high" queue elevated). A program started over
SSH has no desktop to inject input into; one started here has.

A job is <id>.json in the queue's inbox, written by run-job.ps1:
  { "command": "...", "workDir": "...", "timeoutSeconds": 120,
    "expiresUtc": "2026-11-25T10:00:00.0000000Z" }
The runner moves it to work\, runs it through JobRunner (its own Win32 job
object, ended at expiresUtc and when the command exits), and writes
outbox\<id>\stdout.txt, stderr.txt and, last, result.json.
#>
param(
    [Parameter(Mandatory)] [ValidateSet('normal', 'high')] [string] $Queue
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Root = 'C:\wn01'
$QueueDir = Join-Path $Root "queue\$Queue"
$Inbox = Join-Path $QueueDir 'inbox'
$Work = Join-Path $QueueDir 'work'
$Outbox = Join-Path $QueueDir 'outbox'
$MaxTimeoutSeconds = 3600
# The watcher can drop events, so the inbox is also rescanned this often.
$RescanSeconds = 30

Add-Type -Path (Join-Path $Root 'JobRunner.cs')

function Write-AtomicFile([string] $Path, [string] $Text) {
    $tmp = "$Path.tmp"
    [IO.File]::WriteAllText($tmp, $Text)
    Move-Item -LiteralPath $tmp -Destination $Path -Force
}

function Read-Job([string] $Path) {
    # run-job.ps1 writes UTF-8 without a BOM, which Windows PowerShell would
    # otherwise read in the ANSI code page.
    $job = Get-Content -LiteralPath $Path -Raw -Encoding UTF8 | ConvertFrom-Json
    Assert-JobCommand $job
    $timeout = [int] $job.timeoutSeconds
    if ($timeout -lt 1 -or $timeout -gt $MaxTimeoutSeconds) {
        throw "timeoutSeconds must be 1 to $MaxTimeoutSeconds"
    }
    return $job, (Get-JobExpiry $job)
}

function Assert-JobCommand($Job) {
    if (-not ($Job.command -is [string]) -or $Job.command.Length -eq 0) {
        throw 'job has no command'
    }
    if (-not ($Job.workDir -is [string]) -or -not (Test-Path -LiteralPath $Job.workDir -PathType Container)) {
        throw "job's workDir is not a folder"
    }
}

# The job's absolute deadline, as the UTC round-trip time run-job.ps1 wrote.
function Get-JobExpiry($Job) {
    if (-not ($Job.expiresUtc -is [string])) { throw 'job has no expiresUtc' }
    $expires = [DateTime]::ParseExact($Job.expiresUtc, 'o', [Globalization.CultureInfo]::InvariantCulture,
        [Globalization.DateTimeStyles]::RoundtripKind)
    if ($expires.Kind -ne [DateTimeKind]::Utc) { throw 'expiresUtc is not a UTC time' }
    return $expires
}

# What is left of the job's time, in milliseconds: its absolute deadline, set
# when it was handed in, counts the time it waited in the queue. Never more
# than its own timeout, whatever the clock says.
function Get-MillisecondsLeft([int] $TimeoutSeconds, [DateTime] $Expires) {
    $left = ($Expires - [DateTime]::UtcNow).TotalMilliseconds
    return [uint32] [Math]::Max(0, [Math]::Min($left, $TimeoutSeconds * 1000.0))
}

function Invoke-Job([IO.FileInfo] $File) {
    $id = $File.BaseName
    $claimed = Join-Path $Work $File.Name
    Move-Item -LiteralPath $File.FullName -Destination $claimed -Force
    $out = Join-Path $Outbox $id
    New-Item -ItemType Directory -Path $out -Force | Out-Null
    $stdout = Join-Path $out 'stdout.txt'
    $stderr = Join-Path $out 'stderr.txt'
    try {
        $job, $expires = Read-Job $claimed
        $ms = Get-MillisecondsLeft ([int] $job.timeoutSeconds) $expires
        $spec = New-Object Wn01.JobSpec
        # Through cmd.exe, so a job may use its syntax: pipes, &&, redirection.
        $spec.CommandLine = 'cmd.exe /d /s /c "' + $job.command + '"'
        $spec.WorkDir = $job.workDir
        $spec.StdoutPath = $stdout
        $spec.StderrPath = $stderr
        $spec.TimeoutMs = $ms
        $run = [Wn01.JobRunner]::Run($spec)
        $result = @{ exitCode = $run.ExitCode; timedOut = $run.TimedOut; error = $null }
    } catch {
        $result = @{ exitCode = 125; timedOut = $false; error = $_.Exception.Message }
    } finally {
        Remove-Item -LiteralPath $claimed -Force -ErrorAction SilentlyContinue
    }
    Write-AtomicFile (Join-Path $out 'result.json') ($result | ConvertTo-Json -Compress)
}

function Invoke-Pending {
    foreach ($file in (Get-ChildItem -LiteralPath $Inbox -Filter '*.json' | Sort-Object Name)) {
        try {
            Invoke-Job $file
        } catch {
            # The client times out on this job; the runner keeps serving others.
            Add-Content -LiteralPath (Join-Path $QueueDir 'runner.log') "$(Get-Date -Format o) $($file.Name): $_"
        }
    }
}

foreach ($dir in $Inbox, $Work, $Outbox) {
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
}
# A job claimed by a runner that died never finishes: answer it as failed.
Get-ChildItem -LiteralPath $Work -Filter '*.json' | ForEach-Object {
    $out = Join-Path $Outbox $_.BaseName
    New-Item -ItemType Directory -Path $out -Force | Out-Null
    $failed = @{ exitCode = 125; timedOut = $false; error = 'the runner stopped during this job' }
    Write-AtomicFile (Join-Path $out 'result.json') ($failed | ConvertTo-Json -Compress)
    Remove-Item -LiteralPath $_.FullName -Force
}

# The watcher's events wait in PowerShell's event queue while a job runs, so
# none is lost between a scan of the inbox and the next wait. Every name: a
# job arrives by a rename from <id>.tmp, which a '*.json' filter misses.
$watcher = New-Object IO.FileSystemWatcher $Inbox, '*'
$watcher.IncludeSubdirectories = $false
foreach ($change in 'Created', 'Renamed') {
    Register-ObjectEvent -InputObject $watcher -EventName $change -SourceIdentifier "wn01-inbox-$change" | Out-Null
}
$watcher.EnableRaisingEvents = $true
Write-AtomicFile (Join-Path $QueueDir 'ready') "$PID"
while ($true) {
    # Events up to here are covered by the scan that follows.
    Get-Event | Remove-Event
    Invoke-Pending
    Wait-Event -Timeout $RescanSeconds | Remove-Event
}
