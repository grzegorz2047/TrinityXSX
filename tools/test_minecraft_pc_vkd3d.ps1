<#
.SYNOPSIS
Uruchamia Minecraft Preview (Scarlett) lokalnie na Windows ze stosem graficznym używanym na PS5:
  D:\TrinityXSX\dist\prospero-win\prefix\drive_c\XCL\xcl.exe run --in-process
  + spatchowany vkd3d-proton 3.1 + DXVK 3.1.1 + XCL_DIRECT_DXIL=all + XCL_NETWORK=0

.DESCRIPTION
Pozwala porównać działanie gry na Windows i PS5; test PC nie sprawdza zachowania Wine na PS5.
#>
param(
    [string]$TrinityRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$GameDir,
    [string]$WorkDataDir,
    [switch]$UseWindowsD3D12,
    [switch]$TraceApu,
    [switch]$ForceHleRuntime,
    [switch]$TraceAsync
)

$ErrorActionPreference = "Stop"
if (-not $GameDir) { $GameDir = Join-Path $TrinityRoot "games\minecraft-beta" }
if (-not $WorkDataDir) { $WorkDataDir = Join-Path $TrinityRoot "analysis\minecraft-pc" }

$XclDir = if ($UseWindowsD3D12) {
    Join-Path $TrinityRoot "xcl\target\release"
} else {
    Join-Path $TrinityRoot "dist\prospero-win\prefix\drive_c\XCL"
}
$XclExe = Join-Path $XclDir "xcl.exe"

if (-not (Test-Path $XclExe)) {
    throw "Nie znaleziono $XclExe. Zbuduj XCL (cargo build --workspace --release) i tools/make-ps5-package.sh."
}
if (-not (Test-Path (Join-Path $GameDir "Minecraft.Windows.exe"))) {
    throw "Nie znaleziono Minecraft.Windows.exe w $GameDir."
}

$TitleDataDir = Join-Path $WorkDataDir "Microsoft.MinecraftBeta"
$CacheDir     = Join-Path $WorkDataDir "vkd3d_cache"
New-Item -ItemType Directory -Force -Path $TitleDataDir, $CacheDir | Out-Null

# Skopiuj PS5-owy xcl.toml (jeśli nie ma jeszcze nadpisanego przez użytkownika)
$SrcToml = Join-Path $PSScriptRoot "..\configs\minecraft.xcl.toml"
$DstToml = Join-Path $TitleDataDir "xcl.toml"
if (-not (Test-Path $DstToml)) {
    Copy-Item $SrcToml $DstToml
    Write-Host "[INFO] Zapisano profil konfiguracji: $DstToml"
}

$env:XCL_DATA_ROOT           = $WorkDataDir
$env:XCL_DIRECT_DXIL         = "all"
$env:XCL_NETWORK             = "0"
$env:VKD3D_DEBUG             = "warn"
$env:VKD3D_LOG_FILE          = (Join-Path $WorkDataDir "vkd3d-minecraft.log")
$env:VKD3D_SHADER_CACHE_PATH = $CacheDir
$env:DXVK_LOG_PATH           = $WorkDataDir
if ($TraceApu)  { $env:XCL_TRACE_APU = "1" }
$PreviousForceHleRuntime = $env:XCL_FORCE_HLE_RUNTIME
$PreviousTraceAsync = $env:XCL_TRACE_ASYNC
if ($ForceHleRuntime) { $env:XCL_FORCE_HLE_RUNTIME = "1" }
if ($TraceAsync) { $env:XCL_TRACE_ASYNC = "1" }

$BackendLabel = if ($UseWindowsD3D12) { "Windows D3D12 (referencyjny)" } else { "vkd3d-proton 3.1 + DXVK 3.1.1 (jak na PS5!)" }
Write-Host "=========================================================="
Write-Host " XCL-PS5 PC Dry-Run: Minecraft Preview (Scarlett)"
Write-Host " Backend : $BackendLabel"
Write-Host " XCL     : $XclExe"
Write-Host " Gra     : $GameDir"
Write-Host " Logi    : $WorkDataDir"
Write-Host "=========================================================="

Push-Location $XclDir
try {
    & $XclExe run --in-process $GameDir
    $ExitCode = $LASTEXITCODE
} finally {
    Pop-Location
    $env:XCL_FORCE_HLE_RUNTIME = $PreviousForceHleRuntime
    $env:XCL_TRACE_ASYNC = $PreviousTraceAsync
}

Write-Host "`n[KONIEC] Kod wyjścia: $ExitCode"
Write-Host "Sprawdź logi:"
Write-Host "  - $(Join-Path $TitleDataDir 'logs\xcl.log')"
Write-Host "  - $(Join-Path $WorkDataDir 'vkd3d-minecraft.log')"
