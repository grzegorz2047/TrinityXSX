#!/usr/bin/env bash
# Download DXVK (dxgi/d3d11) and DXC (dxcompiler/dxil) releases into tools/, verified against
# the SHA-256 in pins.env. Needs curl, tar and unzip.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=../pins.env
. "$root/pins.env"
get() { # url file sha256
  [ -f "$2" ] || curl -fsSL -o "$2" "$1"
  echo "$3  $2" | sha256sum -c - >/dev/null || { echo "checksum mismatch: $2" >&2; rm -f "$2"; exit 1; }
}

mkdir -p "$root/tools/dxvk" "$root/tools/dxc/x"
cd "$root/tools/dxvk"
get "https://github.com/doitsujin/dxvk/releases/download/v$DXVK_VERSION/dxvk-$DXVK_VERSION.tar.gz" "dxvk-$DXVK_VERSION.tar.gz" "$DXVK_SHA256"
tar -xzf "dxvk-$DXVK_VERSION.tar.gz"
[ -f LICENSE ] || curl -fsSL -o LICENSE "https://raw.githubusercontent.com/doitsujin/dxvk/v$DXVK_VERSION/LICENSE"

cd "$root/tools/dxc"
get "https://github.com/microsoft/DirectXShaderCompiler/releases/download/$DXC_TAG/$DXC_ZIP" "$DXC_ZIP" "$DXC_SHA256"
# the zip names its files with backslashes: unzip turns them into folders and exits with 1 (a warning)
(cd x && { unzip -q -o "../$DXC_ZIP" 2>/dev/null || [ $? -eq 1 ]; })
ls "$root/tools/dxvk/dxvk-$DXVK_VERSION/x64" "$root/tools/dxc/x/bin/x64"
