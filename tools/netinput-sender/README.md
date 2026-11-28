# PC keyboard & mouse for TrinityXSX

TrinityXSX can take a Windows PC's keyboard and mouse over the LAN, as well as a USB keyboard and mouse plugged
into the PS5. Pick how in the library: **Settings → PC keyboard & mouse**. The setting is
`[input] remote = off | browser | sender` in the runtime profile, and the PS5 applies a change within two seconds.

| Setting | On the PC | Captures |
|---|---|---|
| **Off** (default) | nothing: plug a USB keyboard and mouse into the PS5 | everything; no port is open on the console |
| **Web browser** | open the address the Touchpad menu (and the setting) shows, e.g. `http://192.168.1.170:8361`, and enter the PIN | everything except Esc, Alt+Tab and the Windows key, which the browser keeps; the `` ` `` key (below Esc) sends Esc to the game |
| **txin-sender program** | run `txin-sender.exe` (every release ships `txin-sender-<tag>.exe`) | everything, Esc, Alt+Tab and the Windows key included |

A USB keyboard and mouse plugged into the PS5 work in every mode, and so does the pad: they all add up.

**F9** on the PC is the Touchpad in every mode: it opens and closes TrinityXSX's quick menu in a game. While the
menu is open, the ↑ and ↓ arrows choose, **Enter** confirms and **Backspace** (or Esc, `` ` `` from the page)
closes it; those keys and the mouse do not reach the game then. F9 itself never reaches the game.

## Web browser: nothing to install

1. In TrinityXSX, set **PC keyboard & mouse** to **Web browser**. The setting row, and the Touchpad menu in a game,
   show the address and a PIN (for example `http://192.168.1.170:8361  PIN 4821`). The setting is the console's:
   the library and every game started from it follow it. The PIN is kept on the console
   (`/data/homebrew/PPSA83601/netinput.pin`; delete it for a new one), so when a game starts and the PS5's side
   restarts, the page reconnects by itself for up to a minute; click **Full screen** again to carry on.
2. On the PC, open that address, enter the PIN and click **Full screen**.
3. Press **Ctrl+Alt+F12** to start capturing, and again to stop.
4. **Esc** stops at once and leaves full screen. In the game, `` ` `` is Esc.

The page is served by the PS5 itself (`patches/prospero-win/0013`). Only private (LAN) addresses get an answer.
The WebSocket wants the PIN, an IP address as the host name (no DNS rebinding) and the page's own origin. Five
wrong PINs lock it for 30 s, doubling after each further wrong one. One page drives input at a time, and a new
one takes over. Everything held is released when the page stops, closes, or goes silent for 1.5 s.

A page from the console is plain `http://`, so the browser does not let it lock the keyboard: that is why Esc,
Alt+Tab and the Windows key stay with the PC. Closing the tab (Ctrl+W) asks first while you play.

## txin-sender program: full capture

Double-click `txin-sender.exe`, or run it with no arguments. It serves the same page on http://127.0.0.1:9397/ and
opens it in the browser. Served from this computer, the page may lock the keyboard (Keyboard Lock), so the game
gets Esc, Alt+Tab and the Windows key too. Chrome or Edge work best.

1. Enter the PS5's address and click **Full screen**.
2. Press **Ctrl+Alt+F12** to start capturing, and again to stop.
3. **Panic:** hold **Esc** for 1.5 s. Capture stops and the page leaves full screen. A short Esc goes to the game.
   A red bar along the bottom fills up while you hold it.

Keep the console window open while you play. `txin-sender --web 192.168.1.170` fills in the address. If the page
hangs or is paused (a dialog, the debugger) for 1.5 s while capturing, the helper stops and releases everything.

Without a browser, `txin-sender 192.168.1.170` captures with low-level hooks: **Ctrl+Alt+F12** starts and stops
everywhere in Windows, and holding **Esc** for 1.5 s stops it. `txin-sender 192.168.1.170 --test` sends a short
pattern (W held for 0.3 s, the mouse in a circle) without capturing anything, to check that the PS5 receives.

The helper's page and WebSocket only answer on 127.0.0.1 and only to the page itself (Host and Origin checked).
Datagrams go only to private (LAN) addresses, and the PS5 follows one sender at a time, releasing everything after
0.5 s with no datagrams.

## Build

```powershell
cargo build --release --manifest-path tools/netinput-sender/Cargo.toml
```

There are no dependencies, so the build works offline. CI runs `cargo fmt --check`, `cargo clippy` and
`cargo test` and builds the exe for every release.

The page is one file, `src/web/index.html`, for both homes. After changing it, regenerate the console's copy and
refresh `patches/prospero-win/0013`:

```powershell
python tools/netinput-sender/make_page_c.py <prospero-win checkout>/src/pw_webinput_page.c
```

## Protocols

- **txin-sender → PS5:** TXIN v1, a 64-byte UDP datagram to port 9396 that carries the whole state (held virtual
  keys, buttons, and motion and wheel as running totals), so a lost datagram is mended by the next one. See
  `src/protocol.rs` and prospero-win's `src/pw_netinput.h`.
- **Page → PS5 or helper:** short text commands over a WebSocket (`on`, `off`, `h`, `k <vk> <0|1>`,
  `b <button> <0|1>`, `m <dx> <dy>`, `w <detents>`). See prospero-win's `src/pw_webinput.h` and `src/web.rs`.
