# TrinityXSX
<p align="center">
  <a href='https://ko-fi.com/A0A21SWG1P' target='_blank'><img height='36' style='border:0px;height:36px;' src='https://storage.ko-fi.com/cdn/brandasset/v2/support_me_on_kofi_blue.png' border='0' alt='Support project' /></a>
</p>

<p align="center">
  <a href="https://github.com/grzegorz2047/TrinityXSX/releases"><strong>Download latest release</strong></a> ·
  <a href="docs/USER_GUIDE.md">User Guide (EN)</a> ·
  <a href="docs/PORADNIK_PL.md">Poradnik (PL)</a> ·
  <a href="docs/COMPATIBILITY.md">Game Compatibility</a> ·
  <a href="https://github.com/grzegorz2047/TrinityXSX/issues/new/choose">Report a Game / Bug</a>
</p>

**TrinityXSX (`PPSA83601`)** is an experimental standalone **Xbox Series X|S (GDK / Scarlett)** game library and compatibility runtime for jailbroken **PlayStation 5** consoles. It launches directly from its own PS5 home-screen tile (no `prospero-win` front-end required) by combining [XCL](https://github.com/xcl-dev/xcl), [vkd3d-proton](https://github.com/HansKristian-Work/vkd3d-proton) (D3D12 → Vulkan on PS5 RADV), [DXVK](https://github.com/doitsujin/dxvk), and the PS5 Wine runtime.

> **Experimental software:** game compatibility varies significantly between titles. No games, BIOS, keys, or console SDK files are included — you must supply your own unpacked Xbox Series X|S game folders.

## What TrinityXSX can do

- **Standalone PS5 home-screen app (`PPSA83601.zip` for ProsperoStore / `/data/homebrew/PPSA83601`, plus optional `PPSA83601.ffpkg`)** — MIT-licensed native launcher (`eboot.bin`) + bundled MIT one-shot elevation helper (`lapy.elf`) + self-seeding Wine prefix.
- **Multi-folder game library & DualSense folder browser** — scans `C:\XCL\games`, `/data/xboxseries`, `/mnt/ext0..1/xboxseries`, `/mnt/usb0..1/xboxseries`, and any custom folder you add from the controller (`game_paths.txt`), with automatic background detection of newly copied games.
- **Local cover art & game preview** — decodes PNG artwork straight from each game's `MicrosoftGame.config` (no internet needed), showing `TitleId`, version, config status, and save folder size.
- **In-app per-game `xcl.toml` editor & compatibility presets (`R1`)** — apply built-in `TitleId` / tuning presets (Recommended, Low-RAM 4 GB, DXC Shaders, Series X, Debug Trace) or tune RAM/UMA budgets, Direct DXIL, raytracing, and offline mode straight from the DualSense.
- **Phone / PC Web Companion & QR code (`L1`)** — scan the QR code on the same Wi-Fi to download `trinityxsx-logs.zip` bug bundles, **back up and restore game saves (`.zip`)**, apply presets, or edit `xcl.toml` and `game_paths.txt` from a browser.
- **In-game DualSense Touchpad Quick Menu** — click the Touchpad (or `L3+R3`) anywhere in the library or inside a running Xbox game to inspect live **FPS / frame-time / VRAM** telemetry, trigger a **1-tap diagnostic dump**, Resume, Return to TrinityXSX Library, open the PS5 On-Screen Keyboard, or Exit to PS5 Home.
- **Offline profiles, multi-language UI (EN / PL / PT / ES), and 0-FPS loading stall diagnostics** — built to make testing new games and reporting bugs effortless.
- **Verified gameplay:** XCL Test Title passes **51/51** on PS5; **Minecraft Preview 1.18.3064.0 (Scarlett)** runs offline at ~60 FPS with DualSense controls, 3D worlds, persistent saves, and audio ([docs/MINECRAFT_PS5.md](docs/MINECRAFT_PS5.md)); plus two original MIT-licensed open-source 60 FPS Scarlett games included right in this repo for legal testing: **[AETHER DRIFT](samples/aether-drift/README.md)** (full 3D anti-gravity racer with 3 banked circuits, 8 racers, PBR reflections, Time Trial ghosts, and DualSense airbrakes/rumble) and **[NEON VOID XSX](samples/neon-void/README.md)** (twin-stick neon arcade shooter & live VRAM stress benchmark).

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
- **Build PC toolchain (Windows):** Rust (stable MSVC), Visual Studio Build Tools + Windows SDK, MSYS2 UCRT64 (`gcc`, `meson`, `ninja`, `glslang`, `pkgconf`, `tools`), Python 3.12+, and for `.ffpkg` packaging: Clang 18 / LLD + PS5 Payload SDK + .NET 8 (`UFS2Tool`).

---

Status & engineering notes: [docs/STATUS.md](docs/STATUS.md) · [docs/MINECRAFT_PS5.md](docs/MINECRAFT_PS5.md) · [standalone/README.md](standalone/README.md) · Upstream drafts (not sent): [docs/upstream/](docs/upstream/).

## Layout

| Path | What |
|---|---|
| `pins.env` | Pinned upstream revisions, release tags, and verified SHA-256 checksums |
| `library/` | TrinityXSX library source (GPL-2.0-only, compiled into `xcl.exe`): game catalog & folder browser, DualSense menu, covers & QR, `xcl.toml` editor & presets, Web Companion (PIN-limited, session cookie) with save backup/restore, GOG importer, profiles & i18n. See [library/README.md](library/README.md) |
| `patches/xcl/` | XCL patches (`0001`..`0032`, gaps where a patch was only library code): `--in-process`, 16 KiB PE pages, FL 11_0 fallback, PS5 GameInput/WASAPI/XGameRuntime fixes, stall watchdog, Touchpad Quick Menu telemetry & `PresentX`/`vkQueuePresentKHR` pause, PC DXBC 3D replay, Win64 Vulkan support, and (last) the hook that builds `library/` into `xcl.exe` |
| `patches/vkd3d-proton/` | vkd3d-proton patches: extended D3D12 vtables (`VKD3D_VTBL_IS`), predicated buffer copies, mesh shaders without barycentrics |
| `patches/prospero-win/` | Standalone `PPSA83601` runner, `wineserver.prx`, and native DualSense Touchpad Quick Menu + HUD telemetry patches (`0001`..`0010`) |
| `standalone/` | MIT-licensed native PS5 launcher (`eboot.bin`), catalog/staging/UFS2 `.ffpkg` image builder, unit tests |
| `configs/` | Clean example `game.example.xcl.toml`, tested Minecraft preset, and `trinityxsx-library.profile` |
| `tools/setup-sources.sh` | Fetch pinned XCL and vkd3d-proton and apply patches (`git am`); `xcl/` builds `library/` next to it |
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

- **`.github/workflows/ci.yml`** runs on `push`, `pull_request`, and `workflow_dispatch`: applies `patches/xcl/` and builds + tests XCL (`windows-2025`), applies `patches/vkd3d-proton/` and builds vkd3d-proton (MSYS2 UCRT64), runs `standalone` unit tests and verifies `patches/prospero-win/` on `ubuntu-24.04`, and assembles the PS5 package plus standalone `xcl-library.exe` artifacts. `library/` must pass `rustfmt --check` (with `library/rustfmt.toml`) and have no `clippy` findings; XCL's own tree is not formatted or linted. Each build is cached by its inputs (for XCL: `pins.env`, `patches/xcl/`, `library/`).
- **`.github/workflows/release.yml`** runs on `v*` tags (or manually via `workflow_dispatch`): builds XCL, vkd3d-proton, and the complete `fsck_ufs`-verified standalone **`PPSA83601-<tag>.ffpkg.zip`** (via [`tools/build-standalone-ffpkg.sh`](tools/build-standalone-ffpkg.sh)) and publishes a GitHub Release with `PPSA83601-<tag>.ffpkg.zip`, `xcl-library-<tag>.exe`, `TrinityXSX-prospero-win-<tag>.zip`, and `SHA256SUMS-<tag>.txt`.
- **`.github/workflows/upstream-sync.yml`** + **`tools/sync_upstream.py`** keep dependencies current without breaking reproducible builds: every Monday (or locally via `python tools/sync_upstream.py --check` / `--apply`), it checks `xcl-dev/xcl`, `HansKristian-Work/vkd3d-proton` (+ `dxil-spirv`), `doitsujin/dxvk`, `microsoft/DirectXShaderCompiler`, `mpereiraesaa/prospero-win`, and `mpereiraesaa/PS5-Lapy-JB-Daemon`, tests our patch series against upstream `HEAD`, detects upstreamed patches, and opens/updates a `chore/upstream-sync` PR so CI verifies the updated `pins.env` before merging.

## Licences

- Scripts and docs here: GPL-2.0-only ([LICENSE](LICENSE)), as XCL. The independent `standalone/` launcher (`crt0.c`, `launcher.c`, `prefix_seed.c`, `launcher.ld`), packaging tools, and documentation use the [MIT License](standalone/LICENSE).
- `patches/xcl/` and `library/`: GPL-2.0-only (XCL). `patches/vkd3d-proton/`: LGPL-2.1-or-later (vkd3d-proton). `patches/prospero-win/`: LGPL-2.1 / upstream terms.
- The runtime bundles DXVK (zlib) and DXC (LLVM / Microsoft licences); all licence files are included in `licenses/`.
- No console SDK files, no game data, no keys, and nothing for getting around DRM. Nothing from TrinityX360's GPL-3.0 code is used.
