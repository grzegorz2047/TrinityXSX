<#
.SYNOPSIS
    PS5 Live Debugger & Hot-Deploy CLI dla XCL / prospero-win (192.168.1.170:2121).
.EXAMPLE
    .\ps5_debug.ps1 -Action Logs -OutTag run8
    .\ps5_debug.ps1 -Action Tail
    .\ps5_debug.ps1 -Action Screenshots
    .\ps5_debug.ps1 -Action Worlds
    .\ps5_debug.ps1 -Action DeployConfig
    .\ps5_debug.ps1 -Action DeployAll
#>
param(
    [ValidateSet("Logs","Tail","Screenshots","Worlds","DeployConfig","DeployExe","DeployAll")]
    [string]$Action = "Tail",
    [string]$Ps5Host = "192.168.1.170",
    [int]$FtpPort = 2121,
    [string]$TrinityRoot = (Split-Path $PSScriptRoot -Parent),
    [string]$OutTag = "live",
    [int]$TailLines = 80
)

$ErrorActionPreference = "Stop"
$baseUrl = "ftp://${Ps5Host}:${FtpPort}"
$proposal = $TrinityRoot
$xclLogRemote = "$baseUrl/data/prospero-win/prefix/drive_c/users/prospero/AppData/Local/XCL/Microsoft.MinecraftBeta/logs/xcl.log"
$xclTomlRemote = "$baseUrl/data/prospero-win/prefix/drive_c/users/prospero/AppData/Local/XCL/Microsoft.MinecraftBeta/xcl.toml"
$xclExeRemote = "$baseUrl/data/prospero-win/prefix/drive_c/XCL/xcl.exe"
$worldsRemote = "$baseUrl/data/prospero-win/prefix/drive_c/users/prospero/AppData/Local/XCL/Microsoft.MinecraftBeta/pls/Users/2533274790395905/games/com.mojang/minecraftWorlds/"
$photoRemote = "$baseUrl/user/av_contents/photo/NPXS40087/PPSA99995/"

function Invoke-Curl([string[]]$CurlArgs) {
    & curl.exe -s --show-error --disable-epsv --connect-timeout 8 --max-time 45 @CurlArgs
    if ($LASTEXITCODE -ne 0) { throw "curl.exe failed ($LASTEXITCODE)" }
}

switch ($Action) {
    "Tail" {
        $liveDir = Join-Path $TrinityRoot "ps5-logs\minecraft\live"
        New-Item -ItemType Directory -Force -Path $liveDir | Out-Null
        $tmp = Join-Path $liveDir "xcl.log"
        Invoke-Curl @($xclLogRemote, "-o", $tmp)
        Get-Content $tmp | Where-Object {
            $_ -match "\[SW_PROBE|\[INPUT\]|\[NET\]|\[VFS_TRACE\]|\[ASYNC\]|XUser|GameInput|XNetworking|WASAPI|XAudio2|acphal|xapu|WARN|ERROR|STUB|debug:" -and
            $_ -notmatch "unmapped console path"
        } | Select-Object -Last $TailLines
    }
    "Logs" {
        $outDir = Join-Path $proposal "ps5-logs\minecraft\$OutTag"
        New-Item -ItemType Directory -Force -Path $outDir | Out-Null
        Invoke-Curl @($xclLogRemote, "-o", (Join-Path $outDir "xcl.log"))
        $next = (Invoke-Curl @("$baseUrl/data/prospero-win/logs/next.txt")).Trim()
        if ($next -match '^\d+$') {
            $cur = ([int]$next + 7) % 8
            Invoke-Curl @("$baseUrl/data/prospero-win/logs/session-$cur.log", "-o", (Join-Path $outDir "session-$cur.log"))
        }
        Get-ChildItem $outDir | Select-Object Name, Length, LastWriteTime
    }
    "Screenshots" {
        Add-Type -AssemblyName PresentationCore
        $outDir = Join-Path $proposal "ps5-logs\minecraft\screenshots"
        New-Item -ItemType Directory -Force -Path $outDir | Out-Null
        $list = Invoke-Curl @($photoRemote)
        $dirs = ($list -split "`r?`n") | Where-Object { $_ -match '^d' -and $_ -notmatch '\s\.\.?$' } | ForEach-Object { ($_ -split '\s+', 9)[8].Trim() }
        foreach ($sub in $dirs) {
            $subList = Invoke-Curl @("$photoRemote$sub/")
            foreach ($ln in ($subList -split "`r?`n" | Where-Object { $_ -match '\.jxr$' })) {
                $fn = ($ln -split '\s+', 9)[8].Trim()
                $dstJxr = Join-Path $outDir "${sub}_$fn"
                $dstPng = [System.IO.Path]::ChangeExtension($dstJxr, ".png")
                if (-not (Test-Path $dstPng)) {
                    Invoke-Curl @("$photoRemote$sub/$([uri]::EscapeDataString($fn))", "-o", $dstJxr)
                    $dec = [System.Windows.Media.Imaging.BitmapDecoder]::Create((New-Object System.Uri($dstJxr)), [System.Windows.Media.Imaging.BitmapCreateOptions]::PreservePixelFormat, [System.Windows.Media.Imaging.BitmapCacheOption]::OnLoad)
                    $conv = New-Object System.Windows.Media.Imaging.FormatConvertedBitmap($dec.Frames[0], [System.Windows.Media.PixelFormats]::Bgra32, $null, 0)
                    $enc = New-Object System.Windows.Media.Imaging.PngBitmapEncoder
                    $enc.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($conv))
                    $fs = [System.IO.File]::Create($dstPng); $enc.Save($fs); $fs.Close()
                    Write-Host "Converted: $dstPng"
                }
            }
        }
        Get-ChildItem $outDir -Filter "*.png" | Sort-Object LastWriteTime -Descending | Select-Object -First 10 Name, Length, LastWriteTime
    }
    "Worlds" {
        Invoke-Curl @($worldsRemote)
    }
    "DeployConfig" {
        Invoke-Curl @("-T", (Join-Path $proposal "configs\minecraft.xcl.toml"), $xclTomlRemote)
        Write-Host "Deployed xcl.toml OK"
    }
    "DeployExe" {
        & python -I (Join-Path $TrinityRoot "tools\ps5_deploy_runtime.py") $Ps5Host --port $FtpPort --binary (Join-Path $TrinityRoot "dist\prospero-win\prefix\drive_c\XCL\xcl.exe")
        if ($LASTEXITCODE -ne 0) { throw "XCL deployment failed ($LASTEXITCODE)" }
        Write-Host "Deployed xcl.exe OK"
    }
    "DeployAll" {
        & python -I (Join-Path $TrinityRoot "tools\ps5_deploy_runtime.py") $Ps5Host --port $FtpPort --binary (Join-Path $TrinityRoot "dist\prospero-win\prefix\drive_c\XCL\xcl.exe")
        if ($LASTEXITCODE -ne 0) { throw "XCL deployment failed ($LASTEXITCODE)" }
        Invoke-Curl @("-T", (Join-Path $proposal "configs\minecraft.xcl.toml"), $xclTomlRemote)
        Write-Host "Deployed xcl.exe + xcl.toml OK"
    }
}
