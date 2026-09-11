param([Parameter(Mandatory=$true)][string]$Candidate)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
# Ephemeral GitHub-hosted runner only: exact final launcher bytes fetch from a
# loopback HTTPS fixture with normal Windows certificate validation intact.
if ($env:GITHUB_ACTIONS -ne 'true' -or $env:RUNNER_ENVIRONMENT -ne 'github-hosted') { throw 'Run only on an ephemeral hosted CI runner' }
$Candidate = (Resolve-Path $Candidate).Path
$hosts = Join-Path $env:SystemRoot 'System32/drivers/etc/hosts'
$savedHosts = [IO.File]::ReadAllBytes($hosts)
$certificate = $null; $server = $null
$binding = $false
$oldLocal = $env:LOCALAPPDATA; $oldTemp = $env:TEMP; $oldTmp = $env:TMP
try {
    $certificate = New-SelfSignedCertificate -DnsName 'github.com' -CertStoreLocation 'Cert:\LocalMachine\My' -NotAfter (Get-Date).AddDays(1)
    $export = Join-Path $Candidate 'fixture.cer'
    Export-Certificate -Cert $certificate -FilePath $export | Out-Null
    Import-Certificate -FilePath $export -CertStoreLocation 'Cert:\LocalMachine\Root' | Out-Null
    & netsh http add sslcert ipport=127.0.0.1:443 certhash=$certificate.Thumbprint 'appid={b0966d2c-a0fa-4ee0-953d-ae064411cbb6}' | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Could not bind fixture certificate' }
    $binding = $true
    [IO.File]::AppendAllText($hosts, "`r`n127.0.0.1 github.com`r`n")
    Clear-DnsClientCache
    $serverArgs = @('-NoProfile','-ExecutionPolicy','Bypass','-File',"`"$PSScriptRoot\https_fixture.ps1`"",'-Assets',"`"$Candidate\assets`"",'-Log',"`"$Candidate\requests.json`"")
    $server = Start-Process powershell -ArgumentList $serverArgs -PassThru -WindowStyle Hidden -RedirectStandardError (Join-Path $Candidate 'https-error.txt')
    $env:LOCALAPPDATA = Join-Path $Candidate 'owner'
    $env:TEMP = Join-Path $Candidate 'temporary'; $env:TMP = $env:TEMP
    New-Item -ItemType Directory -Path $env:LOCALAPPDATA,$env:TEMP | Out-Null
    Start-Sleep -Seconds 1
    & python "$PSScriptRoot\windows_console.py" "$Candidate\launchers\install.ps1" "$Candidate\console.txt" "$Candidate\result.json"
    if ($LASTEXITCODE -ne 0) { throw 'Actual Windows launcher did not safely cancel' }
    if (-not $server.WaitForExit(10000) -or $server.ExitCode -ne 0) { throw 'HTTPS fixture failed' }
    $requests = Get-Content -Raw "$Candidate\requests.json" | ConvertFrom-Json
    if ($requests.Count -ne 3) { throw 'Unexpected downloader activity' }
    if ((Get-ChildItem -Force $env:TEMP).Count -ne 0 -or (Test-Path "$env:LOCALAPPDATA\CouchInstaller")) { throw 'Launcher left temporary files or created a session' }
} finally {
    $env:LOCALAPPDATA = $oldLocal; $env:TEMP = $oldTemp; $env:TMP = $oldTmp
    if ($server -and -not $server.HasExited) { Stop-Process -Id $server.Id -Force }
    [IO.File]::WriteAllBytes($hosts, $savedHosts); Clear-DnsClientCache
    if ($binding) { & netsh http delete sslcert ipport=127.0.0.1:443 | Out-Null }
    if ($certificate) {
        Remove-Item -LiteralPath "Cert:\LocalMachine\Root\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath "Cert:\LocalMachine\My\$($certificate.Thumbprint)" -ErrorAction SilentlyContinue
    }
}
