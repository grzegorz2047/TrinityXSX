# TrinityXSX library

The TrinityXSX game library that runs on the PS5: the catalog and folder browser, the DualSense menu, covers and QR code, the per-game `xcl.toml` editor and presets, the Web Companion, the GOG installer importer, and the UI translations.

| File | What |
|---|---|
| `library.rs` | Game folder discovery, `game_paths.txt`, per-title settings and presets, saves, profiles, UI settings |
| `library_menu.rs` | The library window and DualSense / keyboard menu (part of `xcl.exe`) |
| `library_art.rs` | PNG cover decoding and the QR code encoder |
| `library_web.rs` | The LAN Web Companion: logs, `xcl.toml` and folder editing, save backup and restore |
| `library_gog.rs` | GOG offline installer import and the PC game handoff |
| `library_i18n.rs` | UI strings in EN / PL / PT / ES |

## How it is built

These files are part of the `xcl` crate. They are not a crate of their own. They use XCL's `Config`, launcher and helpers. `xcl.exe` must also stay a single executable, because it holds the TLS callback, host mode and link flags the games run in.

The last XCL patch (`patches/xcl/0032-…`) wires them in:

```rust
#[path = "../../../../library/library.rs"]
pub mod library;
```

That path is this folder, seen from `xcl/crates/xcl/src/`. `tools/setup-sources.sh` puts XCL in `xcl/` next to it. When the script is given another `DEST`, it links `DEST/library` here. The modules keep the names they had as patches (`xcl::library`, `crate::library_i18n`, …).

## Working on it

Edit the files here. Don't add XCL patches for them. After `tools/setup-sources.sh`:

```sh
cd xcl
cargo test -p xcl --release --lib library        # the library's unit tests
cargo build --release                            # xcl.exe, copied as xcl-library.exe
```

A change to XCL itself (`host.rs`, `present.rs`, `config.rs`, …) is still a patch in `patches/xcl/`.

## An `xcl/` checkout from before this layout

An `xcl/` made before the library moved here still has its own `crates/xcl/src/library*.rs`, and its `lib.rs` has no `#[path]` lines. Cargo would build those old copies and silently ignore this folder. `tools/setup-sources.sh` refuses such a checkout and changes nothing. To update it:

1. Keep your XCL-side work (`git format-patch` or `git stash` in `xcl/`). Copy any library edits from `xcl/crates/xcl/src/library*.rs` into this folder.
2. Move `xcl/` aside and run `tools/setup-sources.sh --only xcl`.
3. Check: `grep -n '#\[path' xcl/crates/xcl/src/lib.rs` lists the `library/` paths.

## Licence

GPL-2.0-only, the same as XCL: these files are compiled into `xcl.exe`.
