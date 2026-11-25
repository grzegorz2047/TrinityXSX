# TrinityXSX
<p align="center">
  <a href='https://ko-fi.com/A0A21SWG1P' target='_blank'><img height='36' style='border:0px;height:36px;' src='https://storage.ko-fi.com/cdn/brandasset/v2/support_me_on_kofi_blue.png' border='0' alt='Support project' /></a>
</p>

Work on running [XCL](https://github.com/xcl-dev/xcl) – the Xbox Series X|S (GDK) → Windows
compatibility layer – on Vulkan and on the PS5. Experimental.

XCL stays a Windows program. It runs on Vulkan through
[vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton) (D3D12 → Vulkan) with
[DXVK](https://github.com/doitsujin/dxvk)'s `dxgi.dll`, first on Windows, and on the PS5 under
[prospero-win](https://github.com/mpereiraesaa/prospero-win) (Wine for PS5 homebrew) on the
console's RADV driver. This repository holds the changes as patches on pinned upstream
revisions, the scripts that build and package them, and the findings.

Status: [docs/STATUS.md](docs/STATUS.md). Drafts for the upstream projects (not sent):
[docs/upstream/](docs/upstream/).

Minecraft Preview on PS5: [tested offline workflow, controller, world-loading and audio fixes](docs/MINECRAFT_PS5.md).
Its working files now live in this project (`configs/`, `tools/`, and ignored `games/`,
`analysis/`, `ps5-logs/` directories).

[Standalone PS5 prototype](standalone/README.md) stages the tested Minecraft profile
and provides an independent MIT-licensed catalog tool.

## Layout

| Path | What |
|---|---|
| `pins.env` | upstream revisions the patches apply to; DXVK / DXC versions and checksums |
| `patches/xcl/` | XCL: `xcl run --in-process` (one process, as Wine on the PS5 needs), no D3D11 bridge for Media Foundation on vkd3d-proton, PE section protections on 16 KiB host pages, a GPU without feature level 12_0 used at 11_0 before WARP; the test title's `--present-seconds N`, the title's window kept off the screens without DWM |
| `patches/vkd3d-proton/` | vkd3d-proton 3.1: accept D3D12 objects whose vtable XCL extended (`VKD3D_VTBL_IS`); mesh shaders without barycentrics for `xcl.exe`; predicated buffer copies |
| `patches/dxil-spirv/` | dxil-spirv: `dx.op.udiv` helper types, structured stride read from the resource tags by tag, `UAddc` / `USubb` |
| `tools/setup-sources.sh` | fetch XCL and vkd3d-proton at the pinned revisions and apply the patches |
| `tools/fetch-deps.sh` | download DXVK and DXC releases (checksums verified) |
| `tools/vk-backend.sh` | run XCL's test suite on Windows on vkd3d-proton + DXVK (`on` / `off`) |
| `tools/make-ps5-package.sh` | assemble `dist/prospero-win/` (XCL + its test title + DLLs + profile) |
| `tools/ps5-deploy-game.sh` | put an unpacked Xbox Series game folder you own on the PS5: checks it (Scarlett build, console imports), uploads it, writes its prospero-win profile |
| `tools/ftp_put_tree.py` | upload a folder to the PS5 in one FTP session |
| `tools/ftp_rm_tree.py` | empty a folder on the PS5 (the test title's XCL overlay between runs) |
| `tools/pdbsym.py` | module + offset → function, file:line from a PDB (crash reports) |
| `tools/vkcaps/` | prints the Vulkan features vkd3d-proton's tiled-resources tier and feature level depend on (run it on the PS5 as its own prospero-win profile) |

## Building (Windows)

Needs Rust (stable, MSVC), Visual Studio Build Tools with the Windows SDK, and MSYS2 with the
UCRT64 toolchain for vkd3d-proton:
`pacman -S mingw-w64-ucrt-x86_64-{gcc,meson,ninja,glslang,pkgconf,tools}`.

```sh
tools/setup-sources.sh                  # xcl/ and vkd3d-proton/ with the patches
(cd xcl && cargo build --workspace --release)
# in an MSYS2 UCRT64 shell:
(cd vkd3d-proton && meson setup build-ndebug --buildtype release -Db_ndebug=true --strip && ninja -C build-ndebug)
tools/fetch-deps.sh
tools/make-ps5-package.sh               # → dist/prospero-win/
```

vkd3d-proton is built with `-Db_ndebug=true`: its release builds keep asserts otherwise.

XCL's tests on Vulkan: `tools/vk-backend.sh on ndebug`, then `cargo test --workspace` in `xcl/`,
then `tools/vk-backend.sh off`.

## Running on the PS5

- A PS5 that runs homebrew, with prospero-win installed (tested: v0.3.0) and opened once.
- A Wine prefix at `/data/prospero-win/prefix` (the `default` prefix). Wine on the PS5 cannot
  create one (process creation is off, so wineboot cannot run); it has to be made by Wine on a
  PC (prospero-win's `tools/build_host_wine.sh` and `tools/pw_prefix.py`), or taken from a
  runtime kit that ships one. None is included here.
- Copy `dist/prospero-win/` to `/data/prospero-win/` (FTP; `tools/ftp_put_tree.py host port
  dist/prospero-win /data/prospero-win`, with `MSYS_NO_PATHCONV=1` under Git Bash), then start
  "XCL test title" in prospero-win.
- Logs: `/data/prospero-win/logs/session-N.log` (the title's output as `STDOUT` records),
  XCL's in the prefix at `drive_c/users/prospero/AppData/Local/XCL/XCL.TestTitle/logs/`,
  vkd3d-proton's at `drive_c/XCL/vkd3d.log`.

## CI, Releases & Upstream Sync

- **.github/workflows/ci.yml** runs on push, pull_request, and workflow_dispatch: applies patches/xcl/ and builds + tests XCL (windows-2025), applies patches/vkd3d-proton/ and builds vkd3d-proton (MSYS2 UCRT64), runs standalone unit tests and verifies patches/prospero-win/ on ubuntu-24.04, and assembles the PS5 package plus standalone xcl-library.exe artifacts. Each build is cached by its inputs.
- **.github/workflows/release.yml** runs on * tags (or manually via workflow_dispatch): builds all artifacts cleanly and publishes a GitHub Release with xcl-library-<tag>.exe, TrinityXSX-prospero-win-<tag>.zip, and SHA256SUMS-<tag>.txt.
- **.github/workflows/upstream-sync.yml** + **	ools/sync_upstream.py** keep dependencies current without breaking reproducible builds: every Monday (or locally via python tools/sync_upstream.py --check / --apply), it checks xcl-dev/xcl, HansKristian-Work/vkd3d-proton (+ dxil-spirv), doitsujin/dxvk, microsoft/DirectXShaderCompiler, mpereiraesaa/prospero-win, and mpereiraesaa/PS5-Lapy-JB-Daemon, tests our patch series against upstream HEAD, detects upstreamed patches, and opens/updates a chore/upstream-sync PR so CI verifies the updated pins.env before merging.
## Licences

- Scripts and docs here: GPL-2.0-only ([LICENSE](LICENSE)), as XCL. The independent
  `standalone/` tools and documentation use the [MIT License](standalone/LICENSE).
- `patches/xcl/`: GPL-2.0-only (XCL). `patches/vkd3d-proton/`: LGPL-2.1-or-later (vkd3d-proton).
  `patches/dxil-spirv/`: MIT (dxil-spirv).
- The package bundles DXVK (zlib) and DXC (LLVM / Microsoft licences); their licence files go
  into `licenses/` next to them.
- No console SDK files, no game data, no keys, and nothing for getting around DRM. Nothing from
  TrinityX360's GPL-3.0 parts is used.
