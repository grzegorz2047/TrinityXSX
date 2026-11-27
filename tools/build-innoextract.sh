#!/usr/bin/env bash
# Build self-contained static-deps trinity_innoextract.dll (MSYS2 UCRT64) from dscharrer/innoextract.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=../pins.env
. "$root/pins.env"

src="$root/innoextract-src"
build="$src/build-dll"
out="$root/out/innoextract"

rm -rf "$src" "$out"
git init -q "$src"
git -C "$src" fetch -q --depth 1 "$INNOEXTRACT_REPO" "$INNOEXTRACT_REV"
git -C "$src" checkout -q FETCH_HEAD

# Apply TrinityXSX DLL, 4 MiB Win32 buffered I/O, and parallel zlib patches
python3 "$root/tools/innoextract/patch_innoextract.py" "$src" "$root/tools/innoextract"

cmake -S "$src" -B "$build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_CXX_FLAGS_RELEASE="-O3 -DNDEBUG" \
  -DCMAKE_FIND_LIBRARY_SUFFIXES=".a" \
  -DUSE_LTO=OFF \
  -DUSE_STATIC_LIBS=ON \
  -DLZMA_USE_STATIC_LIBS=ON \
  -DZLIB_USE_STATIC_LIBS=ON \
  -DBZip2_USE_STATIC_LIBS=ON \
  -DBoost_USE_STATIC_LIBS=ON \
  -Diconv_USE_STATIC_LIBS=ON \
  -DWITH_CONV=win32 \
  -DBUILD_TESTS=OFF \
  -DSET_WARNING_FLAGS=OFF

# Boost's MSYS2 CMake targets hardcode .dll.a for transitive compression libs (bz2/lzma/zstd).
# Rewrite them to their sibling static .a archives so trinity_innoextract.dll has zero MSYS2 DLL deps.
python3 - "$build/build.ninja" <<'PY'
import pathlib, re, sys
ninja_path = pathlib.Path(sys.argv[1])
txt = ninja_path.read_text(encoding="utf-8")
replaced = set()
def repl(m):
    full = m.group(0)
    static_p = full[:-6] + ".a"
    if pathlib.Path(static_p).is_file():
        replaced.add((full, static_p))
        return static_p
    print(f"WARNING: static archive missing for {full}: {static_p}", file=sys.stderr)
    return full
txt = re.sub(r'[^\s"]+lib(?:bz2|lzma|zstd|z|iconv|winpthread)\.dll\.a', repl, txt)
ninja_path.write_text(txt, encoding="utf-8")
for old, new in sorted(replaced):
    print(f"Static link override: {old} -> {new}")
PY

cmake --build "$build" --target innoextract -j "$(nproc)"

mkdir -p "$out"
cp "$build/trinity_innoextract.dll" "$out/"
cp "$src/LICENSE" "$out/innoextract-zlib.txt"
strip --strip-unneeded "$out/trinity_innoextract.dll" || true
ls -lh "$out/"
objdump -p "$out/trinity_innoextract.dll" | grep "DLL Name:" || true
# Fail fast if any non-Windows MSYS2 DLL leaked into imports
if objdump -p "$out/trinity_innoextract.dll" | grep -Ei "DLL Name:.*(libgcc|libstdc\+\+|libwinpthread|libboost|liblzma|libbz2|libz|libzstd|libiconv)"; then
  echo "ERROR: trinity_innoextract.dll has non-static MSYS2 DLL dependency!" >&2
  exit 1
fi
