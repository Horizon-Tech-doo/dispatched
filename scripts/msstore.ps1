#!/usr/bin/env pwsh
# Builds the Microsoft Store package of Dispatched: one unsigned .msixbundle for x64 and ARM64, in msstore/.
# The Store signs it after certification. Runs on Windows, with Node, the Rust targets for both architectures
# and the winapp CLI (winget install Microsoft.winappcli). .github/workflows/msstore.yml runs this.
$ErrorActionPreference = 'Stop'

function Invoke-Checked([scriptblock]$Command) {
  & $Command
  if ($LASTEXITCODE -ne 0) { throw "failed with exit code ${LASTEXITCODE}: $Command" }
}

$Root = Split-Path $PSScriptRoot -Parent
$AppDir = Join-Path $Root 'app'
$TauriDir = Join-Path $AppDir 'src-tauri'
$Out = Join-Path $Root 'msstore'
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Targets = [ordered]@{ x64 = 'x86_64-pc-windows-msvc'; arm64 = 'aarch64-pc-windows-msvc' }

# The first part of a Store version can't be 0 and the fourth must be 0, so the Store version is the app version
# with one added to the major part: 0.1.0 is 1.1.0.0 and 1.0.0 is 2.0.0.0, which keeps every update higher.
$AppVersion = (Get-Content (Join-Path $TauriDir 'tauri.conf.json') -Raw | ConvertFrom-Json).version
if ($AppVersion -notmatch '^(\d+)\.(\d+)\.(\d+)$') { throw "can't turn app version '$AppVersion' into a Store version" }
$StoreVersion = "$([int]$Matches[1] + 1).$($Matches[2]).$($Matches[3]).0"
Write-Host "app version $AppVersion, Store version $StoreVersion"

if (Test-Path $Out) { Remove-Item $Out -Recurse -Force }
New-Item $Out -ItemType Directory | Out-Null

$Manifest = (Get-Content (Join-Path $TauriDir 'msstore/Package.appxmanifest') -Raw).Replace('Version="0.0.0.0"', "Version=`"$StoreVersion`"")

Push-Location $AppDir
try {
  foreach ($Arch in $Targets.Keys) {
    $Triple = $Targets[$Arch]
    Invoke-Checked {
      npx tauri build --no-bundle --target $Triple --features custom-protocol `
        --config src-tauri/tauri.msstore.conf.json -- --no-default-features
    }
    $Release = Join-Path $TargetDir "$Triple/release"
    $Exe = @('Dispatched.exe', 'dispatched-app.exe') | ForEach-Object { Join-Path $Release $_ } |
      Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $Exe) { throw "no app executable in $Release" }

    $Stage = Join-Path $Out $Arch
    New-Item $Stage -ItemType Directory | Out-Null
    Copy-Item $Exe (Join-Path $Stage 'Dispatched.exe')
    Set-Content (Join-Path $Stage 'Package.appxmanifest') $Manifest -NoNewline
  }
}
finally { Pop-Location }

# Every icon the manifest names, generated once from the source icon and shared by both architectures.
$First = Join-Path $Out 'x64'
Invoke-Checked {
  winapp manifest update-assets (Join-Path $TauriDir 'icons/source/icon.png') --manifest (Join-Path $First 'Package.appxmanifest')
}
if (-not (Test-Path (Join-Path $First 'Assets/StoreLogo.png'))) { throw 'winapp did not generate the package icons' }
Copy-Item (Join-Path $First 'Assets') (Join-Path $Out 'arm64/Assets') -Recurse

Push-Location $Out
try { Invoke-Checked { winapp pack x64 arm64 --output "Dispatched_$StoreVersion.msixbundle" } }
finally { Pop-Location }

$Bundle = Join-Path $Out "Dispatched_$StoreVersion.msixbundle"
if (-not (Test-Path $Bundle)) { throw "$Bundle was not created" }
Write-Host "built $Bundle"
