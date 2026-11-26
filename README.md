# TrinityXSX
<p align="center">
  <a href='https://ko-fi.com/A0A21SWG1P' target='_blank'><img height='36' style='border:0px;height:36px;' src='https://storage.ko-fi.com/cdn/brandasset/v2/support_me_on_kofi_blue.png' border='0' alt='Support project' /></a>
</p>

<p align="center">
  <a href="https://prosperostore.net/app/PPSA83601/"><strong>Install via ProsperoStore</strong></a> ·
  <a href="https://github.com/grzegorz2047/TrinityXSX/releases"><strong>Download latest release</strong></a> ·
  <a href="docs/USER_GUIDE.md">User Guide (EN)</a> ·
  <a href="docs/PORADNIK_PL.md">Poradnik (PL)</a> ·
  <a href="docs/COMPATIBILITY.md">Game Compatibility</a> ·
  <a href="https://github.com/grzegorz2047/TrinityXSX/issues/new/choose">Report a Game / Bug</a>
</p>

**TrinityXSX (`PPSA83601`)** is an all-in-one **Xbox Series X|S** and **DRM-free PC/GOG** game library and compatibility runtime for jailbroken **PlayStation 5** consoles. It installs as a single self-contained PS5 home-screen app — no separate PC launcher or extra front-end required — powered by open-source [XCL](https://github.com/xcl-dev/xcl), [vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton), [DXVK](https://github.com/doitsujin/dxvk), [innoextract](https://github.com/dscharrer/innoextract), and the PS5 Wine runtime.

> **Experimental software:** game compatibility varies between titles. No games, BIOS, keys, or console SDK files are included — supply your own unpacked Xbox Series X|S game folders or DRM-free GOG offline installers.

## What TrinityXSX gives you on PS5

- **One-click standalone PS5 app** — install `PPSA83601.zip` directly from **[ProsperoStore](https://prosperostore.net/app/PPSA83601/)** (or mount `PPSA83601.ffpkg` with ShadowMountPlus) and launch straight from the PS5 home screen.
- **Play Xbox Series X|S & DRM-free PC/GOG games from one library** — automatically detects `[XSX]`, `[PC64]`, and `[PC32]` titles, running Xbox Series games through XCL and handing off PC/GOG games cleanly to DXVK / vkd3d-proton / Zink with DualSense → XInput controls.
- **On-console GOG offline installer (`OPTIONS → Install GOG offline games`)** — drop DRM-free GOG offline installers (`setup_*.exe` + `.bin` files) into `/mnt/ext1/xboxseries/installers/` via FTP and unpack + convert them right on the PS5 with a live progress bar, automatic cover art (`Cover.png`), and ready-to-launch library entries (100% open-source `innoextract`, no PC needed).
- **Multi-folder game library & DualSense browser** — automatically scans internal (`/data/xboxseries`), extended M.2 (`/mnt/ext0..1/xboxseries`), USB (`/mnt/usb0..1/xboxseries`), and custom folders (`game_paths.txt`), with local cover art, instant refresh, and flicker-free UI.
- **In-game DualSense Touchpad Quick Menu & FPS HUD** — click the Touchpad (or `L3+R3`) anytime to check live **FPS / frame-time / VRAM** stats, toggle **Frame Generation (2x / 3x / 4x)**, grab a **1-tap diagnostic dump**, open the PS5 On-Screen Keyboard, or jump straight back to the TrinityXSX library.
- **Per-game tuning presets (`R1`) & offline profiles** — switch presets (Recommended, Low-RAM 4 GB, DXC Shaders, Series X, Debug Trace), edit `xcl.toml` right from the DualSense, and use multiple offline profiles in EN / PL / PT / ES.
- **Phone / PC Web Companion (`L1` QR code)** — scan the QR code on the same Wi-Fi to back up and restore saves (`.zip`), download bug-report logs, or tweak configs from any browser.
- **Verified gameplay & included open-source test games:** XCL Test Title passes **51/51** on PS5; **Minecraft Preview 1.18.3064.0 (Scarlett)** runs offline at ~60 FPS with DualSense controls, 3D worlds, saves, and audio ([docs/MINECRAFT_PS5.md](docs/MINECRAFT_PS5.md)); plus two MIT-licensed 60 FPS Scarlett sample games right in this repo: **[AETHER DRIFT](samples/aether-drift/README.md)** (3D anti-gravity racer) and **[NEON VOID XSX](samples/neon-void/README.md)** (twin-stick arcade shooter & VRAM stress test).

## Supported PS5 Firmware & Console Requirements

| Requirement | Details |
| --- | --- |
| **PS5 Firmware** | Any **jailbroken PS5 firmware** capable of running homebrew + `elfldr` + **ShadowMountPlus** + **kstuff** (commonly FW **4.xx – 10.xx** depending on active jailbreak/etaHEN support; developed and verified on a jailbroken PS5 with `etaHEN`, `kstuff`, `elfldr` on port `9021`, and `ShadowMountPlus`). Other firmware/loader combinations may work but are unverified — please report results in [Game Compatibility](docs/COMPATIBILITY.md). |
| **Standalone install** | **Self-contained** — does **not** require `prospero-win` to be installed separately. Install `PPSA83601.zip` via **ProsperoStore** (or extract `PPSA83601/` to `/data/homebrew/PPSA83601`), or mount `/mnt/ext1/PPSA83601.ffpkg` with ShadowMountPlus. |
| **Game storage** | Internal `/data/xboxseries`, extended `/mnt/ext1/xboxseries`, or USB `/mnt/usb0..1/xboxseries` (unpacked Scarlett game folder with `MicrosoftGame.config`). |
| **PC tools (optional)** | Any FTP client (FileZilla / WinSCP) or Python 3.12+ for `tools/ftp_put_tree.py` and `tools/collect_ps5_logs.py`. |

## Upstream & Build Dependencies

All external dependencies are pinned in [`pins.env`](pins.env) and tracked automatically by [`tools/sync_upstream.py`](tools/sync_upstream.py):

- **[XCL](https://github.com/xcl-dev/xcl)** (GPL-2.0-only) + `patches/xcl/` — Xbox Series X|S GDK → Windows compatibility layer and TrinityXSX library UI.
- **[vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton)** (LGPL-2.1-or-later) + **dxil-spirv** (MIT) + `patches/vkd3d-proton/` — D3D12 → Vulkan translation for PS5 RADV.
- **[DXVK](https://github.com/doitsujin/dxvk)** (zlib) — `dxgi.dll` / `d3d11.dll`.
- **[DirectXShaderCompiler (DXC)](https://github.com/microsoft/DirectXShaderCompiler)** (LLVM / MS) — `dxcompiler.dll` / `dxil.dll`.
- **[prospero-win](https://github.com/mpereiraesaa/prospero-win)** (`v0.3.0`) + `patches/prospero-win/` — PS5 Wine runtime & `wineserver.prx`.
- **[PS5-Lapy-JB-Daemon](https://github.com/mpereiraesaa/PS5-Lapy-JB-Daemon)** (MIT) — bundled `lapy.elf` one-shot helper compiled for `PPSA83601`.
- **[innoextract](https://github.com/dscharrer/innoextract)** (zlib) — statically linked `trinity_innoextract.dll` for in-process unpacking of DRM-free GOG offline installers.
- **Build PC toolchain (Windows):** Rust (stable MSVC), Visual Studio Build Tools + Windows SDK, MSYS2 UCRT64 (`gcc`, `meson`, `ninja`, `glslang`, `pkgconf`, `tools`), Python 3.12+, and for `.ffpkg` packaging: Clang 18 / LLD + PS5 Payload SDK + .NET 8 (`UFS2Tool`).

---

Status & engineering notes: [docs/STATUS.md](docs/STATUS.md) · [docs/MINECRAFT_PS5.md](docs/MINECRAFT_PS5.md) · [standalone/README.md](standalone/README.md) · Upstream drafts (not sent): [docs/upstream/](docs/upstream/).

## Layout

| Path | What |
|---|---|
| `pins.env` | Pinned upstream revisions, release tags, and verified SHA-256 checksums |
| `patches/xcl/` | XCL patches (`0001`..`0024`): `--in-process`, 16 KiB PE pages, FL 11_0 fallback, PS5 GameInput/WASAPI/XGameRuntime fixes, TrinityXSX library UI, covers, `xcl.toml` editor & presets, Web Companion QR + save backup/restore, stall watchdog, profiles & i18n, Touchpad Quick Menu telemetry & `PresentX`/`vkQueuePresentKHR` pause, PC DXBC 3D replay, Win64 Vulkan support, and MihawkGen Frame Generation toggle |
| `patches/vkd3d-proton/` | vkd3d-proton patches: extended D3D12 vtables (`VKD3D_VTBL_IS`), predicated buffer copies, mesh shaders without barycentrics |
| `patches/prospero-win/` | Standalone `PPSA83601` runner, `wineserver.prx`, and native DualSense Touchpad Quick Menu + HUD telemetry patches (`0001`..`0007`) |
| `standalone/` | MIT-licensed native PS5 launcher (`eboot.bin`), catalog/staging/UFS2 `.ffpkg` image builder, unit tests |
| `configs/` | Clean example `game.example.xcl.toml`, tested Minecraft preset, and `trinityxsx-library.profile` |
| `tools/setup-sources.sh` | Fetch pinned XCL and vkd3d-proton and apply patches (`git am`) |
| `tools/fetch-deps.sh` | Download DXVK and DXC releases (SHA-256 verified) |
| `tools/sync_upstream.py` | Check upstream repos/releases, test patch series (`git am --3way`), auto-drop upstreamed patches, and update `pins.env` |
| `tools/collect_ps5_logs.py` | One-command FTP log collector creating a shareable `trinityxsx-logs-*.zip` for bug reports |
| `tools/make-ps5-package.sh` | Assemble `dist/prospero-win/` runtime bundle with third-party licenses |
| `tools/ftp_put_tree.py` | Upload a local folder tree to the PS5 in one FTP session |

## Building from Source (Windows)

```sh
tools/setup-sources.sh                  # xcl/ and vkd3d-proton/ with all patches
(cd xcl && cargo build --workspace --release)
# in an MSYS2 UCRT64 shell:
(cd vkd3d-proton && meson setup build-ndebug --buildtype release -Db_ndebug=true --strip && ninja -C build-ndebug)
tools/fetch-deps.sh
tools/make-ps5-package.sh               # → dist/prospero-win/
```

`vkd3d-proton` is built with `-Db_ndebug=true` so release builds omit asserts.
For full standalone `PPSA83601.ffpkg` packaging steps, see [standalone/README.md](standalone/README.md).

## CI, Releases & Upstream Sync

- **`.github/workflows/ci.yml`** runs on `push`, `pull_request`, and `workflow_dispatch`: applies `patches/xcl/` and builds + tests XCL (`windows-2025`), applies `patches/vkd3d-proton/` and builds vkd3d-proton (MSYS2 UCRT64), runs `standalone` unit tests and verifies `patches/prospero-win/` on `ubuntu-24.04`, and assembles the PS5 package plus standalone `xcl-library.exe` artifacts. Each build is cached by its inputs.
- **`.github/workflows/release.yml`** runs on `v*` tags (or manually via `workflow_dispatch`): builds XCL, vkd3d-proton, and the complete `fsck_ufs`-verified standalone **`PPSA83601-<tag>.ffpkg.zip`** (via [`tools/build-standalone-ffpkg.sh`](tools/build-standalone-ffpkg.sh)) and publishes a GitHub Release with `PPSA83601-<tag>.ffpkg.zip`, `xcl-library-<tag>.exe`, `TrinityXSX-prospero-win-<tag>.zip`, and `SHA256SUMS-<tag>.txt`.
- **`.github/workflows/upstream-sync.yml`** + **`tools/sync_upstream.py`** keep dependencies current without breaking reproducible builds: every Monday (or locally via `python tools/sync_upstream.py --check` / `--apply`), it checks `xcl-dev/xcl`, `HansKristian-Work/vkd3d-proton` (+ `dxil-spirv`), `doitsujin/dxvk`, `microsoft/DirectXShaderCompiler`, `mpereiraesaa/prospero-win`, and `mpereiraesaa/PS5-Lapy-JB-Daemon`, tests our patch series against upstream `HEAD`, detects upstreamed patches, and opens/updates a `chore/upstream-sync` PR so CI verifies the updated `pins.env` before merging.

## Licences

- Scripts and docs here: GPL-2.0-only ([LICENSE](LICENSE)), as XCL. The independent `standalone/` launcher (`crt0.c`, `launcher.c`, `prefix_seed.c`, `launcher.ld`), packaging tools, and documentation use the [MIT License](standalone/LICENSE).
- `patches/xcl/`: GPL-2.0-only (XCL). `patches/vkd3d-proton/`: LGPL-2.1-or-later (vkd3d-proton). `patches/prospero-win/`: LGPL-2.1 / upstream terms.
- The runtime bundles DXVK (zlib) and DXC (LLVM / Microsoft licences); all licence files are included in `licenses/`.
- No console SDK files, no game data, no keys, and nothing for getting around DRM. Nothing from TrinityX360's GPL-3.0 code is used.
