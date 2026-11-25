<#
.SYNOPSIS
One Windows Update pass: search, download and install what applies.

.DESCRIPTION
Run as SYSTEM by the scheduled task provision.ps1 registers, because the
Windows Update Agent refuses to download or install from an SSH (network)
logon. Writes C:\wn01\windows-update.json when done:
  { "state": "done" | "reboot" | "failed", "installed": n, "error": "..." }
"done" means nothing was left to install; "reboot" means updates were
installed and Windows needs a restart before the next pass.
#>

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$StatusPath = 'C:\wn01\windows-update.json'

function Write-Status([hashtable] $Status) {
    [IO.File]::WriteAllText("$StatusPath.tmp", ($Status | ConvertTo-Json -Compress))
    Move-Item -LiteralPath "$StatusPath.tmp" -Destination $StatusPath -Force
}

function Install-Pending {
    $session = New-Object -ComObject Microsoft.Update.Session
    $found = $session.CreateUpdateSearcher().Search("IsInstalled=0 and IsHidden=0 and Type='Software'").Updates
    if ($found.Count -eq 0) {
        return @{ state = 'done'; installed = 0; error = $null }
    }
    $batch = Select-Batch $found
    $downloader = $session.CreateUpdateDownloader()
    $downloader.Updates = $batch
    $null = $downloader.Download()
    $installer = $session.CreateUpdateInstaller()
    $installer.Updates = $batch
    $result = $installer.Install()
    # ResultCode 2 is succeeded, 3 succeeded with errors.
    if ($result.ResultCode -notin 2, 3) {
        return @{ state = 'failed'; installed = 0; error = "install result code $($result.ResultCode)" }
    }
    $failed = Get-FailedUpdates $batch $result
    $installed = $batch.Count - $failed.Count
    # Nothing installed: another pass would only fail on the same updates.
    $state = if ($installed -eq 0) { 'failed' } else { 'reboot' }
    return @{ state = $state; installed = $installed; error = ($failed -join '; ') }
}

# What one Install() call takes: an update that must be installed on its
# own (InstallationBehavior.Impact 2) alone, else every update found.
function Select-Batch($Found) {
    $batch = New-Object -ComObject Microsoft.Update.UpdateColl
    $exclusive = @($Found | Where-Object { $_.InstallationBehavior.Impact -eq 2 } | Select-Object -First 1)
    foreach ($update in $(if ($exclusive.Count -gt 0) { $exclusive } else { $Found })) {
        if (-not $update.EulaAccepted) { $update.AcceptEula() }
        $null = $batch.Add($update)
    }
    return , $batch
}

# The updates in the batch that did not install, with their HRESULTs, so a
# build that keeps failing on one says which.
function Get-FailedUpdates($Batch, $Result) {
    $failed = @()
    for ($i = 0; $i -lt $Batch.Count; $i++) {
        $one = $Result.GetUpdateResult($i)
        if ($one.ResultCode -ne 2) {
            $failed += '{0} (result {1}, 0x{2:X8})' -f $Batch.Item($i).Title, $one.ResultCode, $one.HResult
        }
    }
    return , $failed
}

Remove-Item -LiteralPath $StatusPath -Force -ErrorAction SilentlyContinue
try {
    Write-Status (Install-Pending)
} catch {
    Write-Status @{ state = 'failed'; installed = 0; error = $_.Exception.Message }
}
