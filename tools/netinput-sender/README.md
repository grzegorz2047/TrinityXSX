# txin-sender

Lets you play a game running in TrinityXSX on a PS5 with a Windows PC's keyboard and mouse, over the LAN.
The PS5 side is `patches/prospero-win/0012-Add-network-keyboard-and-mouse-input.patch`: while a game runs, the
title listens on UDP port 9396 and posts what arrives into Wine, the same way it posts a USB keyboard and mouse.

Every release includes a ready-made `txin-sender-<tag>.exe`, so you do not need to build it.

## Use: the page in the browser

Double-click `txin-sender.exe`, or run it with no arguments. It opens http://127.0.0.1:9397/ in your browser.

1. Enter the PS5's address and click **Full screen**.
2. A full-screen message asks you to press **Ctrl+Alt+F12**. Press it to start capturing. The keyboard and mouse
   then go to the game; the mouse pointer is locked and hidden.
3. Press **Ctrl+Alt+F12** again to stop capturing. You stay in full screen.
4. **Panic:** hold **Esc** for 1.5 s, at any time. Capture stops, the keyboard and mouse are released and the page
   leaves full screen. A short Esc still goes to the game. A red bar along the bottom fills up while you hold it.

Capture also stops if you leave full screen, switch window, or close the page or the console window.

Chrome or Edge work best: in full screen they let the page take Esc, Alt+Tab and the Windows key (Keyboard Lock),
which games often need. Other browsers still work, but those keys stay with Windows and Esc frees the mouse.
Keyboard Lock only works on a page from this computer, which is why the helper serves the page itself.

Keep the console window open while you play. `txin-sender --web 192.168.1.170` opens the page with the address
filled in.

## Use: without a browser

```powershell
txin-sender 192.168.1.170
```

**Ctrl+Alt+F12** starts and stops capturing everywhere in Windows (low-level hooks), and holding **Esc** for 1.5 s
also stops it. Ctrl+C in the console quits.

`txin-sender 192.168.1.170 --test` sends a short pattern (W held for 0.3 s, then the mouse in a circle) without
capturing anything. Use it to check that the PS5 receives. `--port N` sets another port.

## Safety

- The page and its WebSocket only answer on 127.0.0.1, and only to the page itself (the Host and Origin headers
  are checked), so another website open in the browser cannot type into the PS5.
- Datagrams only go to private (LAN) addresses, and the PS5 only accepts them from those.
- The PS5 follows one sender at a time and releases everything after 0.5 s with no datagrams. A second PC can
  take over after that.
- There is no pairing yet, so anyone on the LAN can type into the game while it runs.

## Build

```powershell
cargo build --release --manifest-path tools/netinput-sender/Cargo.toml
```

There are no dependencies, so the build works offline. CI runs `cargo fmt --check`, `cargo clippy` and
`cargo test` and builds the exe for every release.

## Protocol

TXIN v1 is a 64-byte datagram that carries the whole state: held virtual keys, buttons, and motion and wheel as
running totals. A lost datagram is therefore mended by the next one. See `src/protocol.rs` and prospero-win's
`src/pw_netinput.h`. The page talks to the helper in short text commands (`src/web.rs`).
