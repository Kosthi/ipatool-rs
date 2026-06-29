param(
    [Parameter(Mandatory = $true)]
    [string] $Target,

    [Parameter(Mandatory = $true)]
    [string] $Version
)

$ErrorActionPreference = "Stop"

$binary = Join-Path "target/$Target/release" "ipatool.exe"
$distDir = "dist"
$assetStem = "ipatool-$Version-$Target"
$stageDir = Join-Path "target/package" $assetStem

if (-not (Test-Path $binary)) {
    throw "missing release binary: $binary"
}

Remove-Item $stageDir -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $stageDir | Out-Null
New-Item -ItemType Directory -Force -Path $distDir | Out-Null

Copy-Item $binary (Join-Path $stageDir "ipatool.exe")
Copy-Item "LICENSE" (Join-Path $stageDir "LICENSE")

@"
ipatool $Version

This package contains the ipatool command-line binary for $Target.

Install:
  powershell -ExecutionPolicy Bypass -File .\install.ps1

Verify:
  ipatool --help
"@ | Set-Content -Path (Join-Path $stageDir "README.txt") -Encoding UTF8

@'
$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$installDir = Join-Path $env:LOCALAPPDATA "Programs\ipatool"

New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item (Join-Path $scriptDir "ipatool.exe") (Join-Path $installDir "ipatool.exe") -Force

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$pathEntries = @()
if ($userPath) {
    $pathEntries = $userPath -split ';'
}

if ($pathEntries -notcontains $installDir) {
    $newPath = if ($userPath) { "$userPath;$installDir" } else { $installDir }
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added $installDir to the user PATH. Open a new terminal to use ipatool."
}

Write-Host "Installed ipatool to $installDir\ipatool.exe"
'@ | Set-Content -Path (Join-Path $stageDir "install.ps1") -Encoding UTF8

Copy-Item $binary (Join-Path $distDir "$assetStem.exe")
Compress-Archive -Path (Join-Path $stageDir "*") -DestinationPath (Join-Path $distDir "$assetStem.zip") -Force
