# TrinityXSX User Guide (PS5)

Simple step-by-step instructions to install **TrinityXSX (`PPSA83601`)**, copy your own Xbox Series X|S games, configure them, and report issues.

> **Legal notice:** TrinityXSX includes **no games, no keys, no BIOS, and no console SDK files**. You must supply your own unpacked Xbox Series X|S (GDK / Scarlett) game files.

---

## 1. Requirements

- A jailbroken PS5 with **ShadowMountPlus** and **elfldr** (e.g. etaHEN + kstuff).
- An FTP client on your PC (FileZilla, WinSCP, or `tools/ftp_put_tree.py`).
- An external/extended drive mounted at `/mnt/ext1` (or internal storage `/data` / USB `/mnt/usb0`).
- Your own **unpacked** Xbox Series X|S game folder containing `MicrosoftGame.config` and the Scarlett `.exe` it references.

---

## 2. Install TrinityXSX on PS5

1. Download **`PPSA83601-v0.1.0.ffpkg.zip`** (or the latest release) from the [GitHub Releases](https://github.com/grzegorz2047/TrinityXSX/releases) page.
2. Extract `PPSA83601.ffpkg` from the ZIP archive.
3. Copy `PPSA83601.ffpkg` via FTP to your PS5 at:
   ```text
   /mnt/ext1/PPSA83601.ffpkg
   ```
4. Let **ShadowMountPlus** mount and register the package. The **TrinityXSX** tile will appear on your PS5 home screen.
5. Open **TrinityXSX** once. On first launch it automatically prepares `/data/homebrew/PPSA83601/` and creates `game_paths.txt`.

### Updating later (fast update)
When a new release only updates the library / XCL runtime, you do **not** need to replace the whole 1.3 GB `.ffpkg`:
- Download `xcl-library-vX.Y.Z.exe` from Releases and copy it over FTP to:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/XCL/xcl-library.exe
  ```

---

## 3. Where to put your games

Copy each game into its **own subfolder** inside any of the default locations:

| Location on PS5 | Best for |
| --- | --- |
| `/mnt/ext1/xboxseries/<GameName>/` | Extended / external storage (**recommended**, plenty of space) |
| `/mnt/usb0/xboxseries/<GameName>/` | USB drive (`usb0` or `usb1`) |
| `/data/xboxseries/<GameName>/` | Internal PS5 SSD |
| `/data/homebrew/PPSA83601/games/<GameName>/` | Internal app storage |

### Required folder layout
Each game folder must contain its unpacked files — specifically `MicrosoftGame.config` and the Scarlett executable it points to:

```text
/mnt/ext1/xboxseries/
  MyGame/
    MicrosoftGame.config
    MyGame.exe
    ...all game DLLs and data folders...
```

> **Important:** Wait until your FTP upload finishes **100%** before launching the game! Launching while files are still copying often causes a black screen or crash because `data/` assets are missing.

### Adding custom folders (like in TrinityX360)
You can add any folder on your PS5 right from the controller:
1. In TrinityXSX, press **Square (□)** to open **Game folders**.
2. Press **Square (□)** again (*Add folder*) to open the folder browser (starts in `/mnt`).
3. Navigate with **D-pad / Left Stick**:
   - **Cross (X)** — enter subfolder (folders with `MicrosoftGame.config` are tagged `[Xbox Series game]`)
   - **Circle (O)** — go up one level (pressing Circle in `/mnt` goes to `/` so you can browse `/data`, `/mnt/usb0`, `/mnt/ext1`, etc.)
   - **Triangle (Δ)** — add the currently opened folder to your library
   - **Square (□)** — cancel
4. Removing a folder (**Triangle** in *Game folders*) only removes it from the scan list — it **never deletes your game files**. Disconnected USB drives stay saved in the list and reappear automatically when reconnected.

---

## 4. DualSense Controls

| Button | Game library | Game folders | Folder browser |
| --- | --- | --- | --- |
| **D-pad / Left Stick** | Select game | Select folder | Select subfolder |
| **Cross (X)** | Play selected game | Browse selected folder | Open subfolder |
| **Square (□)** | Manage game folders | Add new folder | Cancel |
| **R1** | Edit game `xcl.toml` settings | — | — |
| **OPTIONS** | Switch offline profile & UI language (EN/PL/PT/ES) | — | — |
| **L1** | Open Phone/PC Web Companion & QR | — | — |
| **Triangle (Δ)** | Rescan all folders | Remove folder from list | Use current folder |
| **Circle (O)** | Exit to PS5 Home | Back to library | Go up one level |

---

## 5. Configuration files — which ones do you actually need?

For almost all games, **you do not need to create any config file manually**. TrinityXSX only uses two simple user-facing files on the PS5:

### A. `game_paths.txt` (your list of game folders)
- Path: `/data/homebrew/PPSA83601/game_paths.txt`
- Created automatically on first run and editable directly in the UI (**Square**).
- Contains one PS5 path per line (e.g. `/mnt/ext1/xboxseries`).

### B. `xcl.toml` (per-game tuning — created automatically on first launch!)
- Path:
  ```text
  /data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/xcl.toml
  ```
  (Where `<IdentityName>` comes from `<Identity Name="..."/>` in the game's `MicrosoftGame.config`.)
- On first launch, TrinityXSX generates a safe PS5 offline default (`device_type = "lockhart"`, `title_memory_mb = 6144`, `coherent_mb = 1024`, `raytracing = "off"`, `direct_dxil = "all"`, `network = false`).
- **Existing `xcl.toml` files are never overwritten**, so your tweaks are preserved across updates.
- See [configs/game.example.xcl.toml](../configs/game.example.xcl.toml) for a fully documented template.

#### Key `xcl.toml` options when troubleshooting a new game

| Option | Default | When to change it |
| --- | --- | --- |
| `device_type` / `console_type_raw` | `"lockhart"` / `5` (Series S) | Try `"anaconda"` / `6` (Series X) only if a game refuses Series S. |
| `title_memory_mb` | `6144` | Lower to `4096` if the game crashes with PS5 out-of-memory (`dmem`) errors. |
| `coherent_mb` | `1024` | Raise to `1536` if `xcl.log` mentions coherent window exhaustion. |
| `direct_dxil` | `"all"` | Change to `"off"` if shaders render corrupted or fail to translate (falls back to DXC). |
| `xbox_live_tokens` / `network` | `false` / `false` | Keep `false` for offline play; prevents infinite Xbox Live login loops. |
| `raytracing` | `"off"` | Keep `"off"` until the game runs stably. |
| `[aliases]` | *(empty)* | Stub unsupported PC/NVIDIA DLLs if needed, e.g. `"nvngx_dlss.dll" = "@stub"`. |

### C. Where are game saves stored?
```text
/data/homebrew/PPSA83601/prefix/drive_c/users/prospero/AppData/Local/XCL/<IdentityName>/pls/
```
Back up this folder over FTP whenever you want a backup of your saves.

---

## 6. Troubleshooting

| Symptom | What to check |
| --- | --- |
| **Game does not appear in library** | Verify the folder contains `MicrosoftGame.config` with a `TargetDeviceFamily="Scarlett"` `<Executable>` and that the `.exe` exists inside the same folder. Press **Triangle** to rescan. |
| **Black screen after launching** | Make sure the entire game finished copying (all `data/` files). Wait 30–60s on first boot for shader cache creation. Check `xcl.log` for errors. |
| **Crash / returns to PS5 Home** | Check `/data/homebrew/PPSA83601/startup.log` and `xcl.log`. Try lowering `title_memory_mb = 4096` or setting `direct_dxil = "off"`. |
| **Infinite sign-in screen** | Confirm `xbox_live_tokens = false` and `network = false` in that game's `xcl.toml`. |
| **No controller input** | Ensure `keyboard_mouse = false` in `xcl.toml`. |

---

## 7. How to report a game bug so we can fix it together

1. Run our log collector on your PC (while PS5 FTP is active):
   ```sh
   python tools/collect_ps5_logs.py <PS5-IP>
   ```
   This packs only logs (`startup.log`, `session-*.log`, `vkd3d-trinityxsx-library.log`, `xcl.log`, `report.json`) and `xcl.toml` into a single `trinityxsx-logs-*.zip` (no game files or saves).
2. Open a **[Game compatibility report](https://github.com/grzegorz2047/TrinityXSX/issues/new?template=game_compatibility.yml)** on GitHub and attach the ZIP.
