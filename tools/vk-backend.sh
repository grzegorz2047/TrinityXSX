#!/usr/bin/env bash
# Put vkd3d-proton (d3d12.dll, d3d12core.dll) and DXVK (dxgi.dll, d3d11.dll) next to every XCL
# test binary and xcl.exe, so `cargo test` runs on Vulkan; `off` removes them again.
#   tools/vk-backend.sh on [dbg|ndebug]   (default dbg: symbols for gdb/addr2line)
#   tools/vk-backend.sh off
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
. "$root/pins.env"
target=$root/xcl/target/debug
mode=${1:-}
flavour=${2:-dbg}
dirs() { echo "$target"; find "$target/build" -maxdepth 3 -type d -name out 2>/dev/null; }
case "$mode" in
  on)
    vk=$root/vkd3d-proton/build-$flavour/libs
    dx=$root/tools/dxvk/dxvk-$DXVK_VERSION/x64
    n=0
    for d in $(dirs); do
      ls "$d"/*.exe >/dev/null 2>&1 || continue
      cp "$vk/d3d12/d3d12.dll" "$vk/d3d12core/d3d12core.dll" "$dx/dxgi.dll" "$dx/d3d11.dll" "$d/"
      n=$((n + 1))
    done
    echo "vulkan backend ($flavour) in $n dirs"
    ;;
  off)
    for d in $(dirs); do rm -f "$d"/d3d12.dll "$d"/d3d12core.dll "$d"/dxgi.dll "$d"/d3d11.dll; done
    echo "windows D3D12 backend"
    ;;
  *) echo "usage: $0 on [dbg|ndebug] | off" >&2; exit 2 ;;
esac
