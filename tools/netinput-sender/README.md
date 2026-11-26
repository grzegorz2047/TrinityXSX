# txin-sender

Sends this PC's keyboard and mouse to TrinityXSX on a PS5 over the LAN. The PS5 side is
`patches/prospero-win/0012-Add-network-keyboard-and-mouse-input.patch`: the title listens on UDP port
9396 and posts what arrives into Wine, the same way it posts a USB keyboard and mouse.

## Build

```powershell
cargo build --release --manifest-path tools/netinput-sender/Cargo.toml
```

There are no dependencies, so the build works offline.

## Use

```powershell
txin-sender 192.168.1.20
```

- **Ctrl+Alt+F12** starts capturing. From then on, keys, mouse buttons, motion and the wheel go to the PS5 only,
  and Windows does not see them. Press **Ctrl+Alt+F12** again to stop.
- **Ctrl+C** in the console quits. The PS5 releases whatever was held. It also does this by itself after 0.5 s
  with no datagrams.
- `--test` sends a short pattern (W held for 0.3 s, then the mouse in a circle) without capturing anything.
  Use it to check that the PS5 receives.
- `--port N` sets another port.

The PS5 accepts one sender at a time, from a private address only (10/8, 172.16/12, 192.168/16, 169.254/16).
A second PC can take over once the first has been silent for 0.5 s. There is no pairing yet, so anyone on the
LAN can type into the game while the title runs.

## Protocol

TXIN v1 is a 64-byte datagram that carries the whole state: held virtual keys, buttons, and motion and wheel as
running totals. A lost datagram is therefore mended by the next one. See `src/protocol.rs` and prospero-win's
`src/pw_netinput.h`.

## Not verified yet

- Capture with the real hooks: the hooks are not exercised by any test.
- Whether Windows still delivers Raw Input while the low-level hook hides the event. The sender handles either
  case: it falls back to the hook's coordinates, which include Windows pointer acceleration.
- On a PS5: the `sceNetBind` / `sceNetRecvfrom` path, and how games under Wine react to the input.
