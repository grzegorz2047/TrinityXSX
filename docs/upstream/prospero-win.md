# Drafts for mpereiraesaa/prospero-win

Status: drafts only. Nothing here has been posted. Fill in the `<...>` fields (firmware, loader)
before posting.

prospero-win's issue forms (`.github/ISSUE_TEMPLATE/`) are "Runtime or installation bug" (title
prefix `[Bug] `) and "Game compatibility report" (title prefix `[Game] `). Blank issues are off.
The drafts below follow those forms' fields. CONTRIBUTING.md asks for exact artifact identity,
reviewed logs (no raw transcripts committed), and no general claims beyond the evidence.

Common to all drafts:

- prospero-win **v0.3.0** release zip (`PW_REPORT/1 build=404cac6ebbb236d2486d2796fd28b9bd39f2caa6`).
  PS5 firmware `<firmware>`, loader `<loader>`.
- The program is [XCL](https://github.com/xcl-dev/xcl) (Xbox Series X|S → Windows compatibility
  layer, a 64-bit Windows program) at `20cf130` with the patches in
  [TrinityXSX](https://github.com/grzegorz2047/TrinityXSX), running **its own test title**
  (`xcl_testtitle.exe`, built from XCL's sources). No game files were used.
- Direct3D 12 is vkd3d-proton 3.1 (built from source with TrinityXSX's patches), `dxgi.dll` /
  `d3d11.dll` are DXVK 3.1.1 (upstream release), all native from the program folder `C:\XCL`.
- All runs used the `default` prefix (`/data/prospero-win/prefix`).

Order: 1 is the most useful to upstream; 4 is small.

---

## 1. `[Bug] ` Writes to a writable page beside read-only pages end in an access violation in VirtualAlloc'd memory (patch 0974's hot-write report)

**Release/source commit and firmware**

v0.3.0 (`404cac6e`), which contains Wine patch 0974. Firmware `<firmware>`.

**Steps to reproduce**

XCL loads a PE image itself: it reserves the image with `VirtualAlloc` (a `valloc` view, not
`SEC_IMAGE`), copies the sections and sets each section's protection with `VirtualProtect` at
4 KiB granularity. In the test title the layout is:

| Image offset | Section | Protection |
|---|---|---|
| `0x5C000`–`0x7DFFF` | `.rdata` | `PAGE_READONLY` |
| `0x7E000`–`0x7EFFF` | `.data` | `PAGE_READWRITE` |
| `0x7F000`–`0x83FFF` | `.pdata` | `PAGE_READONLY` |

The 16 KiB host page at `+0x7C000` holds the last 8 KiB of `.rdata`, all of `.data` and the first
4 KiB of `.pdata`. The title then runs and writes to its `.data`.

**Expected and observed behavior**

Expected (Windows): writes to `.data` never fault; it is `PAGE_READWRITE` and protections are per
4 KiB page.

Observed: a few checks into the run, the process ends with an unhandled write fault. The reported
address is not the address written: it is the first page of the host page, which is a read-only
`.rdata` page. Image base `0x1001580000`, so the host page is `0x10015FC000` and `.data` is
`0x10015FE000`.

From a run with `winedebug = err+all,+loaddll,+virtual` (session log, `REC` prefixes removed):

```
0024:trace:virtual:NtProtectVirtualMemory ffffffffffffffff 10015dc000 00022000 00000002
0024:trace:virtual:NtProtectVirtualMemory ffffffffffffffff 10015fe000 00001000 00000004
0024:trace:virtual:NtProtectVirtualMemory ffffffffffffffff 10015ff000 00005000 00000002
...
0024:trace:virtual:NtProtectVirtualMemory ffffffffffffffff 1001604000 00001000 00000002
0024:trace:virtual:dump_view View: 1001580000 - 1001604fff c-rw- (valloc)
0024:trace:virtual:dump_view       1001580000 - 1001580fff c-r--
0024:trace:virtual:dump_view       1001581000 - 10015dbfff c-r-x
0024:trace:virtual:dump_view       10015dc000 - 10015fdfff c-r--
0024:trace:virtual:dump_view       10015fe000 - 10015fefff c-rw-
0024:trace:virtual:dump_view       10015ff000 - 1001604fff c-r--
...
wine: Unhandled page fault on write access to 00000010015FC000 at address 00000010015CA385 (thread 0058), no debugger, ending the process
```

XCL's own handler saw the same address in the exception record
(`unhandled exception 0xc0000005 in xcl_testtitle.exe+0x4a385 (write at address 0x10015fc000)`).
A run without `+virtual` (another build of the title) failed the same way:
`Unhandled page fault on write access to 00000010015FC000 at address 00000010015CA4BF`.

What we think happens (reading patch 0974 at `404cac6e`, not verified on the console):

- `get_host_page_unix_prot()`: outside images, a read-only committed page takes `PROT_WRITE` away
  from its whole host page. So every write to `.data` faults on the host.
- `virtual_handle_fault()`: the page itself allows the write, so 0974 steps over it. That part
  works for XCL-loaded images. In the run without `+virtual`, before the title started,
  `fault_top` shows 40 resolved (`result=1`) write faults on a writable page of `testgame_mw.dll`,
  which XCL also loads itself at `0x180000000` (size `0x1f000`):
  `fault_top rank=1 count=40 page=0x18001c000 pc=0x18000100a kind=1 vprot=0x23 host_vprot=0x23 result=1`
- `subpage_hot_write()`: the 256th such write in one host page (`SUBPAGE_HOT_STEPS`; the first 255
  are stepped) is not stepped but reported as a write to the first read-only page of that host
  page (`rec->ExceptionInformation[1] = protected_page`, scanning from the host page start). For this host page that is `0x10015FC000`, which is exactly the
  reported address. The 0974 commit message says this is meant for programs that write-protect
  pages to learn when they change (DuckStation). A program that does not do that gets an access
  violation it cannot handle.

We also considered the kernel rounding the fault address down to 16 KiB. Other `fault_top` records
from the same console show pages that are 4 KiB but not 16 KiB aligned (for example
`page=0x1420041000`), so the kernel does not round all fault addresses. We did not count the
stepped writes on the title's host page. The process ended about 50 ms after XCL started the
title's entry point (both runs), before the next `fault_top` record.

PR #739 (patch 0986, merged after v0.3.0) keeps this report for writes beside a read-only page, so we
expect the same result on `main`. Not tested.

Who else this can hit: a program that sets 4 KiB protections inside its own `VirtualAlloc` memory,
with read-only pages next to data it writes often. Programs that load PE images themselves are
one example. Images mapped by Wine (`SEC_IMAGE`) keep the union and are not affected.

Workaround on the program side (now in XCL, TrinityXSX `patches/xcl/0003`): under Wine, assume
16 KiB host pages and give every 4 KiB page in a host page the union of the protections of the
sections in it, as Wine already does for images. With that change the test title runs to the end
on the console in every later run (seven sessions: 47 to 51 checks pass). The checks that failed
there are the window-cloaking check and two file-overlay checks; none was an access violation.

Possible directions, for you to judge:

- a profile switch that turns off the hot-write report (or 0974) for one program;
- a note in `docs/WINE_PS5_BUILD.md` next to 0974/0986 for programs that load PE images themselves.

A host-side check that might reproduce it (from reading the code, not run):
`VirtualAlloc` 16 KiB `PAGE_READWRITE`, `VirtualProtect` the first 4 KiB `PAGE_READONLY`, then
write to the third 4 KiB page 300 times with `WINE_HOST_PAGE_SIZE=16384` (a case that could be
added to `tests/wine_subpage_protect.c`, which `tests/test_wine_subpage_protect.py` runs). On
Windows no write faults.

**Live log or installation error**

Excerpts above. Full session logs can be provided after review.

---

## 2. `[Bug] ` The `default` prefix cannot be made on the console; a missing prefix shows only as `could not load kernel32.dll`

**Release/source commit and firmware**

v0.3.0 (`404cac6e`). Firmware `<firmware>`.

**Steps to reproduce**

1. Install v0.3.0 and open it once.
2. Copy only a program folder into `/data/prospero-win/prefix/drive_c/XCL/` and a profile with
   `prefix = default` into `/data/prospero-win/profiles/`. No Wine prefix was made beforehand.
3. Start the profile from the launcher.

**Expected and observed behavior**

Observed (two runs, same result). The title creates `windows/temp`, finds the working directory,
then Wine stops right after `xcl.exe` and `ntdll.dll`:

```
PW_WINE64 profile id=xcl-testtitle prefix=/data/prospero-win/prefix desktop=1920x1080 ...
PW_WINE64 prefix temp folders=1 last=windows/temp
PW_WINE64 cwd=C:\XCL host=/data/prospero-win/prefix/drive_c/XCL status=0
WINE 0024:err:environ:run_wineboot failed to start wineboot c00000bb
WINE 0024:trace:loaddll:build_module Loaded L"C:\\XCL\\xcl.exe" at 0000000140000000: native
WINE 0024:trace:loaddll:build_ntdll_module Loaded L"C:\\windows\\system32\\ntdll.dll" at 0000008FFF9C0000: builtin
WINE wine: could not load kernel32.dll, status c0000135
PW_WINE64 exit sink_calls=12
PW_WINE64 restart to=launcher cycle=1 reason=wine-exit eboot=/app0/eboot.bin
```

This matches the docs. Patch 0550 stops process creation, so wineboot cannot run (`c00000bb`;
this line is also in every working run). `docs/WINE_PS5_BUILD.md` says the prefix is
initialised on the host and that Wine needs the prefix's `system32` copies. After a prefix made
by Wine on a PC was put at `/data/prospero-win/prefix`, the same profile started and ran.

What would have helped:

- The title could check for the prefix before starting Wine (for example `system.reg` or
  `drive_c/windows/system32/kernel32.dll`). It already reads `user.reg` there for the temp
  folders. It could log something like `PW_WINE64 prefix not initialised` and show it in the
  launcher. Today the only sign is the `kernel32.dll` line and the return to the launcher.

Questions:

- Could the release ship a template `default` prefix, or could the title fill an empty one from its
  own `win/wine/lib/wine/x86_64-windows` and pre-made registry files, as it already copies
  `wow64native.dll` and the Zink provider into prefixes? We have not checked what besides
  `system32` and the registry a usable prefix needs on the console.
- For users with only a Windows PC: `GETTING_STARTED.md` says a VM or WSL works. Is that the
  recommended path for the `default` prefix too (`tools/build_host_wine.sh` under WSL)? PR #748
  (open) adds a Windows importer but still starts from a template made on Linux. We have not
  tried WSL.

**Live log or installation error**

Excerpt above (`/data/prospero-win/logs/session-N.log`).

---

## 3. `[Game] ` XCL test title: 64-bit Direct3D 12 program with vkd3d-proton 3.1 and DXVK 3.1.1 `dxgi` from the program folder, all 51 checks pass and 602 frames presented in 10 s

Not a game. It is a test program that exercises Direct3D 12, XAudio2, file I/O and threads. Filed as a
compatibility note for 64-bit D3D12 programs that bring their own vkd3d-proton.

**Game and edition/version**

XCL test title (`xcl_testtitle.exe`, XCL `20cf130` + TrinityXSX patches), run as
`xcl.exe run --in-process testtitle -- --hello --present-seconds 10`.
`--in-process`: XCL runs the title inside its own process, because the console has no second
process.

**prospero-win release or source commit**

v0.3.0 (`404cac6ebbb236d2486d2796fd28b9bd39f2caa6`).

**PS5 firmware and homebrew loader**

`<firmware>`, `<loader>`.

**Profile name/revision and changes**

```ini
[application]
id = xcl-testtitle
name = XCL test title
executable = C:\XCL\xcl.exe
working_directory = C:\XCL
arguments = run --in-process testtitle -- --hello --present-seconds 10
dll_overrides = d3d12,d3d12core,dxgi,d3d11=n
prefix = default
runtime = wine-wow64
architecture = pe64
graphics = dxvk

[display]
desktop = 1920x1080

[input]
mode = xinput

[debug]
winedebug = err+all,+loaddll
env = VKD3D_DEBUG=warn
env = VKD3D_LOG_FILE=C:/XCL/vkd3d.log
env = DXVK_LOG_PATH=C:/XCL
```

`d3d12.dll` and `d3d12core.dll` (vkd3d-proton 3.1, release build with `-Db_ndebug=true`), plus
DXVK 3.1.1 `dxgi.dll` / `d3d11.dll`, are in `C:\XCL` next to `xcl.exe`. The package puts nothing
into the prefix's `system32` and sets no registry overrides. With `dll_overrides`, `d3d12.dll`,
`d3d12core.dll` and `dxgi.dll` load as native from `C:\XCL` (`+loaddll`).

**What you tested**

- Start to exit, unattended: one run on the final build (the results below), plus earlier runs
  while fixing the program.
- Device: `Found device: PS5 (radv 26.2.0)`. XCL got a D3D12 device at feature level 11_0
  (`PS5 (8160 MiB VRAM), feature level 11_0`). vkd3d-proton refuses 12_0
  (`Feature level 0xc000 is not supported`), as `docs/WINE_PS5_BUILD.md` describes.
- The test title's 51 checks all pass, including: a triangle rendered on the GPU and read back,
  sparse (tiled) buffer heaps and uploads, raytracing feature and descriptor queries (`OPTIONS5`,
  acceleration-structure SRV), graphics memory in a GPU-visible region, DirectStorage reads,
  XAudio2 voices, Media Foundation device manager, file overlay, threads with static TLS, and C++
  exceptions. Before the checks, XCL's import of host memory into a Vulkan buffer
  (handle type `0x80`, host allocation) reported `UMA import check: ok`.
- Presentation: `xcl_testtitle: presented 602 frames in 10.0 s`. XCL's log has
  `600 frames presented, 59.9 fps over the last 300`. The swap chain is 640x360. The console's own
  recorder showed the frames filling the whole screen and changing colour as the title cleared
  them, ending with a clean return to the launcher (`display quiesce reason=wine-exit status=0`).
- Memory: `dmem_peak` about 800 MB, no `dmem_failures`.

**Failure or remaining limitations**

- Not tested: controller or keyboard input in the program, audible sound, a run longer than
  about 15 s, frame pacing on the display (the 60 fps figure is the program's own present count),
  and `show_fps` (the docs say D3D12 programs have no DXVK HUD).
- Audio: `audio=25` grains with `audible=0`. XCL's XAudio2 check passed, but we did not check
  whether sound should have been heard. Not investigated.
- vkd3d-proton 3.1 disables mesh shaders on this RADV:
  `Mesh shaders are supported, but not barycentrics. Disabling mesh shaders as a global UE5 workaround.`
  `VK_EXT_mesh_shader` is there, `VK_KHR_fragment_shader_barycentric` is not. A program that needs
  mesh shaders needs vkd3d-proton's per-application opt-in. TrinityXSX patches vkd3d-proton for
  `xcl.exe`.
- RADV's `maxBufferSize` is 4 GiB − 4. vkd3d-proton warns about, and still creates, 4 GiB
  buffers.
- The program's own top-level window: `DwmSetWindowAttribute(DWMWA_CLOAK)` is a stub in Wine
  (`fixme:dwmapi:DwmSetWindowAttribute (..., d, ...) stub`). `view = window` showed XCL's
  uncloaked window for about half a second before the first frame. XCL now moves that window off
  the virtual screen under Wine.
- Problems found on the way are in drafts 1 and 2.

**Diagnostic attachment**

`<attach reviewed session-N.log of the final run>`

---

## 4. `[Bug] ` `alive` does not count Direct3D 12 / Vulkan swap-chain presents

Small; could be folded into another issue.

**Release/source commit and firmware**

v0.3.0 (`404cac6e`). Firmware `<firmware>`.

**Steps to reproduce**

Run a 64-bit program that presents through a Vulkan swap chain (here vkd3d-proton + DXVK `dxgi`)
for 10 s.

**Expected and observed behavior**

The program presented 602 frames in 10 s, and the capture shows them on screen. The `alive`
records stay flat:

```
PW_WINE64 display released to vulkan close=ok shown=0
PW_WINE64 vulkan frames idle=1 show=1 buffer=1
PW_WINE64 loading done by=frame spinner_steps=57
PW_WINE64 alive tick=360 ... frames_put=5 shown=0 ... vk_shown=2 vk_busy=0 ...
...
PW_WINE64 alive tick=900 ... frames_put=9 shown=0 ... vk_shown=2 vk_busy=0 ...
```

From `native/wine64_main.c`, `vk_shown` counts GDI frames and spinner frames that the title shows
through `pw_videoout_show_tiled` while no swap chain presents. It does not count swap-chain
presents. `docs/DEBUGGING_GUIDE.md` reads `alive` as "Is it drawing (`frames_put`, `vk_shown`)",
and `docs/TELEMETRY.md`'s acceptance rule asks for "rising shown frames". A working D3D12 or
Vulkan run therefore looks like a stalled one in the log.

A present counter in `alive` would help, or a sentence in both docs saying that swap-chain
frames are not counted there. We did not try `PW_VK_BATCH_STATS=1`
(`PW_VK_PRESENT_INTERVAL` records), which may already cover this.

**Live log or installation error**

Excerpt above.

---

## Seen in the logs, not filed

These lines were checked against the prospero-win source and docs. They look expected, cosmetic, or
upstream Wine. They are listed so nobody files them again.

- `err:environ:run_wineboot failed to start wineboot c00000bb`: in every run. It is by design
  (patch 0550).
- `PW_WINE64 runtime not at /app0/win/wine/lib/wine/x86_64-unix`, followed by
  `ntdll=/mnt/sandbox/PPSA99995_000/app0/...`: the title tries `runtime_roots[]` in order, and the
  second root is the one used. Harmless. `WINE_PS5_BUILD.md` still says ntdll is loaded from
  `/app0/...`.
- `PW_WINE64 cpu=translator prefix_cpu=-2` for an `architecture = pe64` profile: the record
  describes a 32-bit program's CPU, so the value means nothing for a 64-bit one.
- The program's own stderr, and Mesa's (`ac_compute_device_uuid's output is based on invalid pci
  bus info.`), arrive as `WINESERVER` records, because fd 2 is captured with that label.
- `pw_wine_dl: cannot open .../x86_64-unix/dnsapi.so, tried .../dnsapi.prx /app0/sce_module/dnsapi.prx: not found`,
  then `err:dnsapi:DllMain No libresolv support, expect problems`: dnsapi's Unix side is not in the
  package, so DNS queries through `dnsapi.dll` would fail. Same pattern as #395 (DirectWrite). Our
  program does no DNS lookups, so this was not tested.
- `fixme:thread:get_thread_times not implemented on this platform`: per-thread CPU times, from
  upstream Wine. Impact not tested.
- `fixme:vulkan:win32u_vkCreateBuffer Unsupported handle types 0x80` (host-allocation handle
  type): XCL's host-memory import check still passed (`UMA import check: ok`).
- DXVK's `readMonitorEdidFromKey: Failed to get EDID reg key size` and `Failed to parse display
  metadata`: no EDID in the registry. DXVK uses blank metadata.
- `err:wgl:internal_context_create Failed to create internal thread context`: in every run after
  `opengl32` / `wined3d` load. We did not find what loads them. No effect seen.
- Session logs keep the newest 2 × 1 MB. With `+virtual` the start of the run is lost. This is
  documented in `GETTING_STARTED.md` ("Getting a log").

## Notes for TrinityXSX (not for posting)

- `patches/xcl/0003`'s message and `docs/STATUS.md` say that the last VirtualProtect on a host page
  wins and that Wine reports the 16 KiB-aligned host page as the fault address. The prospero-win code
  (patch 0974) and the logs fit draft 1 better:
  - a read-only page takes writing away from its whole host page (outside images);
  - writes to the writable neighbour are single-stepped;
  - the 256th such write is reported at the host page's first read-only page.

  The last call on that host page was `.pdata`'s, not `.rdata`'s, and both are read-only, so the run
  cannot tell "last wins" from "read-only wins". The XCL fix (union per host page) is right either
  way. Consider rewording the patch message before sending it upstream.
- Firmware and loader are not recorded in the logs or in `docs/STATUS.md`. Add them before posting.
