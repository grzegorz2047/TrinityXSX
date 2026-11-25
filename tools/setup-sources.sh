#!/usr/bin/env bash
# Fetch XCL and vkd3d-proton at the revisions in pins.env and apply patches/ on top (git am).
#   tools/setup-sources.sh [--only xcl|vkd3d] [DEST]     (DEST defaults to the repo root)
# Creates DEST/xcl and DEST/vkd3d-proton; an existing folder is left alone.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=../pins.env
. "$root/pins.env"
only=
if [ "${1:-}" = --only ]; then only=$2; shift 2; fi
dest=${1:-$root}
export GIT_COMMITTER_NAME=${GIT_COMMITTER_NAME:-TrinityXSX} GIT_COMMITTER_EMAIL=${GIT_COMMITTER_EMAIL:-trinityxsx@localhost}

fetch() { # url rev dir
  if [ -e "$3" ]; then echo "$3 exists, left alone"; return 1; fi
  git init -q "$3"
  git -C "$3" fetch -q --depth 1 "$1" "$2"
  git -C "$3" checkout -q FETCH_HEAD
}

if [ -z "$only" ] || [ "$only" = xcl ]; then
  if fetch "$XCL_REPO" "$XCL_REV" "$dest/xcl"; then
    git -C "$dest/xcl" am -q "$root"/patches/xcl/*.patch
    git -C "$dest/xcl" log --oneline -n "$(ls "$root"/patches/xcl/*.patch | wc -l)"
  fi
fi

if [ -z "$only" ] || [ "$only" = vkd3d ]; then
  if fetch "$VKD3D_REPO" "$VKD3D_REV" "$dest/vkd3d-proton"; then
    git -C "$dest/vkd3d-proton" submodule update -q --init --recursive --depth 1
    [ "$(git -C "$dest/vkd3d-proton/subprojects/dxil-spirv" rev-parse HEAD)" = "$DXIL_SPIRV_REV" ] ||
      { echo "dxil-spirv is not at $DXIL_SPIRV_REV" >&2; exit 1; }
    git -C "$dest/vkd3d-proton" am -q "$root"/patches/vkd3d-proton/*.patch
    if compgen -G "$root/patches/dxil-spirv/*.patch" >/dev/null; then
      git -C "$dest/vkd3d-proton/subprojects/dxil-spirv" am -q "$root"/patches/dxil-spirv/*.patch
      git -C "$dest/vkd3d-proton/subprojects/dxil-spirv" log --oneline -n "$(ls "$root"/patches/dxil-spirv/*.patch | wc -l)"
    fi
    git -C "$dest/vkd3d-proton" log --oneline -n "$(ls "$root"/patches/vkd3d-proton/*.patch | wc -l)"
  fi
fi
