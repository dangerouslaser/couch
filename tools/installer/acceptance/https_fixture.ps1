param([Parameter(Mandatory=$true)][string]$Assets, [Parameter(Mandatory=$true)][string]$Log)
$ErrorActionPreference = 'Stop'
$listener = [Net.HttpListener]::new()
$listener.Prefixes.Add('https://github.com:443/')
$listener.Start()
$requests = @()
try {
    foreach ($expected in @('couch-installer-host-windows-x64.exe', 'couch-installer-tui-windows-x64.exe', 'installer.json')) {
        $context = $listener.GetContext()
        $path = '/dangerouslaser/couch/releases/download/v0.1.0-alpha.20260910.24/' + $expected
        if (-not [Net.IPAddress]::IsLoopback($context.Request.RemoteEndPoint.Address) -or $context.Request.Url.AbsolutePath -ne $path -or $context.Request.HttpMethod -ne 'GET') {
            $context.Response.StatusCode = 403; $context.Response.Close(); throw 'Unexpected fixture request'
        }
        $bytes = [IO.File]::ReadAllBytes((Join-Path $Assets $expected))
        $context.Response.ContentLength64 = $bytes.Length
        $context.Response.OutputStream.Write($bytes, 0, $bytes.Length)
        $context.Response.Close()
        $requests += $path
    }
    $requests | ConvertTo-Json | Set-Content -LiteralPath $Log -Encoding UTF8
} finally { $listener.Close() }
