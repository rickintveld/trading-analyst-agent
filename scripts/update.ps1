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

# Skip the download when the installed binary already matches the release tag.
if (($Tag -eq "latest") -and (Test-Path $Dest)) {
    try {
        $Resolved = (Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" `
            -MaximumRedirection 0 -SkipHttpErrorCheck -ErrorAction SilentlyContinue).Headers.Location
        $ResolvedTag = if ($Resolved) { ($Resolved -split "/tag/")[-1] } else { "" }
        $Installed = (& $Dest --version) -split " " | Select-Object -Last 1
        if ($ResolvedTag -and $Installed -and ("v$Installed" -eq $ResolvedTag)) {
            Write-Host "up to date: $Dest ($ResolvedTag)"
            exit 0
        }
    } catch { } # fall through to a fresh download
}

Write-Host "downloading $Asset ($Tag) from $Repo ..."
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
$Tmp = "$Dest.download"
Invoke-WebRequest -Uri $Url -OutFile $Tmp
Move-Item -Force $Tmp $Dest
Write-Host "installed: $Dest"
& $Dest --version
