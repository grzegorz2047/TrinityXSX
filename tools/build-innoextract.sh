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

# 1. Hook progress::update() in src/util/console.cpp to report byte progress without touching stdout
python3 - "$src" "$root/tools/innoextract/trinity_inno_api.cpp" <<'PY'
import pathlib, shutil, sys

src = pathlib.Path(sys.argv[1])
api_src = pathlib.Path(sys.argv[2])
shutil.copy2(api_src, src / "src" / "cli" / "trinity_inno_api.cpp")

console_cpp = src / "src" / "util" / "console.cpp"
txt = console_cpp.read_text(encoding="utf-8")
hook_decl = 'extern "C" void trinity_inno_notify_progress(boost::uint64_t done_bytes, boost::uint64_t total_bytes);\n'
if "trinity_inno_notify_progress" not in txt:
    txt = hook_decl + txt
needle = "\tvalue += delta;\n"
assert needle in txt, "progress::update value += delta not found"
replacement = (
    "\tvalue += delta;\n"
    "\ttrinity_inno_notify_progress(value, max);\n"
    "\treturn true;\n"
)
txt = txt.replace(needle, replacement, 1)
console_cpp.write_text(txt, encoding="utf-8")

cmake = src / "CMakeLists.txt"
ct = cmake.read_text(encoding="utf-8")
ct = ct.replace("src/cli/main.cpp", "src/cli/trinity_inno_api.cpp")
old_target = "add_executable(innoextract ${INNOEXTRACT_SOURCES})\ntarget_link_libraries(innoextract ${LIBRARIES})"
new_target = (
    "add_library(innoextract SHARED ${INNOEXTRACT_SOURCES})\n"
    "set_target_properties(innoextract PROPERTIES PREFIX \"\" OUTPUT_NAME \"trinity_innoextract\")\n"
    "target_link_libraries(innoextract ${LIBRARIES} -static -static-libgcc -static-libstdc++ -Wl,-Bstatic -lwinpthread -Wl,-Bdynamic)"
)
assert old_target in ct, "innoextract CMake target not found"
ct = ct.replace(old_target, new_target, 1)
cmake.write_text(ct, encoding="utf-8")
print("Patched innoextract CMakeLists.txt & console.cpp for static trinity_innoextract.dll")
PY

cmake -S "$src" -B "$build" -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_FIND_LIBRARY_SUFFIXES=".a" \
  -DUSE_STATIC_LIBS=ON \
  -DLZMA_USE_STATIC_LIBS=ON \
  -DBoost_USE_STATIC_LIBS=ON \
  -Diconv_USE_STATIC_LIBS=ON \
  -DWITH_CONV=win32 \
  -DBUILD_TESTS=OFF \
  -DSET_WARNING_FLAGS=OFF

cmake --build "$build" --target innoextract -j "$(nproc)"

mkdir -p "$out"
cp "$build/trinity_innoextract.dll" "$out/"
cp "$src/LICENSE" "$out/innoextract-zlib.txt"
strip --strip-unneeded "$out/trinity_innoextract.dll" || true
ls -lh "$out/"
objdump -p "$out/trinity_innoextract.dll" | grep "DLL Name:" || true
# Fail fast if any non-Windows MSYS2 DLL leaked into imports
if objdump -p "$out/trinity_innoextract.dll" | grep -Ei "DLL Name:.*(libgcc|libstdc\+\+|libwinpthread|libboost|liblzma|libbz2|libz|libiconv)"; then
  echo "ERROR: trinity_innoextract.dll has non-static MSYS2 DLL dependency!" >&2
  exit 1
fi
