# TrinityXSX Game Compatibility

TrinityXSX is experimental. Every Xbox Series X|S (GDK / Scarlett) title has different memory, shader, audio, and GDK API requirements — a menu booting is not yet a guarantee of full gameplay.

## Tested titles on PS5 (`PPSA83601`)

| Title | Package Identity & Version | Status | Notes |
| --- | --- | --- | --- |
| **XCL Test Title** | `XCL.TestTitle` | **Pass (51/51)** | Full D3D12.X + vkd3d-proton + RADV verification suite, 60 FPS fullscreen presentation. |
| **Minecraft Preview** | `Microsoft.MinecraftBeta` `1.18.3064.0` (`639AA8DD`) | **Playable** | Boots directly from standalone TrinityXSX tile at ~60 FPS; DualSense input (OreUI + gameplay), 3D world creation/loading, persistent saves, and WASAPI audio verified offline (`xbox_live_tokens = false`, `network = false`, `coherent_mb = 1024`, `direct_dxil = "all"`). |

## Reporting a game you tested

Tested one of your own unpacked Xbox Series X|S games? Help expand compatibility:

1. Collect logs from your PS5 with one command (collects only logs and `xcl.toml` — never game files or saves):
   ```sh
   python tools/collect_ps5_logs.py <PS5-IP>
   ```
2. Open a **[Game compatibility report](https://github.com/grzegorz2047/TrinityXSX/issues/new?template=game_compatibility.yml)** on GitHub and attach the generated `trinityxsx-logs-*.zip`.

**Rules:**
- Never upload game files, `.exe`/`.dll` binaries, keys, or links to copyrighted content.
- Include the exact `Identity Name` and `Version` from `MicrosoftGame.config` and any changes you made to `xcl.toml`.
