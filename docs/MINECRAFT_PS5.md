# Minecraft Preview on PS5

Working directory: `D:\TrinityXSX`. Imported on 2026-11-25 from the user's
`XCL-PS5-Proposal` and owned Minecraft Preview 1.18.3064.0 (Scarlett) package.

Verified on PS5 in this session: DualSense menu/character controls, loading the copied PC
world, creating a new offline world, and audible sound in both menus and gameplay.

The **TrinityXSX** entry in prospero-win now scans `C:\XCL\games` and can launch
Minecraft Preview from the DualSense. The PS5 session in
`ps5-logs/minecraft/library-ps5-first-run/` records the scan, XInput, the game
launch, world database activity and successful audio initialization. See the
[library setup](../standalone/README.md) for the FTP path and one-tile build status.

## Files

- `games/minecraft-beta/`: local working copy of the game; 17,484 files, excluding
  `gameos.xvd`. This directory is ignored by Git.
- `configs/minecraft.xcl.toml`: XCL configuration for the PS5.
- `configs/minecraft-beta.profile`: prospero-win profile (DualSense via XInput).
- `tools/ps5_debug.ps1`: logs, screenshots, world listings, configuration/runtime deployment.
- `tools/ps5_deploy_runtime.py`: runtime replacement with local and remote backups,
  SHA-256 verification before activation, and rollback if activation rename fails.
- `tools/test_minecraft_pc_vkd3d.ps1`: local game test; writes to
  `analysis/minecraft-pc/` by default. Windows remains a different host from PS5 Wine.
- `analysis/minecraft-proposal-import/`: preserved proposal, scripts, reports, and earlier logs.
  These are historical material, not the active configuration.
- `ps5-logs/minecraft/`: current PS5 captures and deployment backups.

The original Desktop folders were retained. No game data is tracked in Git.

## Input diagnosis (2026-11-25)

The PS5 screenshot `ps5-logs/minecraft/live/screen.png` shows the new **Create New World**
screen, before world generation. The captured log shows about 59.9 fps and button changes
reaching both GameInput and Minecraft's `OreUIGameControllerInputHandler::handleEvent`.
This rules out a complete loss of controller input in that captured run.

The running console binary differed from the current local release. In the captured run,
GameInput device arrival preceded the XUser device association callback. The local XCL
checkout at `93d8f3e` already has patch `0012`, which waits for association callback
delivery before asynchronous device arrival. This is a candidate fix for the new menu's
missing gamepad registration. After deployment, the PS5 log confirms association completion
before GameInput arrival (`assoc_delivered=true`) and calls to the previously missing
`registerGamepad` probe. The user confirmed that controls now work on Create New World.

The existing patch was built and deployed with SHA-256
`629deeca014230b9799dd95334278f91a016ee38a21f726b4abba582868994a0`.
The previous executable is backed up locally in
`ps5-logs/minecraft/deploy-20261125-090352-168043/` and remotely as
`C:\XCL\xcl.exe.previous-20261125-090352-812574`.
Restart the title through prospero-win to activate it.

Validation run in this session (Codex, GPT-6):

- `cargo test -p xcl --lib gameinput -- --test-threads=1`: 9 passed, 0 failed, 0 ignored.
- `cargo test -p xcl --lib hle::xgameruntime::tests -- --test-threads=1`:
  10 passed, 0 failed, 0 ignored. The junction case in `pls_used_does_not_follow_dir_links`
  could not create its junction in the sandbox and returned without covering that case.
- `cargo build -p xcl --release`: succeeded; existing compiler warnings remain.
- PowerShell syntax checks for both imported active scripts: passed.
- No full GPU suite or confirmed in-world PS5 run was performed at this point.

## World connection and sign-in investigation

After the input fix, creating a world produces `level.dat`, `levelname.txt`, and initial
LevelDB files. The screenshot `ps5-logs/minecraft/screenshots/12d_20261125_090532_00266406.png`
reports **Unable to connect to the world**, rather than showing a frozen generation screen.
The render loop continues at about 59.9 fps, and the captured runtime reports no direct-memory
allocation failure. The initial database log is empty. These observations do not yet establish
why the local server fails to start or connect. Logs: `ps5-logs/minecraft/world-load-1/`.

Disabling Multiplayer and LAN did not resolve the new-world error. A separate copy of an
existing PC world, named **PC Reference (XCL)**, was uploaded without changing the original.
That copy instead waits at **Signing in with your Microsoft Account** even after the user
disabled Multiplayer and LAN. This is a waiting screen, not evidence that credentials are
required for local play. Captures: `ps5-logs/minecraft/reference-world-1/` and screenshot
`ps5-logs/minecraft/screenshots/385_20261125_092909_00811193.png`.

The PS5 socket trace shows successful loopback UDP send/receive self-tests and successful
binds. LAN broadcast and IPv6 multicast errors also occur, but these alone do not establish
the cause. No network bypass or fabricated connection success was added.

### Standalone runtime comparison

The same current binary/configuration can enter the existing world on Windows with the
system Gaming Services runtime. Windows normally delegates XAsync and XTaskQueue to that
runtime; PS5 uses XCL's built-in implementation. Patch `0013` adds the opt-in
`XCL_FORCE_HLE_RUNTIME=1` comparison and bounded `XCL_TRACE_ASYNC=1` logging. Disabling the
system runtime on Windows reproduced stalled async operations: the XUser operation began,
but its work and completion callbacks were not delivered, and the world database was not
opened. Logs are preserved in `analysis/minecraft-pc-current/`:

- `system-runtime-world.log`: successful Windows reference.
- `hle-runtime-before-monitor-fix.log`: stalled built-in runtime before the fix.
- `hle-runtime-after-monitor-fix.log` and `hle-runtime-after-monitor-fix-report.json`:
  the fixed built-in runtime, 105-second run, exit code 0, 5,939 frames shown.

### Shared task queue monitor fix

XCL shared composite queue ports, but notified only the submitting queue's monitors.
A manual dispatcher waiting on another queue sharing the port could therefore remain
asleep. Patch `0014` notifies every attached queue with its own port index, retaining weak
queue references and releasing the registry lock before invoking callbacks. This follows
the shared-port notification behavior of Microsoft's public
[libHttpClient implementation](https://github.com/microsoft/libHttpClient/blob/main/Source/Task/TaskQueue.cpp).

The new regression test failed before the fix (missing parent notification) and passed
afterward. It checks submissions in both directions, swapped port indices, dispatch, and
unregistration. Validation in this session (Codex, GPT-6):

- `cargo test -p xcl --lib hle::xgameruntime::taskqueue -- --test-threads=1`:
  11 passed, 0 failed, 0 ignored.
- `cargo test -p xcl --lib xasync -- --test-threads=1`:
  15 passed, 0 failed, 0 ignored.
- `cargo test -p xcl --lib gameinput -- --test-threads=1` after the queue fix:
  9 passed, 0 failed, 0 ignored.
- `cargo build -p xcl --release`: succeeded with the existing warnings.

The fixed Windows built-in runtime now delivers XUser callbacks, queries the gamertag and
XGameSave interfaces, opens the world database, calls `registerServerInstance`, and progresses
to sustained world rendering. These stages were missing in the failing run. Both the Windows
reference and fixed comparison warn about pending GPU fences during the timed shutdown's
10-second drain; this remains a separate limitation. No full GPU suite was run.

The candidate runtime was deployed to PS5 with read-back SHA-256 verification:
`3efc5af286affb6aef30a241546f9a9ed48d90d2f65d25a43e9bc0e0619a61af`.
Backup: `ps5-logs/minecraft/deploy-20261125-094817-860868/`, remote previous file
`C:\XCL\xcl.exe.previous-20261125-094818-438487`. After restarting, the user confirmed that
**PC Reference (XCL)** loads on PS5 and the character responds to the DualSense. Capture:
`ps5-logs/minecraft/shared-monitor-world-success/`. The user also confirmed successful
creation of a new offline world and working character controls. Logs:
`ps5-logs/minecraft/new-world-audio-check/`.
Patches `0013` and `0014` were applied sequentially to copies of the XCL HEAD files and
verified to reproduce the working files; no commit was created.

The world diagnostic profile/configuration are preserved as
`configs/minecraft-network-debug.profile` and `configs/minecraft-world-debug.xcl.toml`.
Original remote files are backed up in `ps5-logs/minecraft/before-network-debug/`.
Normal logging settings were deployed and verified in
`ps5-logs/minecraft/normal-config-20261125-095229-912884/`; they take effect at next launch.

## Audio fix (verified on PS5)

After both world-loading scenarios worked on PS5, the user reported no sound. In the initial new-world
capture, prospero-win opens its audio sink and port successfully (`sink=1`, `status=ok`).
Only four silent grains are delivered (`audio=4`, `audible=0`), with no subsequent audio
progress during gameplay. Wine also logs `mmdevapi:stream_init Unknown flags: 01040000`.
FMOD loads and allocates SHAPE/XMA resources; that allocation alone does not prove the missing
SHAPE mixer is responsible, because FMOD may reserve these resources regardless of codec use.
The game includes Ogg Vorbis audio and its FMOD build includes a WASAPI output backend.
Tracing stream initialization and shutdown isolated the problem without changing volume
or implementing the SHAPE mixer.

The follow-up capture `ps5-logs/minecraft/audio-init-trace-2/session-3.log` narrows the
failure to FMOD's final `IAudioClient::Initialize`: stereo float32, 48 kHz, shared mode,
flags `0x01040000`. Wine rejects this request before creating the stream; FMOD immediately
stops/releases all three audio clients. Earlier probe clients with `0x00040000` initialize
successfully. Wine's pinned
[stream_init implementation](https://github.com/wine-mirror/wine/blob/490f6d5dcbb2a5047345b8af88d114bbcaad69a8/dlls/mmdevapi/client.c)
returns `E_INVALIDARG` for the additional flag. The PS5 `options.txt` has every game audio
volume set to 1, ruling out a saved in-game mute in this capture.

Patch `0015` installs an in-place hook on the system `IAudioClient::Initialize` vtable.
It calls the original first; only an `E_INVALIDARG` request containing `0x01000000` is
retried with that bit cleared. It returns the actual retry result. A host accepting the
original request is unchanged, and device/format errors and unrelated unknown flags
remain errors. The implementation module is pinned; setup COM references and the setup
apartment are released without starting a stream. Two existing windows-rs binding features
were enabled for the typed `IMMDevice::Activate` call; no dependency version changed.

Validation in this session (Codex, GPT-6):

- The new COM-vtable regression test failed with pass-through initialization, then passed
  with the retry. It checks argument preservation, existing host support, format/device
  error propagation, and rejection of other flags.
- `cargo test -p xcl --lib hle::wasapi -- --test-threads=1`: 1 passed, 0 failed, 0 ignored.
- `cargo test -p xcl --lib hle::xaudio2 -- --test-threads=1`: 9 passed, 0 failed, 0 ignored,
  no skipped cases reported.
- Patches `0013` through `0015` apply sequentially to copies of XCL HEAD and reproduce the
  changed working files. No commit or GitHub post was created.

The release build succeeded with the existing compiler warnings. A subsequent Windows
Minecraft run with the built-in runtime installed the WASAPI hook, loaded the existing world,
and rendered it through the end of the 80-second trial (4,514 frames shown, process exit 0).
The existing 10-second GPU-drain warning also occurs at this timed shutdown. Captures:
`analysis/minecraft-pc-current/wasapi-fix-world.log` and `wasapi-fix-report.json`. This Windows
run checks startup and world-loading regression; PS5 playback was subsequently verified below.

The runtime with the audio fix was uploaded with read-back SHA-256 verification:
`67396ba0a32fdedc7699f23350931cfaa4a58f21d2a835f42b56c8d816fee972`.
Backup of the previously working world-loading binary:
`ps5-logs/minecraft/deploy-20261125-100648-619312/`, remote previous file
`C:\XCL\xcl.exe.previous-20261125-100649-235614`.
Normal configuration/profile were restored and verified; the temporary audio trace profile
is retained in `configs/minecraft-audio-debug.profile`. Latest settings backup:
`ps5-logs/minecraft/normal-config-20261125-100654-969890/`.

After restarting, the user confirmed sound in both the menu and world. The final capture
`ps5-logs/minecraft/audio-world-success/` records successful WASAPI retry (`0x0`) and
13,247 delivered audio grains, including 5,557 with nonzero samples. The previous failure
stopped at four silent grains. The normal configuration is active and detailed tracing
is disabled. Verification covers local offline play.

## Commands (PowerShell)

```powershell
Set-Location D:\TrinityXSX
.\tools\ps5_debug.ps1 -Action Logs -OutTag input-test
.\tools\ps5_debug.ps1 -Action Screenshots
.\tools\ps5_debug.ps1 -Action Tail
.\tools\test_minecraft_pc_vkd3d.ps1
# Compare with the built-in runtime used when Gaming Services is unavailable:
.\tools\test_minecraft_pc_vkd3d.ps1 -ForceHleRuntime -TraceAsync
```

After rebuilding XCL, stage and deploy explicitly:

```powershell
Copy-Item .\xcl\target\release\xcl.exe .\dist\prospero-win\prefix\drive_c\XCL\xcl.exe
.\tools\ps5_debug.ps1 -Action DeployExe
```

Use `-Ps5Host` and `-FtpPort` when the console address differs from the local default
`192.168.1.170:2121`. The runtime deployment preserves the old file rather than
truncating a binary that Wine may still have mapped. Restart the game afterward.

The software probe offsets in `minecraft.xcl.toml` are specific to this executable:
Minecraft.Windows.exe SHA-256
`1fca6f125994703a77533c97fb35d5510cb40aa1743ec0be7e463cd0bfa49ede`.
