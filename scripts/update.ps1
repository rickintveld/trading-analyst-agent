# Download the latest trade-analyzer release binary for Windows into
# scripts\bin\trade-analyzer.exe.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File scripts\update.ps1            # latest
#   powershell -ExecutionPolicy Bypass -File scripts\update.ps1 v0.2.0    # specific tag
param([string]$Tag = "latest")

$ErrorActionPreference = "Stop"
$Repo = if ($env:REPO) { $env:REPO } else { "rickintveld/trading-analyst-agent" }
$Asset = "trading-analyst-agent-windows.exe"
$BinDir = Join-Path $PSScriptRoot "bin"
$Dest = Join-Path $BinDir "trade-analyzer.exe"

if ($Tag -eq "latest") {
    $Url = "https://github.com/$Repo/releases/latest/download/$Asset"
} else {
    $Url = "https://github.com/$Repo/releases/download/$Tag/$Asset"
}

Write-Host "downloading $Asset ($Tag) from $Repo ..."
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
$Tmp = "$Dest.download"
Invoke-WebRequest -Uri $Url -OutFile $Tmp
Move-Item -Force $Tmp $Dest
Write-Host "installed: $Dest"
& $Dest --version
