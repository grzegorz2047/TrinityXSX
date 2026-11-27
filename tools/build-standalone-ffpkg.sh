#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
# Build a complete, fsck_ufs-verified standalone TrinityXSX PPSA83601.ffpkg image on Linux
# (Ubuntu 24.04 / GitHub Actions) from pinned upstream sources in pins.env.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
# shellcheck source=../pins.env
. "$root/pins.env"

xcl_runtime=""
library_exe=""
output=""
zip_output=""
content_version="01.000.016"
work="$root/.work/standalone-ffpkg"
wine_cache="$root/.work/wine-cache"
llvm_bin=""
jobs=$(nproc 2>/dev/null || echo 4)

while [ $# -gt 0 ]; do
  case "$1" in
    --xcl-runtime) xcl_runtime=$2; shift 2 ;;
    --library-exe) library_exe=$2; shift 2 ;;
    --output) output=$2; shift 2 ;;
    --zip-output) zip_output=$2; shift 2 ;;
    --content-version) content_version=$2; shift 2 ;;
    --work) work=$2; shift 2 ;;
    --wine-cache) wine_cache=$2; shift 2 ;;
    --llvm-bin) llvm_bin=$2; shift 2 ;;
    --jobs) jobs=$2; shift 2 ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -n "$xcl_runtime" ] && [ -d "$xcl_runtime" ] || { echo "--xcl-runtime directory is required" >&2; exit 2; }
[ -n "$library_exe" ] && [ -f "$library_exe" ] || { echo "--library-exe file is required" >&2; exit 2; }
[ -n "$output" ] || { echo "--output path/to/PPSA83601.ffpkg is required" >&2; exit 2; }

if [ -z "$llvm_bin" ]; then
  for candidate in /usr/lib/llvm-18/bin /usr/lib/llvm-19/bin /usr/lib/llvm-20/bin /usr/bin; do
    if [ -x "$candidate/clang" ] && [ -x "$candidate/ld.lld" ]; then
      llvm_bin="$candidate"
      break
    fi
  done
fi
[ -n "$llvm_bin" ] && [ -x "$llvm_bin/clang" ] && [ -x "$llvm_bin/ld.lld" ] || {
  echo "clang + ld.lld not found (install clang-18 lld-18 or pass --llvm-bin)" >&2
  exit 1
}

export PATH="$llvm_bin:$PATH"
mkdir -p "$work" "$wine_cache"
work=$(cd "$work" && pwd)
wine_cache=$(cd "$wine_cache" && pwd)
xcl_runtime=$(cd "$xcl_runtime" && pwd)
library_exe=$(cd "$(dirname "$library_exe")" && pwd)/$(basename "$library_exe")

download_checked() {
  local url=$1 dest=$2 sha=$3
  if [ ! -f "$dest" ]; then
    echo "==> Downloading $(basename "$dest")"
    curl -fsSL --retry 4 --retry-delay 5 -o "$dest.part" "$url"
    mv "$dest.part" "$dest"
  fi
  echo "$sha  $dest" | sha256sum -c - >/dev/null || {
    echo "SHA-256 mismatch for $dest" >&2
    rm -f "$dest"
    exit 1
  }
}

fetch_commit() {
  local repo=$1 rev=$2 dir=$3
  if [ ! -d "$dir/.git" ]; then
    git init -q "$dir"
    git -C "$dir" remote add origin "$repo"
  fi
  if ! git -C "$dir" cat-file -e "$rev^{commit}" 2>/dev/null; then
    echo "==> Fetching $(basename "$dir") @ ${rev:0:12}"
    git -C "$dir" fetch -q --depth 1 origin "$rev"
  fi
  git -C "$dir" checkout -q --force "$rev"
  git -C "$dir" clean -q -fdx
}

# 1. Verified binary archives (PS5 Payload SDK & prospero-win v0.3.0 release)
sdk_zip="$work/ps5-payload-sdk-${PS5_SDK_TAG}.zip"
download_checked "https://github.com/ps5-payload-dev/sdk/releases/download/${PS5_SDK_TAG}/ps5-payload-sdk.zip" \
  "$sdk_zip" "$PS5_SDK_SHA256"

sdk="$work/ps5-payload-sdk"
if [ ! -x "$sdk/bin/prospero-clang" ]; then
  rm -rf "$sdk"
  unzip -q "$sdk_zip" -d "$work"
  chmod +x "$sdk"/bin/*
fi
export PS5_PAYLOAD_SDK="$sdk"

pw_zip="$work/prospero-win-${PROSPERO_WIN_TAG}.zip"
download_checked "https://github.com/mpereiraesaa/prospero-win/releases/download/${PROSPERO_WIN_TAG}/prospero-win-${PROSPERO_WIN_TAG}.zip" \
  "$pw_zip" "$PROSPERO_WIN_ZIP_SHA256"

# 2. Native app boilerplate (app CRT + ps5-native-tool) and PRX foundation (for wineserver.prx)
foundation="$work/foundation"
fetch_commit "$BOILERPLATE_REPO" "$BOILERPLATE_REV" "$foundation"

prx_foundation="$work/prx-foundation"
fetch_commit "$BOILERPLATE_REPO" "$BOILERPLATE_PRX_REV" "$prx_foundation"

mkdir -p "$prx_foundation/build/host"
native_tool="$prx_foundation/build/host/ps5-native-tool"
if [ ! -x "$native_tool" ]; then
  echo "==> Building ps5-native-tool"
  "$llvm_bin/clang++" -std=c++20 -O2 \
    "$prx_foundation/tooling/native/native_app_builder.cpp" \
    "$prx_foundation/tooling/native/self_container.cpp" \
    "$prx_foundation/tooling/native/elf_object.cpp" \
    "$prx_foundation/tooling/native/sce_module_writer.cpp" \
    -lz -o "$native_tool"
fi

# 3. UFS2Tool (.NET 8)
ufs2_src="$work/UFS2Tool"
ufs2_dll="$ufs2_src/bin/Release/net8.0/UFS2Tool.dll"
if [ ! -f "$ufs2_dll" ]; then
  fetch_commit "$UFS2TOOL_REPO" "$UFS2TOOL_REV" "$ufs2_src"
  echo "==> Building UFS2Tool"
  dotnet build "$ufs2_src/UFS2Tool.csproj" -c Release -v minimal
fi

# 4. Patched prospero-win source (HEAD stays at PROSPERO_WIN_REV for build_runner.py verification)
pw_src="$work/prospero-win"
fetch_commit "$PROSPERO_WIN_REPO" "$PROSPERO_WIN_REV" "$pw_src"
echo "==> Applying patches/prospero-win"
for p in "$root"/patches/prospero-win/*.patch; do
  echo "    $(basename "$p")"
  git -C "$pw_src" apply --whitespace=nowarn "$p"
done

# 5. Cross-compiled opus.dll (BSD-3-Clause, cached in wine_cache)
runtime_stage="$work/xcl-runtime-stage"
rm -rf "$runtime_stage"
cp -a "$xcl_runtime" "$runtime_stage"
if [ ! -f "$runtime_stage/opus.dll" ]; then
  cached_opus="$wine_cache/opus/opus.dll"
  cached_opus_lic="$wine_cache/opus/opus-BSD-3-Clause.txt"
  if [ ! -f "$cached_opus" ]; then
    echo "==> Building Windows opus.dll (libopus $OPUS_VERSION)"
    opus_tar="$work/opus-${OPUS_VERSION}.tar.gz"
    download_checked "https://github.com/xiph/opus/releases/download/v${OPUS_VERSION}/opus-${OPUS_VERSION}.tar.gz" \
      "$opus_tar" "$OPUS_SHA256"
    opus_build="$work/opus-build"
    rm -rf "$opus_build"
    mkdir -p "$opus_build"
    tar -xzf "$opus_tar" -C "$opus_build" --strip-components=1
    (
      cd "$opus_build"
      ./configure --host=x86_64-w64-mingw32 --enable-shared --disable-static \
        --disable-doc --disable-extra-programs >/dev/null
      make -j"$jobs" >/dev/null
      x86_64-w64-mingw32-strip -s .libs/libopus-0.dll
    )
    mkdir -p "$wine_cache/opus"
    cp "$opus_build/.libs/libopus-0.dll" "$cached_opus"
    cp "$opus_build/COPYING" "$cached_opus_lic"
  fi
  cp "$cached_opus" "$runtime_stage/opus.dll"
  mkdir -p "$runtime_stage/licenses"
  cp "$cached_opus_lic" "$runtime_stage/licenses/opus-BSD-3-Clause.txt"
fi

# 6. Patched wineserver.prx + ntdll.prx + win32u.prx + report.json (cached in wine_cache)
cached_wine="$wine_cache/wine-build"
if [ ! -f "$cached_wine/prx/sce_module/wineserver.prx" ] || [ ! -f "$cached_wine/prx/sce_module/ntdll.prx" ] || [ ! -f "$cached_wine/prx/sce_module/win32u.prx" ] || [ ! -f "$cached_wine/report.json" ]; then
  echo "==> Building PS5 wineserver.prx, ntdll.prx & win32u.prx (Wine ${WINE_REV:0:12})"
  wine_src="$work/wine-source"
  fetch_commit "$WINE_REPO" "$WINE_REV" "$wine_src"

  host_build="$work/wine-host-tools"
  mkdir -p "$host_build"
  if [ ! -x "$host_build/tools/winebuild/winebuild" ]; then
    (
      cd "$host_build"
      "$wine_src/configure" --enable-archs=i386,x86_64 --disable-tests \
        --without-x --without-freetype --without-wayland --without-gstreamer >/dev/null
      make -j"$jobs" tools/winebuild/winebuild tools/widl/widl tools/winegcc/winegcc tools/wrc/wrc tools/wmc/wmc >/dev/null
    )
  fi

  wine_ps5_work="$work/wine-ps5"
  rm -rf "$wine_ps5_work"
  bash "$pw_src/tools/build_wine_ps5.sh" \
    --work "$wine_ps5_work" \
    --source "$wine_src" \
    --host-tools "$host_build" \
    --sdk "$sdk" \
    --foundation "$prx_foundation" \
    --prx-foundation "$prx_foundation" \
    --jobs "$jobs"

  rm -rf "$cached_wine"
  mkdir -p "$cached_wine/prx/sce_module"
  cp "$wine_ps5_work/prx/sce_module/wineserver.prx" "$cached_wine/prx/sce_module/wineserver.prx"
  cp "$wine_ps5_work/prx/sce_module/ntdll.prx" "$cached_wine/prx/sce_module/ntdll.prx"
  cp "$wine_ps5_work/prx/sce_module/win32u.prx" "$cached_wine/prx/sce_module/win32u.prx"
  cp "$wine_ps5_work/report.json" "$cached_wine/report.json"
fi

# 7. Clean PS5 console prefix seed (cached in wine_cache)
cached_prefix="$wine_cache/prefix-seed"
if [ ! -f "$cached_prefix/system.reg" ] || [ ! -f "$cached_prefix/dosdevices/.pw-symlinks" ]; then
  echo "==> Initializing clean PS5 console Wine prefix seed"
  rm -rf "$cached_prefix"
  PYTHONPATH="$root" python3 -m standalone.native.build_prefix_seed \
    --release-zip "$pw_zip" \
    --output "$cached_prefix"
fi

# 8. Bundled MIT lapy.elf one-shot helper for PPSA83601
lapy_src="$work/PS5-Lapy-JB-Daemon"
fetch_commit "$LAPY_REPO" "$LAPY_REV" "$lapy_src"
echo "==> Building lapy.elf for PPSA83601"
python3 "$lapy_src/tools/build_owned_daemon.py" \
  --sdk "$sdk" \
  --logging-client "$pw_src/native/ps5log" \
  --elf-helper \
  --title PPSA83601
lapy_elf="$lapy_src/build/owned_root_helper-PPSA83601/lapy.elf"
[ -f "$lapy_elf" ] || { echo "Missing built lapy.elf" >&2; exit 1; }

# 9. MIT native launcher (eboot.bin)
launcher_bin="$work/eboot-mit.bin"
rm -f "$launcher_bin"
echo "==> Building MIT native launcher (eboot.bin)"
PYTHONPATH="$root" python3 -m standalone.native.build_launcher \
  --sdk "$sdk" \
  --llvm-bin "$llvm_bin" \
  --native-tool "$native_tool" \
  --output "$launcher_bin" \
  --with-seed

# 10. Patched PS5 Wine runner (pw_runner.bin + win/wine modules + Wine Mono)
wine_mono_tar="$wine_cache/wine-mono-${WINE_MONO_VERSION}-x86.tar.xz"
download_checked "https://github.com/wine-mono/wine-mono/releases/download/wine-mono-${WINE_MONO_VERSION}/wine-mono-${WINE_MONO_VERSION}-x86.tar.xz" \
  "$wine_mono_tar" "$WINE_MONO_SHA256"

runner_dir="$work/runner-standalone"
rm -rf "$runner_dir"
echo "==> Building patched TrinityXSX runner (with Wine Mono ${WINE_MONO_VERSION})"
PYTHONPATH="$root" python3 -m standalone.native.build_runner \
  "$pw_src" "$foundation" "$sdk" "$llvm_bin" "$native_tool" "$pw_zip" "$runner_dir" \
  --wine-build "$cached_wine" \
  --lapy-elf "$lapy_elf" \
  --wine-mono "$wine_mono_tar" \
  --wine-mono-version "$WINE_MONO_VERSION"

# 11. Stage PPSA83601 data seed (prefix + profiles + XCL runtime + xcl-library.exe)
data_seed="$work/PPSA83601-seed"
rm -rf "$data_seed"
echo "==> Staging PPSA83601 data seed"
PYTHONPATH="$root" python3 -m standalone.stage "$data_seed" \
  --library-exe "$library_exe" \
  --xcl-runtime "$runtime_stage" \
  --prefix-seed "$cached_prefix"

# 12. Assemble and fsck_ufs-verify PPSA83601.ffpkg (and optional ProsperoStore PPSA83601.zip)
rm -f "$output"
mkdir -p "$(dirname "$output")"
zip_args=()
if [ -n "$zip_output" ]; then
  rm -f "$zip_output"
  mkdir -p "$(dirname "$zip_output")"
  zip_args=(--zip-output "$zip_output")
fi
echo "==> Building UFS2 image ($content_version) -> $output"
PYTHONPATH="$root" python3 -m standalone.image \
  "$runner_dir" "$output" \
  --launcher "$launcher_bin" \
  --data-seed "$data_seed" \
  --content-version "$content_version" \
  --ufs2tool "$ufs2_dll" \
  "${zip_args[@]}"
