# DeepScan launcher for Windows. Installed alongside the bundled binaries
# by the MSI (see deepscan.wxs); starts the engine + daemon and opens the
# UI in the default browser. Mirrors packaging/macos/launcher.sh.

$ErrorActionPreference = "Stop"
$InstallDir = Split-Path -Parent $MyInvocation.MyCommand.Path

$env:DEEPSCAN_ENV = "production"
$env:DEEPSCAN_MODE = "local"

$engine = Start-Process -FilePath "$InstallDir\bin\deepscan-engine.exe" -PassThru -WindowStyle Hidden

$lockPath = "$env:USERPROFILE\.deepscan\engine.lock"
for ($i = 0; $i -lt 30; $i++) {
    if (Test-Path $lockPath) { break }
    Start-Sleep -Milliseconds 500
}

$daemon = Start-Process -FilePath "$InstallDir\bin\deepscan-daemon.exe" -PassThru -WindowStyle Hidden

$parserJar = "$InstallDir\lib\deepscan-parser.jar"
$jre = "$InstallDir\jre\bin\java.exe"
if ((Test-Path $parserJar) -and (Test-Path $jre)) {
    Start-Process -FilePath $jre -ArgumentList "-jar", "`"$parserJar`"" -WindowStyle Hidden
}

$port = 51424
if (Test-Path $lockPath) {
    $port = (Get-Content $lockPath | ConvertFrom-Json).port
}
Start-Process "http://127.0.0.1:$port"

Wait-Process -Id $engine.Id
