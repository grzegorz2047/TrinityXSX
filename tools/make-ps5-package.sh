#!/usr/bin/env bash
# Assemble XCL + its test title for prospero-win (Wine on the PS5), laid out as
# /data/prospero-win on the console:
#   dist/prospero-win/prefix/drive_c/XCL/     xcl.exe, vkd3d-proton, DXVK, DXC, testtitle/
#   dist/prospero-win/profiles/xcl-testtitle.profile
#   dist/prospero-win/profiles/profiles.lst   (only used when the console has none yet)
# "prefix = default" is /data/prospero-win/prefix (native/wine64_main.c of prospero-win).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
. "$root/pins.env"
xcl=$root/xcl
rel=$xcl/target/release
vk=$root/vkd3d-proton/${VKD3D_BUILD:-build-ndebug}/libs
dxvk=$root/tools/dxvk/dxvk-$DXVK_VERSION/x64
dxc=$root/tools/dxc/x/bin/x64
out=$root/dist/prospero-win
dir=$out/prefix/drive_c/XCL

for f in "$rel/xcl.exe" "$rel/xcl_testtitle.exe" "$rel/testgame_mw.dll" "$vk/d3d12/d3d12.dll" "$vk/d3d12core/d3d12core.dll" \
         "$dxvk/dxgi.dll" "$dxvk/d3d11.dll" "$dxc/dxcompiler.dll" "$dxc/dxil.dll"; do
  [ -f "$f" ] || { echo "missing $f" >&2; exit 1; }
done
# the release xcl.exe must contain the --in-process mode
grep -a -q -- '--in-process' "$rel/xcl.exe" || { echo "xcl.exe has no --in-process: build ps5/test-package" >&2; exit 1; }

mkdir -p "$dir/testtitle" "$dir/licenses" "$out/profiles"
cp "$rel/xcl.exe" "$vk/d3d12/d3d12.dll" "$vk/d3d12core/d3d12core.dll" "$dxvk/dxgi.dll" "$dxvk/d3d11.dll" \
   "$dxc/dxcompiler.dll" "$dxc/dxil.dll" "$dir/"
cp -r "$xcl/tests/data/testgame/." "$dir/testtitle/"
cp "$rel/xcl_testtitle.exe" "$rel/testgame_mw.dll" "$dir/testtitle/"

cp "$xcl/LICENSE" "$dir/licenses/XCL-GPL-2.0.txt"
cp "$root/vkd3d-proton/LICENSE" "$dir/licenses/vkd3d-proton-LGPL-2.1.txt"
cp "$root/vkd3d-proton/COPYING" "$dir/licenses/vkd3d-proton-COPYING.txt"
cp "$root/vkd3d-proton/subprojects/dxil-spirv/LICENSE.MIT" "$dir/licenses/dxil-spirv-MIT.txt"
cp "$root/tools/dxvk/LICENSE" "$dir/licenses/dxvk-zlib.txt"
cp "$root/tools/dxc/x/LICENSE-LLVM.txt" "$root/tools/dxc/x/LICENSE-MS.txt" "$dir/licenses/"

cat > "$out/profiles/xcl-testtitle.profile" <<'EOF'
; XCL (Xbox Series X|S compatibility layer) running its own test title.
; Wine on the console runs one process, so XCL runs the title in its own
; process (xcl run --in-process). D3D12 is vkd3d-proton with XCL's patches
; (C:\XCL\d3d12*.dll), dxgi/d3d11 are DXVK 3.1.1, all native from C:\XCL.
; The title prints [PASS]/[FAIL] lines to stdout: STDOUT records of the
; session log. XCL's own log: drive_c/users/prospero/AppData/Local/XCL/
; XCL.TestTitle/logs/xcl.log (and report.json) in this prefix.
[application]
id = xcl-testtitle
name = XCL test title
executable = C:\XCL\xcl.exe
working_directory = C:\XCL
arguments = run --in-process testtitle -- --hello
dll_overrides = d3d12,d3d12core,dxgi,d3d11=n
prefix = default
runtime = wine-wow64
architecture = pe64
graphics = dxvk

[display]
desktop = 1920x1080

[input]
mode = xinput

[debug]
winedebug = err+all,+loaddll
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d.log
env = DXVK_LOG_PATH=C:/XCL
EOF
printf 'xcl-testtitle.profile\n' > "$out/profiles/profiles.lst"

# a checkout's branch @ commit; without one (CI: built elsewhere) the pinned revision + patches
rev() { # dir pinned-rev
  local b c
  if [ -e "$1/.git" ] && c=$(git -C "$1" rev-parse --short HEAD 2>/dev/null); then
    b=$(git -C "$1" branch --show-current 2>/dev/null)
    echo "${b:-detached} @ $c"
  else
    echo "${2:0:7} + patches/ of TrinityXSX @ $(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo '?')"
  fi
}
cat > "$dir/VERSION.txt" <<EOF
XCL test package for prospero-win
XCL          $(rev "$xcl" "$XCL_REV") (release build)
vkd3d-proton $(rev "$root/vkd3d-proton" "$VKD3D_REV"), dxil-spirv $(rev "$root/vkd3d-proton/subprojects/dxil-spirv" "$DXIL_SPIRV_REV") (meson release, b_ndebug=true)
DXVK         $DXVK_VERSION dxgi.dll + d3d11.dll (upstream release)
DXC          $DXC_TAG dxcompiler.dll + dxil.dll (official GitHub release)
EOF
du -sh "$out"; find "$out" -type f | sed "s|$out/||" | sort
