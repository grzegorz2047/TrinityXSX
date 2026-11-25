# NEON VOID XSX

An original, MIT-licensed, open-source **Xbox Series X|S (Scarlett)** 60 FPS twin-stick neon arcade shooter and hardware benchmark built in pure Rust for **TrinityXSX on PS5**.

Zero proprietary SDKs, zero DRM, zero commercial assets — compiles from scratch with standard Rust (`cargo build --release`) using `raw-dylib` imports for `kernelx.dll`, `xgameruntime.dll`, `d3d12_xs.dll`, `xinput1_4.dll`, and `xaudio2_9.dll`.

## What it exercises on PS5 (TrinityXSX)
- **D3D12.X → vkd3d-proton → PS5 RADV Vulkan**: 1920×1080 60 FPS alpha-blended dynamic vertex rendering via `PresentX` and frame-clock events (`SetFrameIntervalX`, `ScheduleFrameEventX`, `WaitFrameEventX`).
- **DualSense + Rumble**: Twin-stick movement & 360° plasma cannon, auto-aim fallback (`Cross`/`R2`), EMP Shockwave Nova (`L2`/`Square`), and haptic rumble on fire, hits, and shockwaves.
- **XAudio2 → WASAPI → PS5 Audio**: Procedural 48 kHz PCM16 arcade synth effects.
- **Offline Profile & Persistent Local Storage (`pls/`)**: Reads your active TrinityXSX profile via `XUserGetGamertag` and saves your best score, wave, and kills to `pls/highscore.txt` (testable with Web Companion **Backup/Restore Saves `.zip`**).
- **Live VRAM & GPU Particle Stress Benchmark (`R1`)**: Cycles `+256 MiB` / `+512 MiB` / `+1024 MiB` real `D3D12_HEAP_TYPE_DEFAULT` allocations + up to 2,200 particles so you can inspect live FPS & VRAM in the **Touchpad Quick Menu**.

## Controls
| DualSense | Action |
| --- | --- |
| **Left Stick / D-pad** | Fly ship |
| **Right Stick** (or **Cross / R2**) | Aim & fire plasma cannon |
| **L2 / Square (□)** | EMP Shockwave Nova |
| **R1** | Cycle VRAM + GPU Particle Stress (`OFF` / `+256` / `+512` / `+1024 MiB`) |
| **Touchpad click** | Open TrinityXSX Quick Menu (live FPS/VRAM, Diagnostic Dump, Return to Library) |

## Building from source
```powershell
cd samples/neon-void
cargo build --release
```
Copy `target/release/neon_void.exe`, `assets/MicrosoftGame.config`, and `assets/Cover.png` into `/mnt/ext1/xboxseries/neon-void-xsx/` on your PS5.
