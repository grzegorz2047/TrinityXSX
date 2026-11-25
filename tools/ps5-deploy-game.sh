#!/usr/bin/env bash
# Put an unpacked Xbox Series (GDK) game folder you own on the PS5 for XCL under prospero-win:
#   - checks the folder (MicrosoftGame.config, an executable for "Scarlett", console imports),
#   - uploads it to C:\XCL\games\<id> of the default prefix (/data/prospero-win/prefix),
#   - writes profiles/<id>.profile (xcl run --in-process) and adds it to profiles.lst,
#   - with --reset-data, empties XCL's overlay of that title (its writes from earlier runs).
# XCL itself (C:\XCL: xcl.exe, vkd3d-proton, DXVK, DXC) comes from tools/make-ps5-package.sh;
# --with-runtime uploads dist/prospero-win first.
#
#   tools/ps5-deploy-game.sh <game folder> <id> <PS5 address> [--port 2121] [--name "Shown name"]
#                            [--args "game arguments"] [--with-runtime] [--reset-data] [--dry-run]
# <id>: lower-case letters, digits, '-' and '_' (prospero-win's profile id rule).
# Under Git Bash: remote paths go out as they are (MSYS_NO_PATHCONV=1), local ones as Windows paths.
set -euo pipefail
export MSYS_NO_PATHCONV=1
root=$(cd "$(dirname "$0")/.." && pwd)
# a local path for a native program (Windows python / curl under Git Bash); as it is elsewhere
local_path() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s
' "$1"; fi; }

usage() { sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 2; }
[ $# -ge 3 ] || usage
game=$1 id=$2 host=$3; shift 3
port=2121 name= game_args= with_runtime= reset= dry=
while [ $# -gt 0 ]; do
  case "$1" in
    --port) port=$2; shift 2 ;;
    --name) name=$2; shift 2 ;;
    --args) game_args=$2; shift 2 ;;
    --with-runtime) with_runtime=1; shift ;;
    --reset-data) reset=1; shift ;;
    --dry-run) dry=1; shift ;;
    *) usage ;;
  esac
done

fail() { echo "ps5-deploy-game: $*" >&2; exit 1; }
[[ $id =~ ^[a-z0-9][a-z0-9_-]*$ ]] || fail "id '$id': lower-case letters, digits, '-' and '_' only"
[ -d "$game" ] || fail "no folder $game"
cfg=$(find "$game" -maxdepth 1 -iname MicrosoftGame.config | head -1)
[ -n "$cfg" ] || fail "$game has no MicrosoftGame.config: not an unpacked GDK game folder"

# the manifest: identity (XCL's data folder), executable, target device
identity=$(grep -o -E '<Identity[^>]* Name="[^"]+"' "$cfg" | sed -E 's/.* Name="([^"]+)"/\1/' | head -1)
exe=$(grep -o -E '<Executable[^>]* Name="[^"]+"' "$cfg" | sed -E 's/.* Name="([^"]+)"/\1/' | head -1)
family=$(grep -o -E 'TargetDeviceFamily="[^"]+"' "$cfg" | sed -E 's/.*="([^"]+)"/\1/' | sort -u | tr '\n' ' ')
[ -n "$exe" ] && [ -f "$game/$exe" ] || fail "the executable the manifest names ($exe) is not in $game"
case "$family" in
  *Scarlett*) ;;
  *) fail "TargetDeviceFamily is '$family': XCL needs the Xbox Series (Scarlett) build – its shaders are RDNA2" ;;
esac
[ -n "$name" ] || name=$(grep -o -E 'DefaultDisplayName="[^"]+"' "$cfg" | sed -E 's/.*="([^"]+)"/\1/' | head -1)
[ -n "$name" ] || name=$identity

# a console executable imports console libraries; XCL's own check says so
xcl_exe=$root/xcl/target/release/xcl.exe
if [ -x "$xcl_exe" ]; then
  if ! "$xcl_exe" inspect "$game/$exe" 2>/dev/null | grep -q -i -E 'kernelx|xgameruntime|d3d12_x'; then
    fail "$exe imports no console library (xcl inspect): a PC build, not the console one"
  fi
fi

size=$(du -sh "$game" | cut -f1)
remote_game="/data/prospero-win/prefix/drive_c/XCL/games/$id"
win_game="C:\\XCL\\games\\$id"
echo "game:     $name ($identity), $exe, $family, $size"
echo "to:       ftp://$host:$port$remote_game"
echo "profile:  /data/prospero-win/profiles/$id.profile"

profile=$(mktemp)
trap 'rm -f "$profile" "$profile.lst"' EXIT
cat > "$profile" <<EOF
; $name – XCL (Xbox Series compatibility layer) under prospero-win, made by
; TrinityXSX tools/ps5-deploy-game.sh. The game runs in XCL's process
; (xcl run --in-process); XCL's log: drive_c/users/prospero/AppData/Local/XCL/
; $identity/logs/xcl.log in this prefix; the session log has the game's output.
[application]
id = $id
name = $name
executable = C:\\XCL\\xcl.exe
working_directory = C:\\XCL
arguments = run --in-process $win_game${game_args:+ -- $game_args}
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
winedebug = err+all
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d.log
EOF
if [ -n "$dry" ]; then echo "--- profile (dry run, nothing uploaded)"; cat "$profile"; exit 0; fi

ftp=(python -I "$(local_path "$root/tools/ftp_put_tree.py")" "$host" "$port")
url="ftp://$host:$port"
if [ -n "$with_runtime" ]; then
  [ -d "$root/dist/prospero-win" ] || fail "no dist/prospero-win: run tools/make-ps5-package.sh first"
  "${ftp[@]}" "$(local_path "$root/dist/prospero-win/prefix")" /data/prospero-win/prefix
fi
# (a listing: this FTP server takes no byte ranges)
curl -s --disable-epsv "$url/data/prospero-win/prefix/drive_c/XCL/" | awk '{print $NF}' | grep -qx 'xcl.exe' ||
  fail "no C:\\XCL\\xcl.exe on the console: deploy the runtime with --with-runtime"

"${ftp[@]}" "$(local_path "$game")" "$remote_game"
curl -s --disable-epsv -T "$(local_path "$profile")" "$url/data/prospero-win/profiles/$id.profile"
# the launcher's list: one profile file name per line, kept as it is with ours added once
curl -s --disable-epsv "$url/data/prospero-win/profiles/profiles.lst" > "$profile.lst" || true
grep -qx "$id.profile" "$profile.lst" || printf '%s.profile\n' "$id" >> "$profile.lst"
curl -s --disable-epsv -T "$(local_path "$profile.lst")" "$url/data/prospero-win/profiles/profiles.lst"

if [ -n "$reset" ]; then
  overlay="/data/prospero-win/prefix/drive_c/users/prospero/AppData/Local/XCL/$identity/overlay"
  if curl -s --disable-epsv "$url$overlay/" >/dev/null 2>&1; then
    python -I "$(local_path "$root/tools/ftp_rm_tree.py")" "$host" "$port" "$overlay"
  fi
fi
echo "done: start \"$name\" in prospero-win"
