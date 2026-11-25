# AETHER DRIFT v2.0 (Xbox Series X|S / Scarlett — TrinityXSX on PS5)

An original, MIT-licensed, open-source **1080p 60 FPS 3D anti-gravity racing game** built natively in Rust + **D3D12.X** (`d3d12_xs.dll` `PresentX`), **XGameRuntime** (`XUser` + `PLS` saves/ghosts), **XInput** (DualSense analog triggers & rumble), and **XAudio2** (continuous pitch-shifted turbine + 128 BPM synthwave soundtrack).

## Highlights (v2.0)
- **Sculpted Blender 5.2 LTS 3D Models** (`tools/gen_blender_assets.py`): smooth beveled 2,420-tri Twin-Pod Interceptor (`hero_craft.bin`), 1,344-tri Rival Craft (`rival_craft.bin`), and multi-tier cyberpunk skyscraper spires (`skyscraper_spire.bin`).
- **Hardware PBR Textures** (`tools/gen_textures.py`): wet brushed-titanium track deck with expansion joints, organic rain-puddle reflection & micro-normal map, glowing cyberpunk billboards (`NEO-KYOTO`, `VORTEX ENERGY`, `CHRONOS TECN`, `AETHER-1` + animated turn chevrons), and illuminated skyscraper window facades.
- **Real-Time GPU Volumetric Storm Clouds & Megacity Mist** (`shaders/aether_v2.hlsl`): 22-step GPU 3D raymarched storm thunderheads (5-octave FBM, Beer-Lambert self-shadowing, Henyey-Greenstein twilight scattering) + analytical volumetric height fog + 6 layered drifting megacity mist sheets weaving between skyscrapers.
- **True Stencil-Masked Wet-Track Mirror Reflections**: hardware `D24_UNORM_S8_UINT` stencil pass mirrors all 8 hovercrafts, plasma thruster cones, neon rail bars, and billboards across the banked curves inside rain puddles.
- **Offscreen HDR Bloom & ACES Tone Mapping**: `R16G16B16A16_FLOAT` HDR scene target -> quarter-res bright-pass + separable Gaussian blur -> ACES filmic composite with subtle hyperspeed chromatic aberration.
- **3 Banked Spline Circuits & 2 Modes**: *Neo-Kyoto Skyway*, *Orbital Ring*, *Canyon Storm*; 8-racer **Grand Prix** + solo **Time Trial** with saved translucent cyan **Ghost** replays in Xbox `PLS` (`S:\`).

## DualSense Controls
- `R2` / `Cross (X)` — Analog throttle
- `Left Stick` — Steer & bank craft
- `L1` / `R1` — Left / right airbrake (tight hairpin carving)
- `Square` / `Triangle` — Turbo Boost (recharge via track boost chevrons)
- `L2` — Hard brake
- `Options` — Return to track select
- `Touchpad` — TrinityXSX Quick Menu

## Building from Source
1. *(Optional)* Regenerate meshes in Blender 5.2+:
   ```bash
   blender --background --python tools/gen_blender_assets.py
   ```
2. *(Optional)* Regenerate PBR textures (using Python + NumPy):
   ```bash
   python tools/gen_textures.py
   ```
3. Build the Scarlett executable:
   ```bash
   cargo build --release
   ```
Requires TrinityXSX with XCL patch `0022` (PC-pipeline root CBV & SRV descriptor-table replay).
