# SPDX-License-Identifier: MIT
[CmdletBinding()]
param(
    [string]$Foundation = '',
    [Parameter(Mandatory)][string]$Sdk,
    [Parameter(Mandatory)][string]$LlvmBin,
    [Parameter(Mandatory)][string]$NativeTool,
    [Parameter(Mandatory)][string]$Output,
    [switch]$WithSeed
)

$ErrorActionPreference = 'Stop'
$include = Join-Path (Resolve-Path $Sdk).Path 'target\include'
$libs = Join-Path (Resolve-Path $Sdk).Path 'target\lib'
$clang = Join-Path (Resolve-Path $LlvmBin).Path 'clang.exe'
$lld = Join-Path (Resolve-Path $LlvmBin).Path 'ld.lld.exe'
$tool = (Resolve-Path $NativeTool).Path
$destination = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $destination) { throw "Output already exists: $destination" }
$directory = New-Item -Type Directory -Force (Split-Path -Parent $destination)
$scratch = New-Item -Type Directory (Join-Path $directory.FullName ('launcher-' + [guid]::NewGuid().ToString('N')))

function Invoke-Tool([string]$Executable, [string[]]$Arguments) {
    & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Executable exited with status $LASTEXITCODE" }
}

$flags = @('-target', 'x86_64-sie-ps5', '-fvisibility-nodllstorageclass=default', '-isystem',
    $include, '-fno-stack-protector', '-fno-plt', '-femulated-tls',
    '-O2', '-Wall', '-Wextra', '-Werror', '-ffunction-sections', '-fdata-sections')
$crt = Join-Path $scratch.FullName 'crt0.o'
$entry = Join-Path $scratch.FullName 'launcher.o'
$seed = Join-Path $scratch.FullName 'prefix_seed.o'
$pie = Join-Path $scratch.FullName 'launcher-pie.elf'
$elf = Join-Path $scratch.FullName 'launcher-sce.elf'
$pending = Join-Path $scratch.FullName 'eboot.bin'
$objects = @($crt, $entry)
$defines = @()
if ($WithSeed) {
    $defines += '-DTRINITYXSX_WITH_SEED'
    Invoke-Tool $clang ($flags + @('-std=c11', '-c', "$PSScriptRoot\prefix_seed.c", '-o', $seed))
    $objects += $seed
}
Invoke-Tool $clang ($flags + @('-std=c11', '-c', "$PSScriptRoot\crt0.c", '-o', $crt))
Invoke-Tool $clang ($flags + @('-std=c11') + $defines + @('-c', "$PSScriptRoot\launcher.c", '-o', $entry))

# Link the application stubs explicitly. The SDK also has Web and system variants
# which export overlapping names with different providers.
$stubDirectory = New-Item -ItemType Directory (Join-Path $scratch.FullName 'stubs')
$stubs = foreach ($name in @('libkernel.so', 'libSceLibcInternal.so', 'libSceSystemService.so')) {
    Copy-Item -LiteralPath (Join-Path $libs $name) -Destination $stubDirectory.FullName
    Join-Path $stubDirectory.FullName $name
}
Invoke-Tool $lld (@('-m', 'elf_x86_64', '-pie', '-T', "$PSScriptRoot\launcher.ld", '--eh-frame-hdr',
    '-z', 'max-page-size=0x4000', '-mllvm',
    '-emulated-tls', '--hash-style=gnu', '-e', '_start', '-o', $pie) + $objects + @('--as-needed') + $stubs)
Invoke-Tool $tool @('link', '--in', $pie, '--out', $elf, '--stub-dir', $stubDirectory.FullName, '--module-sdk',
    '0x02000009', '--companion-sdk', '0x08050001', '--file-name', 'eboot.elf')
Invoke-Tool $tool @('self', '--sign', '--in', $elf, '--out', $pending, '--magic', '0x1D3D154F')
Invoke-Tool $tool @('self', '--inspect', '--file', $pending)
Move-Item -LiteralPath $pending -Destination $destination
Get-FileHash -LiteralPath $destination -Algorithm SHA256
