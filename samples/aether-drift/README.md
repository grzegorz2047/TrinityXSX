# AETHER DRIFT

An original, MIT-licensed, open-source **3D 1080p 60 FPS anti-gravity racing game** built in pure Rust + **D3D12.X** for **Xbox Series X|S (Scarlett)** running natively on **PlayStation 5 via TrinityXSX**.

Zero proprietary SDKs, zero DRM — compiles cleanly with standard Rust (`cargo build --release`) and exercises true 3D D3D12.X depth rendering, PBR-inspired wet reflections, DualSense analog triggers + airbrakes + haptics, continuous pitch-shifted XAudio2 turbine synthesis, and Persistent Local Storage ghost replays.

## Highlights
- **Full 3D D3D12.X Engine (`1920×1080 60 FPS`, `D32_FLOAT` Depth Buffer)**:
  - Wet reflective metallic track surface, glowing neon guardrails, chevron boost pads, overhead neon arches, and a procedural cyberpunk megacity skyline rising from twilight mist.
  - Additive 3D glow pass for sculpted hovercraft plasma thrusters, tapering 3D ribbon trails, wall-scrape sparks, and hyperspeed motion streaks.
- **3 Distinct 3D Banked Circuits**:
  1. **NEON SKYWAY** — fast twilight megacity circuit with sweeping banked turns.
  2. **ORBITAL RING** — high-altitude rollercoaster with dramatic climbs and corkscrew banking.
  3. **CANYON STORM** — twin hyper-straights and sharp 90° hairpins demanding `L1`/`R1` airbrakes.
- **Grand Prix (8 Racers) & Time Trial with Saved Ghosts (`pls/`)**:
  - Race 7 competitive AI hovercrafts or set records in Time Trial — your best run is saved to `pls/ghost_<track>.txt` and replayed as a translucent holographic cyan ghost ship!
- **DualSense-First Controls & Continuous XAudio2 Audio**:
  - Analog throttle (`R2`), WipEout-style left/right airbrakes (`L1`/`R1`), Turbo Boost (`Square`/`Triangle`) with dynamic FOV surge (`62° → 82°`), directional wall-scrape & boost rumble, plus a continuous pitch-shifted turbine hum and 128 BPM synthwave soundtrack.

## Controls
| DualSense | Action |
| --- | --- |
| **R2** or **Cross (X)** | Analog throttle (Cross also starts/restarts race) |
| **Left Stick / D-pad** | Steer & bank craft (or select track in menu) |
| **L1 / R1** | Left / Right Airbrake (sharp corner carving) |
| **Square (□) / Triangle (Δ)** | Turbo Boost (or switch Grand Prix / Time Trial in menu) |
| **L2** | Hard brake |
| **OPTIONS** | Return to Track Select |
| **Touchpad click** | Open TrinityXSX Quick Menu (live FPS/VRAM, Diagnostic Dump, Return to Library) |

## Building from source
```powershell
cd samples/aether-drift
cargo build --release
```
Copy `target/release/aether_drift.exe`, `assets/MicrosoftGame.config`, and `assets/Cover.png` into `/mnt/ext1/xboxseries/aether-drift/` on your PS5.
