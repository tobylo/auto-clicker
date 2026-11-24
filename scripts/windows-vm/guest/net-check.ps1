<#
.SYNOPSIS
Checks from inside the VM what it can and cannot reach.

.DESCRIPTION
Run over SSH by `vm selftest`, which passes the dev box's own addresses and
ports that listen there. Expected: no direct connection reaches anything (not
the dev box, not the LAN, not the internet); the proxy refuses an unlisted
host and every address literal, and passes a listed host. Prints one line per
check and exits 0 only when every check came out as expected.

.PARAMETER Targets
host:port pairs that must stay unreachable, directly and through the proxy.
#>
param(
    [Parameter(Mandatory)] [string[]] $Targets
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Proxy = 'http://10.0.2.100:3128'
$script:failures = 0

function Report([bool] $Ok, [string] $What) {
    if (-not $Ok) { $script:failures++ }
    Write-Output ("{0} {1}" -f $(if ($Ok) { 'ok  ' } else { 'FAIL' }), $What)
}

function Test-Direct([string] $HostName, [int] $Port) {
    $client = New-Object Net.Sockets.TcpClient
    try {
        $task = $client.ConnectAsync($HostName, $Port)
        try { return $task.Wait(3000) -and $client.Connected } catch { return $false }
    } finally {
        $client.Dispose()
    }
}

function Get-ProxyAnswer([string] $Url) {
    $code = & curl.exe --silent --output NUL --max-time 20 --proxy $Proxy --write-out '%{http_connect}' $Url
    return [int] $code
}

foreach ($target in $Targets + @('1.1.1.1:443', '10.0.2.2:22')) {
    $hostName, $port = $target -split ':'
    Report (-not (Test-Direct $hostName ([int] $port))) "no direct connection to $target"
}
foreach ($target in $Targets + @('example.com:443', '127.0.0.1:443', '10.0.2.2:443')) {
    $answer = Get-ProxyAnswer "https://$target/"
    Report ($answer -ne 200) "proxy refuses $target (answered $answer)"
}
$answer = Get-ProxyAnswer 'https://static.rust-lang.org/rustup/release-stable.toml'
Report ($answer -eq 200) "proxy passes static.rust-lang.org (answered $answer)"

if ($script:failures -gt 0) {
    Write-Output "$($script:failures) check(s) failed"
    exit 1
}
