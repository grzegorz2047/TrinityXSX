# TrinityXSX User Guide (PS5)

Step-by-step instructions to install **TrinityXSX (`PPSA83601`)**, add your own **Xbox Series X|S** and **DRM-free PC / GOG** games, use DualSense controls (including the live Gamepad ⇄ Mouse & Keyboard switcher), understand configuration files, and report results.

> **Legal notice:** TrinityXSX includes **no games, no keys, no BIOS, and no console SDK files**. Supply your own unpacked Xbox Series X|S (GDK / Scarlett) game folders or your own DRM-free PC / GOG offline installers.

---

## 1. Requirements

- A jailbroken PS5 with **ShadowMountPlus** and **elfldr** (e.g. etaHEN + kstuff).
- An FTP client on your PC (FileZilla, WinSCP, or `tools/ftp_put_tree.py`) — or **ProsperoStore** directly on the PS5.
- Extended/external storage at `/mnt/ext1` (or internal `/data` / USB `/mnt/usb0`).
- Your own games:
  - **Xbox Series X|S:** unpacked game folder containing `MicrosoftGame.config` (Scarlett) and its `.exe`, **or**
  - **DRM-free PC / GOG:** GOG offline installer files (`setup_*.exe` + `.bin` slices) **or** an unpacked 32-bit/64-bit Windows PC game folder.

---

## 2. Install TrinityXSX on PS5

### Option A — ProsperoStore (easiest)
1. Open **[ProsperoStore](https://prosperostore.net/app/PPSA83601/)** on your PS5, install **TrinityXSX (`PPSA83601`)**, and launch it from the home screen.

### Option B — Standalone `.ffpkg` (ShadowMountPlus)
1. Download the latest **`PPSA83601-vX.Y.Z.ffpkg.zip`** from [GitHub Releases](https://github.com/grzegorz2047/TrinityXSX/releases).
2. Extract `PPSA83601.ffpkg` and copy it via FTP to your PS5 at:
   ```text
   /mnt/ext1/PPSA83601.ffpkg
   ```
3. Let **ShadowMountPlus** mount the package; the **TrinityXSX** tile appears on the PS5 home screen.
4. Launch **TrinityXSX** once — it automatically prepares `/data/homebrew/PPSA83601/` and `game_paths.txt`.

### Updating later
- **Full update** (new `pw_runner.bin`, `win32u.prx`, Wine Mono, etc.): close TrinityXSX to PS5 Home and replace `/mnt/ext1/PPSA83601.ffpkg` (or update via ProsperoStore). Games, saves, settings, and `xcl.toml` are preserved.
- **Fast library-only update:** download `xcl-library-vX.Y.Z.exe` from Releases and copy it over FTP to:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/XCL/xcl-library.exe
  ```

---

## 3. Adding Your Games (All Installation Methods)

TrinityXSX supports **4 ways to add games** — via **USB drive (no FTP needed)**, via **FTP**, via the **built-in on-PS5 GOG installer**, or by adding **custom folders** straight from the DualSense.

### Method 1 — Unpacked Xbox Series X|S Game (`[XSX]`)
1. Copy each unpacked game folder (containing `MicrosoftGame.config` with `TargetDeviceFamily="Scarlett"` and its `.exe`) into its **own subfolder** inside any default scan folder:

| Location on PS5 | Best for |
| --- | --- |
| `/mnt/ext1/xboxseries/<GameName>/` | Extended / external storage (**recommended**, most space) |
| `/mnt/usb0/xboxseries/<GameName>/` | exFAT USB drive (`usb0` or `usb1`) — **no FTP**, copy on PC and plug directly into PS5 |
| `/data/xboxseries/<GameName>/` | Internal PS5 SSD |
| `/data/homebrew/PPSA83601/games/<GameName>/` | Internal app storage |

2. Press **Triangle (Δ)** in the library to rescan if TrinityXSX was already open.

### Method 2 — DRM-Free GOG Offline Installers Directly on PS5 (No PC Install)
Works with GOG offline backup installers built with **Inno Setup** (`setup_<game>.exe` + all `setup_<game>-1.bin`, `-2.bin`, ...):
1. Download the **Offline Backup Game Installers** from your GOG library.
2. Copy them onto an **exFAT USB drive** or over **FTP** into any automatically scanned installer folder:
   - `/mnt/usb0/` or `/mnt/usb0/installers/` *(USB drive — installs onto your first available library drive, e.g. `/mnt/ext1/xboxseries/`)*,
   - `/mnt/ext1/xboxseries/installers/` or `/mnt/ext1/installers/` *(1-level subfolders like `installers/witcher2/` are scanned too)*,
   - or directly inside any library folder listed in `game_paths.txt`.
3. In TrinityXSX press **`OPTIONS` -> Install GOG Games**, select the game, and press **`X`**.
4. Open-source `innoextract` unpacks the game directly on PS5 with a progress bar and ETA, extracts `Cover.png`, and registers it in your library. Press **`R1`** on an installed entry to delete its installer files (or toggle **`□`** before installing to auto-delete).
5. **Bundled Wine Mono `11.3.0`** provides built-in `.NET` / `C++/CLI` / `XNA` runtime support automatically.

### Method 3 — PC Game Installed on PC & Copied to PS5 (`[PC64]` / `[PC32]`)
Best for classic disc/CD games, non-Inno installers (NSIS, InstallShield), patched/modded setups (e.g. *Age of Empires II* with *UserPatch*), or any DRM-free PC game folder:
1. Install the game (and any patches or mods) on your PC as usual.
2. Copy the entire installed game folder onto an **exFAT USB drive** at `xboxseries/<GameName>/` (`/mnt/usb0/xboxseries/<GameName>/` on PS5) **or** upload over FTP to `/mnt/ext1/xboxseries/<GameName>/`.
3. **No `MicrosoftGame.config` needed** — TrinityXSX automatically discovers `.exe` folders, detects 32/64-bit architecture, selects the graphics backend (`DXVK` / `vkd3d-proton` / `Zink` for OpenGL), and detects controller API imports.
4. **Multi-`.exe` PC games** (expansions, patches, or launchers alongside the main binary): highlight the game -> press **`R1` -> Game info -> Executable**, and pick the exact `.exe` with the DualSense.
5. **Custom cover art:** if the game has no `.ico` with an embedded PNG, drop a **`Cover.png`** file into the game's root folder to give it tile artwork.

### Method 4 — Custom Folders on USB or PS5 Storage
You do not have to keep games inside `xboxseries/` — you can add any existing folder on USB or internal storage:
1. Press **Square (□)** in the library -> **Game folders**.
2. Press **Square (□)** (*Add folder*) to browse `/mnt`, `/data`, `/mnt/usb0`, `/mnt/ext1`, etc.
3. **Cross (X)** enters a subfolder, **Circle (O)** goes up, **Triangle (Δ)** adds the current folder. Removing a folder from the list **never deletes game files**.

### Transferring Files to PS5 (USB vs FTP)
- **USB drive (exFAT):** fastest for large games with zero network setup — plug into PS5 (`/mnt/usb0` or `/mnt/usb1`) to play directly from USB or unpack GOG installers from USB onto `/mnt/ext1`.
- **FTP (FileZilla / WinSCP or the bundled script):** for uploading large directory trees cleanly to PS5 `ftpsrv.elf` (port `2121`), you can use the repo helper:
  ```sh
  python tools/ftp_put_tree.py <PS5-IP> 2121 "C:\Games\MyGame" /mnt/ext1/xboxseries/MyGame
  ```

> **Important:** When using FTP, always wait until the upload reaches **100%** before launching the game!

---

## 4. DualSense Controls

### Library Screen
| Button | Action |
| --- | --- |
| **D-pad / Left Stick** | Select game |
| **Cross (X)** | Launch selected game |
| **R1** | Game settings (`xcl.toml` for XSX; **Executable**, **PC Controls**, and shader cache for PC/GOG) |
| **Square (□)** | Manage game folders |
| **L1** | Phone/PC Web Companion & QR (logs, `.zip` save backup/restore, presets, `xcl.toml`) |
| **OPTIONS** | App Settings (GOG installer, language EN/PL/PT/ES, offline profile, Frame Generation 2x/3x/4x, FPS counter, PC keyboard & mouse) |
| **L2** | Toggle favorite |
| **R2 / Left / Right** | Cycle sort mode (A–Z / Recent / Favorites) |
| **Triangle (Δ)** | Rescan folders |
| **Circle (O)** | Exit to PS5 Home |

### In-Game Controls (Xbox Series & PC / GOG)
| DualSense Input | In-Game Action |
| --- | --- |
| **`OPTIONS`** | **Xbox `START`** — in-game pause menu |
| **Left half of Touchpad** (click left side) | **Xbox `BACK` / `Select` / `View`** — map, inventory, journal, main panel |
| **Right half of Touchpad** (click right side) or **`L3 + R3`** | Open / close **TRINITYXSX MENU** (FPS/VRAM, live **`GAMEPAD` ⇄ `MOUSE+KB`** toggle, Restart Game, Diagnostic Dump, Return to Library, On-Screen Keyboard, Exit to PS5 Home) |
| **Two-finger Touchpad click** | PS5 On-Screen Keyboard (OSK) |

### DualSense `Mouse & Keyboard` Mode for PC/GOG Games
Set it per-game in **`R1 -> Game info -> PC Controls`** (`Auto` automatically picks Mouse & Keyboard for classic PC titles without controller API imports) **or toggle live mid-game** via the **right-half Touchpad menu** (`INPUT MODE: GAMEPAD ⇄ MOUSE+KB` — handy when a PC game needs mouse clicks in Options to enable its Gamepad mode):
- **Left Stick:** mouse cursor
- **`X` / `R2`:** left mouse click · **`□` / `L2`:** right mouse click
- **`○` / `OPTIONS`:** `Esc` · **`△`:** `Enter`
- **D-pad:** `W / A / S / D` · **`L1`:** Space · **`R1`:** `Tab`

### Remote PC Keyboard & Mouse over LAN
In **`OPTIONS -> PC keyboard & mouse`** choose:
- **Web browser:** open `http://<PS5-IP>:8361` on any PC/phone on your LAN and enter the PIN shown in Settings or the Touchpad menu (full screen + `Ctrl+Alt+F12` captures input, `F9` opens the Touchpad menu).
- **`txin-sender` program:** run `txin-sender-vX.Y.Z.exe` on Windows (captures `Esc` and `Alt+Tab` too; holding `Esc` for 1.5 s always releases capture).

---

## 5. Configuration Files & Saves

For almost all games **no manual config editing is required**:
- **Scan folders:** `/data/homebrew/PPSA83601/game_paths.txt` (editable via **□**).
- **UI & per-game PC overrides** (language, profiles, favorites, selected `.exe`, PC input mode): `/data/homebrew/PPSA83601/ui_settings.toml`.
- **Per-game Xbox Series `xcl.toml`:** auto-generated on first launch at:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/xcl.toml
  ```
  Existing `xcl.toml` files are never overwritten. See [configs/game.example.xcl.toml](../configs/game.example.xcl.toml).
- **Saves:**
  - Xbox Series (`XCL`): `/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/pls/` (1-click `.zip` backup/restore in **Web Companion `L1`**).
  - PC / GOG: standard Wine prefix folders under `/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/` (`Documents`, `AppData`, `Saved Games`).

---

## 6. Troubleshooting

| Symptom | What to check |
| --- | --- |
| **PC game ignores controller sticks** | Click the **right half of the Touchpad** -> toggle **`INPUT MODE`** to **`MOUSE+KB`** and enable Gamepad in the game's Options (or set **`R1 -> PC Controls`** in the library). |
| **Need Xbox `Back / Select` (map / inventory)** | Click the **left half of the Touchpad** (right half opens TrinityXSX Menu; `OPTIONS` is `Start`). |
| **PC game folder has multiple `.exe` files** | Highlight the game -> press **`R1`** -> change **`Executable`** to the `.exe` you want. |
| **Game missing from library** | For XSX games verify `MicrosoftGame.config` (`TargetDeviceFamily="Scarlett"`). For PC games ensure the folder contains `.exe` files and press **Triangle (Δ)** to rescan. |
| **Black screen on first launch** | Wait until FTP upload finished 100%, then wait 30–60 s for initial shader cache compilation. |
| **XSX game crashes / OOM** | Press **`R1`** and try the **Low-RAM (4 GB)** preset or set **Direct DXIL** to `off (use DXC)`. |

---

## 7. Reporting Games & Bugs

1. With FTP active on your PS5, run on your PC:
   ```sh
   python tools/collect_ps5_logs.py <PS5-IP>
   ```
   Packs only logs (`startup.log`, `session-*.log`, `xcl.log`, `report.json`) and `xcl.toml` into `trinityxsx-logs-*.zip` (no game files or saves). You can also download a diagnostic dump from **Web Companion (`L1`)**.
2. Open a **[Game compatibility report](https://github.com/grzegorz2047/TrinityXSX/issues/new?template=game_compatibility.yml)** on GitHub and attach the ZIP.
