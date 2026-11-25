<#
.SYNOPSIS
Hands one command to the runner in tester's desktop and waits for its result.

.DESCRIPTION
Run over SSH by `vm run`. Writes the job into the queue's inbox (a temp file,
then a rename, so the runner never reads half a job) with an absolute
deadline, TimeoutSeconds from now, which the runner ends it at however long it
waited in the queue. It waits for result.json for that long plus a grace, prints the job's stdout and stderr, and exits with the
job's exit code: 124 when the job hit its deadline, 125 when the runner could
not run it, 126 when no result came back in time.

The command arrives base64-encoded (UTF-8) so no shell between the dev box and
here rewrites its quoting.
#>
param(
    [Parameter(Mandatory)] [string] $CommandBase64,
    [ValidateRange(1, 3600)] [int] $TimeoutSeconds = 120,
    [string] $WorkDir = 'C:\work',
    [ValidateSet('normal', 'high')] [string] $Queue = 'normal'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$QueueDir = "C:\wn01\queue\$Queue"
# Time the runner may take to pick a job up and write its result back.
$GraceSeconds = 60

function Wait-Result([string] $Path, [datetime] $Deadline) {
    # Queued events: a rename that lands between the check and the wait still
    # wakes the wait. Every name: result.json arrives by a rename.
    $watcher = New-Object IO.FileSystemWatcher (Split-Path $Path), '*'
    Register-ObjectEvent -InputObject $watcher -EventName Renamed -SourceIdentifier 'wn01-result' | Out-Null
    $watcher.EnableRaisingEvents = $true
    try {
        while (-not (Test-Path -LiteralPath $Path)) {
            $left = [int] [Math]::Ceiling(($Deadline - (Get-Date)).TotalSeconds)
            if ($left -le 0) {
                return $false
            }
            Wait-Event -SourceIdentifier 'wn01-result' -Timeout $left | Remove-Event
        }
        return $true
    } finally {
        Unregister-Event -SourceIdentifier 'wn01-result'
        $watcher.Dispose()
    }
}

$command = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($CommandBase64))
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
$id = [guid]::NewGuid().ToString('N')
$out = Join-Path $QueueDir "outbox\$id"
New-Item -ItemType Directory -Path $out -Force | Out-Null
$submitted = [DateTime]::UtcNow
$expires = $submitted.AddSeconds($TimeoutSeconds).ToString('o')
$job = @{ command = $command; workDir = $WorkDir; timeoutSeconds = $TimeoutSeconds; expiresUtc = $expires } | ConvertTo-Json -Compress
$inbox = Join-Path $QueueDir 'inbox'
[IO.File]::WriteAllText((Join-Path $inbox "$id.tmp"), $job)
Move-Item -LiteralPath (Join-Path $inbox "$id.tmp") -Destination (Join-Path $inbox "$id.json")

$resultPath = Join-Path $out 'result.json'
try {
    # By then the runner has ended the job, unless the runner itself is gone.
    if (-not (Wait-Result $resultPath $submitted.ToLocalTime().AddSeconds($TimeoutSeconds + $GraceSeconds))) {
        [Console]::Error.WriteLine("no result from the $Queue runner within $($TimeoutSeconds + $GraceSeconds) s; is it running?")
        exit 126
    }
    $result = Get-Content -LiteralPath $resultPath -Raw | ConvertFrom-Json
    foreach ($stream in @{ path = 'stdout.txt'; writer = [Console]::Out }, @{ path = 'stderr.txt'; writer = [Console]::Error }) {
        $file = Join-Path $out $stream.path
        if (Test-Path -LiteralPath $file) {
            $stream.writer.Write([IO.File]::ReadAllText($file))
        }
    }
    if ($result.error) {
        [Console]::Error.WriteLine("runner: $($result.error)")
    }
    if ($result.timedOut) {
        [Console]::Error.WriteLine("runner: the job hit its $TimeoutSeconds s deadline and was ended")
    }
    exit ([int] $result.exitCode)
} finally {
    Remove-Item -LiteralPath (Join-Path $inbox "$id.json") -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $out -Recurse -Force -ErrorAction SilentlyContinue
}
