# DeepScan launcher for Windows. Installed alongside the bundled binaries
# by the MSI (see deepscan.wxs); starts the engine + daemon and opens the
# UI in the default browser. Mirrors packaging/macos/launcher.sh.

$ErrorActionPreference = "Stop"
$InstallDir = Split-Path -Parent $MyInvocation.MyCommand.Path

$env:DEEPSCAN_ENV = "production"
$env:DEEPSCAN_MODE = "local"
# Point directly at the bundled, read-only models instead of copying
# ~275MB into the user's profile on first launch.
$env:DEEPSCAN_MODEL_DIR = "$InstallDir\models"

$logPath = "$env:USERPROFILE\.deepscan-launch.log"
$engine = Start-Process -FilePath "$InstallDir\bin\deepscan-engine.exe" -PassThru -WindowStyle Hidden `
    -RedirectStandardOutput $logPath -RedirectStandardError "$logPath.err"

$lockPath = "$env:USERPROFILE\.deepscan\engine.lock"
$engineAlive = $true
for ($i = 0; $i -lt 30; $i++) {
    if (Test-Path $lockPath) { break }
    if ($engine.HasExited) { $engineAlive = $false; break }
    Start-Sleep -Milliseconds 500
}

if (-not $engineAlive -or -not (Test-Path $lockPath)) {
    Add-Type -AssemblyName PresentationFramework
    [System.Windows.MessageBox]::Show(
        "The DeepScan engine exited on startup. Log: $logPath",
        "DeepScan couldn't start", "OK", "Error")
    exit 1
}

Start-Process -FilePath "$InstallDir\bin\deepscan-daemon.exe" -WindowStyle Hidden

$parserJar = "$InstallDir\lib\deepscan-parser.jar"
$jre = "$InstallDir\jre\bin\java.exe"
if ((Test-Path $parserJar) -and (Test-Path $jre)) {
    Start-Process -FilePath $jre -ArgumentList "-jar", "`"$parserJar`"" -WindowStyle Hidden
}

# DEEPSCAN_ENGINE_HTTP_PORT isn't set here, so this matches config.rs's own
# default (51424) exactly — engine.lock only records the gRPC port, not
# this one, so reading the port from it would grab the wrong value.
#
# Chrome's --app= mode opens a standalone window with no address bar/tabs
# instead of a normal browser tab, so DeepScan feels like its own app. A
# dedicated --user-data-dir keeps this profile separate from the user's
# regular Chrome session. Falls back to the default browser (a normal tab)
# if Chrome isn't installed at either common location.
$chromePaths = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
)
$chrome = $chromePaths | Where-Object { Test-Path $_ } | Select-Object -First 1

if ($chrome) {
    Start-Process -FilePath $chrome -ArgumentList `
        "--app=http://127.0.0.1:51424", `
        "--user-data-dir=$env:USERPROFILE\.deepscan\chrome-app-profile"
} else {
    Start-Process "http://127.0.0.1:51424"
}

Wait-Process -Id $engine.Id
