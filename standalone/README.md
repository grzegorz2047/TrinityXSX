# TrinityXSX standalone prototype

The catalog and staging tools run on the build PC with Python 3.12 or newer.
They use the standard library and write the profile format that prospero-win
already reads. Run the commands below from the repository root.

```powershell
python -m standalone.catalog games
python -m standalone.stage games\minecraft-beta dist\standalone\PPSA83601-data
python -m unittest standalone.test_standalone -v
```

The catalog scans immediate child directories, reports rejected entries and gives
relative paths to available cover and splash images. The stage command selects the
tested Minecraft Preview 1.18.3064.0 preset, writes one profile and `profiles.lst`,
and copies the XCL runtime into the console prefix layout. It leaves an empty game
directory for the local copy. `TRINITYXSX-BUILD.json` records what the stage contains
and the SHA-256 of each XCL runtime binary. Output must be a new directory.

## Game library on PS5

XCL patch `0016` adds the **TrinityXSX** library screen. It scans immediate
children of `C:\XCL\games` at launch. The corresponding FTP directory for the
current test installation is `/data/prospero-win/prefix/drive_c/XCL/games/`.
Each game folder needs a `MicrosoftGame.config` and the Scarlett executable
named by that file. Leave the game data in that folder.

In the library, use the D-pad to select a game, Cross to play, Triangle to
rescan and Circle to return to the host menu. The first launch writes an offline
PS5 config for the title. Minecraft Preview gets the tested user ID and memory
settings, which also lets the existing tested install use its current save
location. The library uses the existing `xcl.toml` when one is present.

For a PC catalog check, run `xcl.exe library <games-folder> --list`. For a PS5
test with the existing prospero-win installation, build XCL with patch `0016`
and run `python -m standalone.deploy_library <PS5-host> <built-xcl.exe>`. The
installer publishes a separate **TrinityXSX** profile after verifying both files
and keeps a backup of the profile index. The existing Minecraft profile remains
available. Use `--replace` to update the library executable in place.

On 2026-11-25 the PS5 displayed the library, detected two game folders and
launched Minecraft Preview from the DualSense. The captured session also shows
the game opening a local world's database, receiving controller input and
initializing the audio stream. The tested library executable has SHA-256
`3e6331024f35b50e08c2d952c5dbadb3a8a339ce035ff7816743f19ccc80e565`.

## One-tile package

The image includes the TrinityXSX entry point, a runner based on prospero-win
v0.3.0, Wine modules and a clean prefix. Build `01.000.011` uses
`/mnt/ext1/TrinityXSX-PPSA83601`. On first launch the entry point checks the
owner of a new directory and prefix before copying data. It copies the
existing profiles and prefix from `/data/homebrew/PPSA83601` when available,
or initializes them from `/app0/data-seed`. It then starts
`/app0/runtime/pw_runner.bin` with `profile=trinityxsx-library`.
The library reads game folders under
`/mnt/ext1/TrinityXSX-PPSA83601/prefix/drive_c/XCL/games/`.
Closing the library or game returns to the PS5 home screen.

The launcher links the native `libkernel.sprx` provider explicitly. It logs
to `/data/homebrew/PPSA83601/startup.log` or to `launcher.log` in the new
directory. Wine records `startup.log` and `wineserver-startup.log` in the
new directory. The PS5 must expose `/mnt/ext1` to the title for this layout
to work.

For a local rebuild, use Clang 18, LLD and the published PS5 SDK. The launcher
and runner builds use the native application boilerplate's startup and image
tools at `38c9175869f101c8ab7f2c14195c69c5e581e905`. The runtime files come
from the verified prospero-win v0.3.0 archive.
Apply `patches/prospero-win/0001-Use-the-title-sandbox.patch` to the matching
runner source before building it. Use the clean Playden Kit 0.36 bottle
template as the prefix seed.

```powershell
standalone\native\build_launcher.ps1 -Foundation <foundation> -Sdk <sdk> `
  -LlvmBin <llvm-bin> -NativeTool <ps5-native-tool.exe> `
  -Output dist\standalone\native-build\eboot-seeded.bin -WithSeed

python -m standalone.native.build_runner <patched-source> <foundation> <sdk> `
  <llvm-bin> <ps5-native-tool.exe> <prospero-win-v0.3.0.zip> dist\standalone\native-build\runner

python -m standalone.stage dist\standalone\PPSA83601-seed `
  --library-exe xcl\target\release\xcl.exe --prefix-seed <clean-console-prefix>
```

### Image installation matching TrinityX360

The TrinityX360 reference is the `release/source-1.0.6` branch at `2c5cdc5`.
The local TrinityX360 `main` matched remote `main` at `0dc7bfd`.
The console copy on 2026-11-25 was `/mnt/ext1/PPSA83600.ffpkg`, with writable
state in `/data/homebrew/PPSA83600`. The installed image and the release script
agree on UFS2 with 64 KiB blocks, 8 KiB fragments and the Title ID as volume label.
Its shell metadata reports `01.006.000`, while the internal `VERSION` is `1.0.2`
and the image includes candidate manifests. The exact installed build commit is
therefore unverified. Use the observed layout as the packaging reference.

For the matching TrinityXSX layout, build the image with the launcher as
`eboot.bin`, the runner as `runtime/pw_runner.bin` and the runner's bundled
Wine modules at `win/wine/`. The image builder sets the tile name to TrinityXSX.
It uses the project tile artwork in `assets/icon0.png` as `sce_sys/icon0.png`.
The console image destination is `/mnt/ext1/PPSA83601.ffpkg`.

```powershell
python -m standalone.image dist\standalone\native-build\runner dist\standalone\PPSA83601.ffpkg `
  --launcher dist\standalone\native-build\eboot-seeded.bin `
  --data-seed dist\standalone\PPSA83601-seed `
  --content-version 01.000.011 `
  --ufs2tool <UFS2Tool.exe-or-dll>
```

The reference UFS2Tool revision is `b5307a60d5b4e3a68ba680e0e33cfadf05017c77`.
The image builder checks the title ID, sets executable permissions and runs
`fsck_ufs -n` before publishing the image.

The November 25, 2026 test build was uploaded to `/mnt/ext1/PPSA83601.ffpkg`,
verified by a full FTP readback and registered by ShadowMountPlus as TrinityXSX.
Diagnostic build `01.000.004` uses the same destination. Its verified SHA-256 is
`3cb877f3ad31d6ac424b854727b59041a25a823ef2995efdd994317c147169bd`.
That build loads `ntdll.prx`, initializes the Wine environment and reaches
`server_init_process`, where Wine exits before the catalog opens. See the
console capture at `ps5-logs/trinityxsx-startup-text/startup.log`.
The `01.000.005` experiment with modules from the existing prospero-win
installation failed at `ntdll.prx` with `0x80020008`. The active image was then
restored to `01.000.004`.

### Wine server startup on this PS5

The FTP capture at `ps5-logs/trinityxsx-server-diagnosis/startup.log` records
successful native module loading, environment setup and thread startup. Wine's
last startup marker is `server_init_process`. The working prospero-win capture
at `ps5-logs/minecraft/library-ps5-first-run/session-1.log` reports a change in
runtime permissions (`elevated_before=0`, `elevated_after=1`) and then
`server connected` followed by the XCL library window.

[Upstream's data directory notes](https://github.com/mpereiraesaa/prospero-win/blob/main/docs/WINE_PS5_BUILD.md#data-directory)
document the same stop when a title can see `/data` but `lstat()` returns
`EPERM`. This is the leading diagnosis for the standalone title. Its capture
contains the startup marker but lacks the failing filesystem call's return
code. The source candidate in
`patches/prospero-win/0002-Inspect-server-directories-by-descriptor.patch`
changes Wine's two server directory checks to use `open()` and `fstat()` on
PS5. It rejects a symbolic link at the final name and keeps the existing
directory creation, ownership and identity checks. Apply it to prospero-win
v0.3.0 before building Wine's PS5 modules with `tools/build_wine_ps5.sh`.
Pass `--wine-build <Wine-PS5-build-directory>` to
`standalone.native.build_runner` to include the rebuilt `wineserver.prx` in
version `01.000.006`. The runner builder checks the Wine revision, the patch,
the module's import report and its SHA-256 before replacing the bundled file.
On PS5, `01.000.006` still exited after `server_init_process`. The capture is
`ps5-logs/trinityxsx-wineserver-descriptor/startup.log`. The exact failing
filesystem operation or server initialization step is still unknown.
`patches/prospero-win/0003-Log-Wine-server-startup-errors.patch` adds a
diagnostic build `01.000.007` that records the fatal server message to
`/data/homebrew/PPSA83601/wineserver-startup.log` when using the shared mount.
The diagnostic console run reported `/data/homebrew/PPSA83601/prefix is not
owned by you`. FTP listed UID 0 on the existing prefix and UID 1 on files
created by TrinityXSX.

The `01.000.008` trial stopped with `EACCES` before Wine started. In
`01.000.009` the console logged `prefix-staging-owner status=13` before
the launcher copied files. The new directory on the shared data mount
failed the owner comparison after creation and inspection. That led to the
external storage layout in `01.000.011`.

Console acceptance starts with the new home screen tile and a clean prefix.
Verify the direct start into the library, creating a world offline, DualSense
controls, audio, closing to the PS5 home screen, and reopening the saved world
after a relaunch. The PC tests use fixture runner files to check assembly and
refusal of a wrong Title ID.

### Tile artwork

The built-in ImageGen tool generated the source artwork. The PS5 asset is a
512×512 RGB PNG at `assets/icon0.png`.

```text
Use case: logo-brand. Create a square app icon for the PS5 home screen tile of TrinityXSX, a game library. The only text is exactly “XSX” (X, S, X), drawn as a continuous custom angular monogram with extreme legibility at 64 pixels. Make the two X glyphs clear and the middle S unmistakable. Thick balanced geometric strokes with precision beveled cuts, left-to-right ice-mint, warm white, and cool mint accents. Center the mark in the safe inner 75% of the canvas on a full-bleed matte charcoal-blue background (#101a20) with a subtle deep-teal glow behind it. Modern console UI, restrained details, excellent silhouette and ample edge padding. Render artwork straight-on as the finished square icon, no rounded mask, frame, mockup, extra text, real console logos, or watermark.
```

## Licensing

The native launcher source (`launcher.c`, `prefix_seed.c`, `crt0.c`, `launcher.ld`),
packaging scripts and documentation in this directory use the [MIT License](LICENSE).
The standalone `eboot.bin` launcher links only this MIT C startup code and the PS5 SDK
system stubs — it does not link GPL-3.0 startup code. The image packages the launcher
and runner as separate executables which communicate through launch arguments
(`sceSystemServiceLoadExec`). The separately built native runner (`runtime/pw_runner.bin`)
links the boilerplate's GPL-3.0-or-later startup code and retains its corresponding
source notices. The current library UI is compiled into XCL and distributed under XCL's
GPL-2.0-only terms. Generated folders retain the terms of their inputs, including the
GPL-2.0-only XCL binary and the runner's separately licensed components (LGPL-2.1 Wine /
vkd3d-proton, zlib DXVK). Before public redistribution, verify the terms of the exact
runtime inputs and provide their required notices and corresponding source. The private
test stage is not a release bundle.
