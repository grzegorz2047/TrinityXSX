# XCL (xcl-dev/xcl): drafts of a test report and six PR descriptions

These are drafts only. Nothing here has been posted. They follow xcl's `CONTRIBUTING.md` and `AGENTS.md`
(PR outline, size limits, required GPU suite run and `scripts/smoke.ps1` regression run, test reports in
#42, AI-use disclosure).

## Before anything is posted

- **No smoke run.** `scripts/smoke.ps1 -Game <folder>` needs a retail title, and none was used. Every
  result below comes from xcl's own test title (`tests/testtitle`) and the test suite. Five of the six
  patches change `crates/xcl/src` outside tests, so `scripts/pr_check.py` refuses them without two
  `xcl-smoke` blocks (main and the PR's head). They have to stay **drafts** until someone who owns a
  title runs `smoke.ps1` on `main` and on each PR's head and pastes both blocks. Do not write a smoke
  block by hand. 0005 only touches `tests/testtitle/` and does not need a smoke block.
- **Rebase first.** All counts below were measured on `main` at `20cf130` (2026-11-25 00:08 +0100).
  `gh api repos/xcl-dev/xcl/compare/20cf130...main` reports `main` 177 commits ahead of that
  (queried 2026-11-25; the number keeps growing). Each
  branch has to be rebased, and the whole suite rerun on the rebased head, before its PR leaves draft.
  The counts in **Tested** then have to be replaced with the new ones.
- **Driver.** The runs below used NVIDIA 610.62. That is what DXVK's log reported at 01:37, and the
  current driver was installed at about 05:28 (Windows' PnP events), after the last run cited here
  (04:50). The machine now has 621.40 (DXVK log at 05:32). Nothing below was rerun on 621.40.
- **Stale commit-message lines.** 0004 and 0006 end with "Not yet run on the PS5". The PS5 logs show
  runs with both (see their **Tested**). Amend those bodies before pushing.
- **Overlapping upstream PRs (open), found with `gh pr list --search`.** Each one is named in the PR it
  touches below. 0006 overlaps #794, #1462 and #1481. 0002 touches the same function as #1677. 0003
  touches the same lines of `map_image_ex` as #1416. CONTRIBUTING asks for a short plan comment before
  anything bigger than a small fix. For the Wine/PS5 ones (0001, 0003, 0004, 0006), #175 (Wine +
  vkd3d-proton) is the closest existing issue.
- **Author.** CONTRIBUTING requires that the person who opens the PR has read every line of the diff
  and can answer for it. The **AI use** sections below say what the tool did. They do not replace that.

### Split and order

Each patch is already its own branch, based directly on `main` `20cf130`. Each branch's diff is
identical to the corresponding commit of the stack `ps5/test-package`, which was checked by
comparing the diffs (for 0006 only the blob ids of `tests/testtitle/src/main.rs` differ, because
0005 is not under it). No PR depends on another, so none needs `Stacked on #N`.

| Patch | Branch | Area | Changed lines | Runtime (smoke needed) |
|---|---|---|---|---|
| 0001 | `ps5/run-host-in-process` | `xcl run` | +112 / −4 | yes |
| 0002 | `vulkan/mfplat-skip-d3d11-bridge-on-vkd3d` | HLE (mfplat) | +55 | yes |
| 0003 | `ps5/pe-protect-host-pages` | PE loader | +130 / −6 | yes |
| 0004 | `ps5/device-lower-feature-level` | D3D12.X | +73 / −39 | yes |
| 0005 | `ps5/testtitle-present-seconds` | Tests | +67 / −23 | no (`tests/testtitle` only) |
| 0006 | `hle/title-window-off-screen-without-dwm` | HLE (user32) | +101 / −12 | yes |

Suggested order:

1. 0005 (test-only, no smoke needed).
2. 0001 and 0003. They are independent of each other, and both are what the PS5 needs to start the
   test title at all.
3. 0002, 0004 and 0006, in any order. They are independent of each other.

0005 and 0006 both edit `tests/testtitle/src/main.rs`, in different hunks. The two branches merge
without a conflict (`git merge-tree`).

---

## (a) Test report: comment for #42

````markdown
**RTX 3060 Laptop GPU (wave32), driver 610.62: `main` 20cf130 and six patches on it**

**Tested**
- NVIDIA GeForce RTX 3060 Laptop GPU (XCL: "5994 MiB VRAM"), driver 610.62. There is also an Intel UHD
  iGPU, and XCL picked the NVIDIA adapter (`D3D12.X: backend 'real' – NVIDIA GeForce RTX 3060 Laptop GPU`).
  Intel Core i5-12450H. Windows 11 Home build 26200. Rust 1.100, stable `x86_64-pc-windows-msvc`. Debug
  build. No `XCL_SAMPLE_*` variables set. DXC as logged by the suite: dxcompiler 1.9.2602.17 (21d28f72),
  dxil 1.9.2602.17.
- `main` at `20cf13022f764208384c610c2ae6a3f4eed201a2`:
  - `cargo test --workspace -- --test-threads=4` stops after the lib binary:
    **926 passed, 1 failed, 25 ignored** (lib). The failure is
    `hle::d3d12x::bind::tests::execute_indirect_reads_the_arguments_written_before_it` (below).
  - The same run with that test filtered out (`--skip execute_indirect_reads_the_arguments_written_before_it`;
    the lib result shows `1 filtered out`): **975 passed, 0 failed, 45 ignored**, summed over all
    test-result blocks.
- `main` + six patches (`905fe08`; the PRs listed at the end), `cargo test --workspace --no-fail-fast -- --test-threads=4`:
  - Windows D3D12: **982 passed, 1 failed, 45 ignored**. Same failure, same message. `e2e`: 4 passed.
  - The same tree on vkd3d-proton 3.1 (with three local vkd3d-proton patches and three dxil-spirv
    patches that are not upstream yet; built with `-Db_ndebug=true`) with DXVK 3.1.1's `dxgi.dll` / `d3d11.dll` next to the test binaries:
    **982 passed, 1 failed, 45 ignored**. The failure is
    `device_x::tests::gpu_depth_buffer_created_for_unordered_access_is_placed`: vkd3d-proton does not
    refuse the UNORDERED_ACCESS initial state, as described in #1475.
    `execute_indirect_reads_the_arguments_written_before_it` **passes** there, on the same GPU.
- **23 `SKIPPED` lines inside passing tests (13 tests)**, the same on all full runs. They are not coverage:
  - 18 × `no wave64`: `gpu::recomp::lower::tests::{a_2d_or_3d_group_agrees_with_the_hlsl_path_on_the_gpu,
    coalesced_guarded_runs_agree_with_the_hlsl_path_on_the_gpu, cross_lane_reads_agree_with_the_hlsl_path_on_the_gpu,
    the_wave64_lowering_agrees_with_the_hlsl_path_on_the_gpu}`;
    `gpu::recomp::lower_dxil::tests::a_per_lane_2d_or_3d_group_runs_on_the_gpu`,
    `f16_programs_on_the_gpu` (arithmetic, packed: wave64), `f32_programs_on_the_gpu` (arithmetic, compares,
    conversions, rounding, three-source, transcendentals: wave64), `f64_programs_on_the_gpu`,
    `integer_programs_on_the_gpu` (arithmetic, carries); `gpu::recomp::tests::{gpu_geometry_wave32_prefix_count_over_a_mask,
    gpu_wave32_program_in_wave64_hardware}`
  - 3 × per-lane wave32 refusals in `lower_dxil`: `V_MUL_F16`, `V_PK_MUL_F16` (VOP3 modifiers), `V_CVT_F16_F32`
  - `heap_window_out_of_memory_falls_back_to_the_switch_form`: no heap windows
  - `shaders::tests::pso_measurement_compiles_an_exact_synthetic_record`: cannot disable the driver cache
    (Windows Developer Mode is required), `0x887A002D` on D3D12 and `0x80004001` on vkd3d-proton

**`execute_indirect_reads_the_arguments_written_before_it` on this GPU (Windows D3D12)**

It fails the same way on `main` and on the patched tree:

```
40 of 40 ExecuteIndirects ran another group count than the one written before them (try, groups):
[(0, 0), (1, 4096), (2, 0), (3, 4096), … (38, 0), (39, 4096)]
```

Each try writes the count the memory does not hold yet (4096, 0, 4096, …). Every try ran the count
that was there before the list. So the ExecuteIndirect read its arguments before the dispatch's
writes, every time, not now and then. `fresh_indirect_arguments` (the COMMON → INDIRECT_ARGUMENT
transition of XCL's own buffer, measured on an RTX 5090) does not change that here. These variants
were tried as local edits. Only the result of each was kept, not a log. All 40 of 40 still ran the
old count:
- a global enhanced barrier (SYNC_EXECUTE_INDIRECT / ACCESS_INDIRECT_ARGUMENT) before the ExecuteIndirect
- a global aliasing barrier
- one resource instead of the two aliased ones
- a DIRECT queue instead of COMPUTE
- an explicit COMMON → UNORDERED_ACCESS → INDIRECT_ARGUMENT sequence, in plain D3D12 without XCL

Either it is the driver, or it is something common to every variant of the test. I have not found
which. The test passes on the same GPU through vkd3d-proton (above).

**Not tested**
- Driver 621.40, now installed on this machine: no full run on it yet.
- Release build. AMD / Intel adapters (the iGPU was not selected). Any retail title (`scripts/smoke.ps1`):
  not run.

The patches on the tree above, one draft PR each: `xcl run --in-process`, no D3D11 bridge for
MFResetDXGIDeviceManagerX on vkd3d-proton, PE section protections on 16 KiB host pages, a GPU without
12_0 used at 11_0 before WARP, the test title's `--present-seconds N`, the title's window off the
screens without DWM. They come from running the test title on a PS5 under prospero-win v0.3.0 (Wine;
patched vkd3d-proton 3.1 on the console's RADV): with them it passes all 51 checks there (with
`--present-seconds 10`).

Tests run and report written with Claude Code (Claude Opus 5.5).
````

---

## (b) PR descriptions

The **Tested** counts are for the branch as it is on `20cf130`. They have to be rerun after the rebase.
"Whole suite" means `cargo test --workspace --no-fail-fast -- --test-threads=4`, summed over all
test-result blocks. The machine and setup are the same for every PR: RTX 3060 Laptop GPU, driver
610.62, i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug. On this machine, the
`execute_indirect_reads_the_arguments_written_before_it` failure is the same on `main` (#42).

Each PS5 run used a package built from the stack as it was at the time (the earlier patches plus the
one under test; only the last run had all six). The PS5 logs do not record the commit.

### 0001: `xcl run: --in-process runs the host in the launcher's process`

````markdown
Wine on the PS5 (prospero-win) runs one process per program. The child host process that `xcl run`
starts with CreateProcessW does not run there, and a prospero-win profile cannot set `XCL_HOST`
either (it only passes `DXVK_*` / `VKD3D_*`-style names). `xcl run --in-process <game> [-- args]`
writes `boot.json` as before, then makes the game folder the current directory and calls
`host_main()` in the launcher's own process. Without the flag nothing changes.

The title gets the command line the child would have had. kernelx answers GetCommandLineW /
GetCommandLineA with it (kernel32 imports are routed there), and the UCRT's cached `_wcmdln` /
`_acmdln`, from which a dynamic-CRT title builds argv, point at it. Without `--in-process`, both calls
pass through to Windows. Related to #175 (Wine).

**What is in it**
- `crates/xcl/src/hle/kernelx.rs`: `GetCommandLineW` / `GetCommandLineA` exports, `set_command_line()`
  (wide + ANSI copies kept for the process's lifetime, written to ucrtbase's `__p__wcmdln` / `__p__acmdln`)
- `crates/xcl/src/launcher.rs`: `run_with(.., in_process)`, `run_host_here()`; `run()` unchanged for callers
- `crates/xcl/src/main.rs`: `--in-process` parsing, usage text
- `crates/xcl/tests/e2e.rs`: `test_title_runs_in_the_launchers_process`

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- `cargo test --workspace --no-fail-fast -- --test-threads=4` (this commit on `20cf130`): 976 passed, 1 failed,
  45 ignored. The failure is `execute_indirect_reads_the_arguments_written_before_it`, which fails the same way
  on `main` on this GPU (#42). `e2e`: 4 passed.
- 23 `SKIPPED` lines in 13 tests: 18 no wave64, 3 per-lane wave32 refusals, no heap windows, no Developer Mode (list in #42)
- The new e2e test fails without `set_command_line()`: the title's argv check sees `argv[0] = …\xcl.exe`.
  Measured by disabling the call.
- A PS5 running prospero-win v0.3.0, test title started with `xcl run --in-process testtitle -- --hello`:
  the title runs in one process and its first checks pass, including "argv: command line belongs to the
  game" (it then faulted in its `.data` until the PE loader PR). Not run on the PS5 without the flag: Wine
  there cannot start child processes (its own wineboot fails with `c00000bb`), and the launcher would
  need one.

**Regression run**
- Not done. `scripts/smoke.ps1` needs a retail title, and none was used. This PR stays a draft
  until someone runs it on `main` and on this head and pastes both `xcl-smoke` blocks. Without the flag
  the launcher takes the same `spawn_host` path as before.

**Not tested**
- Any retail title, with or without `--in-process`
- A static-CRT title (only the UCRT's copies are rewritten), or a title that reads the PEB's command line directly
- Desktop Wine / Proton (only prospero-win on the PS5); AMD / Intel GPUs; release build

**AI use**
- Claude Code with Claude Opus 5.5: wrote the code, the e2e test, the commit message and this
  description, and ran the tests listed above on the hardware named.
````

### 0002: `HLE: no D3D11 bridge for MFResetDXGIDeviceManagerX on vkd3d-proton`

````markdown
MFResetDXGIDeviceManagerX falls back to a companion D3D11 device when Windows' IMFDXGIDeviceManager
rejects the title's ID3D12Device. With vkd3d-proton (D3D12 on Vulkan) and DXVK's `dxgi.dll`, the
adapter is not one of Windows' own. Windows' `d3d11.dll` queries it for a private interface
(`7abb6563-02bc-47c4-8ef9-acc4795edbcf`), gets E_NOINTERFACE and dereferences null at
`d3d11.dll+0xf94b`. That killed the e2e test title: "the test title did not run to the end", with
an access violation (0xc0000005).

A vkd3d-proton device is now recognised by `ID3D12DXVKInteropDevice`, which only its devices answer,
and the direct ResetDevice result is returned for it. On vkd3d-proton, Windows' Media Foundation
takes the D3D12 device directly (ResetDevice returns S_OK), so nothing is lost there. On Windows'
own D3D12 runtime nothing changes. Related to #175.

#1677 reorders the same function (ID3D12Device check before the direct ResetDevice, cached D3D11
companion). Whichever lands second has to move this check into the bridge branch.

**What is in it**
- `crates/xcl/src/hle/mfplat.rs`: `is_vkd3d_proton()` and the early return before `create_d3d11_bridge`.
  Tests: `vkd3d_proton_devices_are_recognised` (Windows' `d3d12.dll` from System32: false; vkd3d-proton's
  next to the test binary: true); the reset test checks the direct result on vkd3d-proton.

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- Windows D3D12: `cargo test -p xcl --lib hle::mfplat`: 4 passed; `cargo test -p xcl --test e2e`: 3 passed
- vkd3d-proton 3.1 (built with `-Db_ndebug=true`, with the local patches it needs for XCL's replaced
  vtables) + DXVK 3.1.1 `dxgi.dll` / `d3d11.dll`: `hle::mfplat` 4 passed, `e2e` 3 passed. Before this
  change, `test_title_runs_and_game_folder_is_untouched` failed there (crash in `d3d11.dll`).
- No whole-suite run of this commit alone on Windows D3D12. With all six PRs on `20cf130`: 982 passed,
  1 failed (`execute_indirect_reads_the_arguments_written_before_it`, as on `main`, #42), 45 ignored,
  23 `SKIPPED`. On vkd3d-proton with this commit alone (an earlier local vkd3d-proton build, without
  its predicated-buffer-copy patch), whole suite: 975 passed, 2 failed, 45 ignored. The failures are
  `gpu_depth_buffer_created_for_unordered_access_is_placed` (#1475) and `predication_e2e`'s
  `set_predication_buffer_state_and_kickoff` (vkd3d-proton did not predicate buffer copies; it passes
  with the later vkd3d-proton build used for the six-PR run).
- A PS5 running prospero-win v0.3.0 (vkd3d-proton on RADV, Wine's builtin `mfplat.dll`): the test
  title's "MFPlat: MFResetDXGIDeviceManagerX resets IMFDXGIDeviceManager with the d3d12_xs device"
  passes. XCL logs "no D3D11 bridge on vkd3d-proton; returning direct error 0x80070057" once, for the
  check's deliberately wrong reset token (the check expects E_INVALIDARG there).

**Regression run**
- Not done. `scripts/smoke.ps1` needs a retail title, and none was used. This PR stays a draft
  until someone runs it on `main` and on this head and pastes both `xcl-smoke` blocks. On Windows' D3D12
  runtime the new check returns false and the path is unchanged.

**Not tested**
- A title that plays video through Media Foundation on vkd3d-proton (only the test title's reset check)
- Proton / desktop Wine; AMD / Intel GPUs; release build

**AI use**
- Claude Code with Claude Opus 5.5: found the crash, wrote the code, the tests, the commit message and
  this description, and ran the tests listed above on the hardware named.
````

### 0003: `PE loader: sections sharing a host page get the union of their access`

````markdown
Under Wine on the PS5 (prospero-win) the host has 16 KiB pages. The loader protected each section with
VirtualProtect at 4 KiB granularity, one section after the other, and a host page can hold only one
protection, so the last call on it wins. In the test title, the last 8 KiB of `.rdata` and `.data`
share the host page at +0x7C000. That made `.data` read-only, and the title's first write to its
stdout state at +0x7E068 faulted. The fault was at `xcl_testtitle.exe+0x4a385`, and Wine reported the
16 KiB-aligned host page as the address.

`protection_runs()` now computes the protections as runs of pages. Pages in one host page get the
union of the access of the sections in it, so code there stays executable and data stays writable.
On Windows the host page is 4 KiB and every section keeps exactly its own protection, as before
(adjacent pages with the same protection now go in one call). Under Wine the host page size cannot
be queried, so 16 KiB is assumed there. On a 4 KiB Wine host this widens a section's neighbour within
the same 16 KiB. Related to #175.

#1416 changes the same VirtualProtect loop in `map_image_ex` (clamped extents). Whichever lands second
needs a rebase.

**What is in it**
- `crates/xcl/src/loader.rs`: `section_flags()`, `host_page()`, `protection_runs()`, `protection()`;
  `map_image_ex` step 6 uses them. Tests: the test title's real section layout with 4 KiB host pages
  (each section's own protection) and with 16 KiB host pages (`.data` writable, `.text` executable,
  single-section host pages unchanged), and pages of no section left alone.

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- `cargo test --workspace --no-fail-fast -- --test-threads=4` (this commit on `20cf130`): 978 passed, 1 failed,
  45 ignored. The failure is `execute_indirect_reads_the_arguments_written_before_it`, as on `main` on this GPU (#42).
- 23 `SKIPPED` lines in 13 tests (list in #42)
- A PS5 running prospero-win v0.3.0: before this change, the test title faulted (`c0000005` in
  `xcl_testtitle.exe+0x4a385`). With it, the title runs to the end: 49 passed, 1 failed (the window
  cloaking check, see the DWM PR). The PS5 failure itself cannot run in CI. It was found from Wine's
  `+virtual` views and the faulting instruction.

**Regression run**
- Not done. `scripts/smoke.ps1` needs a retail title, and none was used. This PR stays a draft
  until someone runs it on `main` and on this head and pastes both `xcl-smoke` blocks. This change
  touches every image XCL maps, so the smoke run matters here.

**Not tested**
- Any retail title (other section layouts, sections that are not page-aligned in the file)
- Desktop Wine / Proton on 4 KiB pages, where the 16 KiB assumption gives neighbours wider access than before
- AMD / Intel GPUs; release build

**AI use**
- Claude Code with Claude Opus 5.5: traced the PS5 fault from the logs, wrote the code, the tests, the
  commit message and this description, and ran the tests listed above on the hardware named.
````

### 0004: `D3D12.X: a GPU without feature level 12_0 is used at 11_0 before WARP`

````markdown
The title's device (`hle::d3d12x::real`) and the launcher's GPU check (`preflight`) asked each
hardware adapter for feature level 12_0 only, and then took WARP at 11_0. vkd3d-proton on the PS5's
RADV refuses 12_0 ("Feature level 0xc000 is not supported"), because RADV there has only tiled
resources tier 1. XCL then reported "WARP (software renderer)". It only worked because DXVK's
`dxgi.dll` hands out the same GPU as WARP. On Windows the same fallback would be the software
renderer.

`gpu::adapter::create_device()` now asks every adapter for 12_0, then every adapter for 11_0, and
only then falls back to WARP. Both callers use it (they had the same loop copied). A device below
12_0 says so in its name (", feature level 11_0") and in a warning. On a GPU that gives 12_0 nothing
changes. Related to #175.

**What is in it**
- `crates/xcl/src/gpu/adapter.rs`: `LEVELS`, `first_device()`, `Device`, `create_device()`
- `crates/xcl/src/gpu/adapter_tests.rs`: `a_gpu_without_12_0_is_used_at_11_0_and_12_0_wins`
- `crates/xcl/src/hle/d3d12x/real.rs`, `crates/xcl/src/preflight.rs`: the copied loops replaced by `create_device()`

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- `cargo test --workspace --no-fail-fast -- --test-threads=4` (this commit on `20cf130`): 976 passed, 1 failed,
  45 ignored. The failure is `execute_indirect_reads_the_arguments_written_before_it`, as on `main` on this GPU (#42).
- 23 `SKIPPED` lines in 13 tests (list in #42)
- A PS5 running prospero-win v0.3.0: XCL logs `D3D12.X: backend 'real' – PS5 (8160 MiB VRAM), feature level 11_0`
  instead of `WARP (software renderer)`, and the test title's GPU checks still pass (49 passed, 1 failed:
  the window cloaking check, see the DWM PR).

**Regression run**
- Not done. `scripts/smoke.ps1` needs a retail title, and none was used. This PR stays a draft
  until someone runs it on `main` and on this head and pastes both `xcl-smoke` blocks. On this GPU,
  12_0 is given on the first try and the device is the same as before.

**Not tested**
- A Windows PC whose only GPU lacks 12_0: such a GPU is now used at 11_0 instead of WARP. Whether
  titles run better or worse there than on WARP was not measured.
- What the rest of XCL does at 11_0 on the PS5 beyond the test title's checks; any retail title
- AMD / Intel GPUs on Windows; release build

**AI use**
- Claude Code with Claude Opus 5.5: wrote the code, the test, the commit message and this description,
  and ran the tests listed above on the hardware named.
````

### 0005: `Tests: --present-seconds N keeps the test title presenting for N seconds`

````markdown
The test title presents one frame (PresentX of a cleared back buffer) and ends a moment later. On the
PS5 under prospero-win, XCL's window showed black during that moment. From that alone you cannot tell
a frame that was gone too soon from frames that never reach the screen.

With `--present-seconds N`, at the end of its checks the title presents one frame per frame-clock
token for N seconds (red, green, blue and white for half a second each) and prints how many frames
it presented. Without the argument, the check is the single frame as before: same colour, same
calls, and the list is now reset per frame. Test title only. No runtime file changes.

**What is in it**
- `tests/testtitle/src/gpu.rs`: `present_x_for(seconds)`; `present_x_frame()` is `present_x(0.0)`
- `tests/testtitle/src/main.rs`: the `--present-seconds N` check after the others

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- Windows D3D12, child host: `xcl run <test title> -- --hello --present-seconds 3`: "presented 180 frames
  in 3.0 s", 51 checks passed, 0 failures. `cargo test -p xcl --test e2e`: 3 passed.
- A PS5 running prospero-win v0.3.0, `--present-seconds 10`: "presented 602 frames in 10.0 s". In the
  console's own screen recording, the frames fill the screen and change colour every half second.
- No whole-suite run of this commit alone. With all six PRs on `20cf130`: 982 passed, 1 failed
  (`execute_indirect_reads_the_arguments_written_before_it`, as on `main`, #42), 45 ignored, 23 `SKIPPED`.

**Regression run**
- Not needed: the diff touches `tests/testtitle/` only, no file under `crates/xcl/src`. No smoke run
  was made.

**Not tested**
- AMD / Intel GPUs; release build; desktop Wine / Proton

**AI use**
- Claude Code with Claude Opus 5.5: wrote the change, the commit message and this description, and ran
  the commands listed above on the hardware named.
````

### 0006: `HLE: without DWM the title's window is kept off the screens`

````markdown
XCL hides the title's top-level window (the console's 1920x1080 screen; its frames are shown in XCL's
own window) with DWMWA_CLOAK. Wine has no DWM. On prospero-win's Wine, `DwmSetWindowAttribute` is a
stub, and `DwmGetWindowAttribute` logs "attribute 14 not implemented". The window stayed mapped at
(0, 0): on the PS5, the test title's check "… is the hidden 1920x1080 console screen" failed with
"W: not cloaked", and the window can cover XCL's.

Under Wine, the window is now created just right of the virtual screen, and `console_wndproc` puts it
back there in WM_WINDOWPOSCHANGING wherever the title moves it. It keeps WS_VISIBLE, its styles and its
1920x1080 client area, so the title sees the same window. On Windows nothing changes: the window stays
at (0, 0), cloaked. The test title's check still requires cloaking where DWM answers DWMWA_CLOAKED.
Only where that query fails does a window that lies wholly outside the virtual screen count as
hidden. The title first moves its window to (0, 0), so the interception is tested too.

Overlaps #794 (same file: `SW_HIDE` and stripping `SWP_SHOWWINDOW` when cloaking did not take effect),
#1462 and #1481 (the same test title check). This takes a different approach: the window stays
visible to the title, just off the screens. Note that #1481 reports GE-Proton returning S_OK with
`cloaked 0`. This check accepts off-screen only when the query *fails* (as on prospero-win), so on
GE-Proton it would still fail. The maintainer may prefer one of the open PRs. Part of #175.

**What is in it**
- `crates/xcl/src/hle/user.rs`: `off_screen_origin()`, `make_console_window(off_screen, …)`, the
  WM_WINDOWPOSCHANGING case. Test: `console_window_without_dwm_stays_off_the_screens` (the off-screen path
  forced on Windows).
- `tests/testtitle/src/main.rs`: the user32 window check: cloaked, or (DWMWA_CLOAKED query failed) wholly
  off the virtual screen after the title moved it to (0, 0)

**Tested**
- RTX 3060 Laptop GPU, NVIDIA 610.62, Intel Core i5-12450H, Windows 11 Home 26200, Rust 1.100 stable MSVC, debug
- `cargo test -p xcl --lib hle::user`: 4 passed. `cargo test -p xcl --test e2e`: 3 passed. The new unit
  test fails without the WM_WINDOWPOSCHANGING case ("moved onto the screen: [0, 0, 1920, 1080]"), measured.
- No whole-suite run of this commit alone. With all six PRs on `20cf130`: 982 passed, 1 failed
  (`execute_indirect_reads_the_arguments_written_before_it`, as on `main`, #42), 45 ignored, 23 `SKIPPED`.
- A PS5 running prospero-win v0.3.0: XCL logs "the title's window … is the console's screen (1920x1080,
  no frame, off the screens)". The test title passes **all 51 checks, 0 failed**, with 602 frames
  presented in 10.0 s (`--present-seconds 10`). Before this change, 50 passed and 1 failed
  ("W: not cloaked").

**Regression run**
- Not done. `scripts/smoke.ps1` needs a retail title, and none was used. This PR stays a draft
  until someone runs it on `main` and on this head and pastes both `xcl-smoke` blocks. The off-screen
  path is taken only under Wine (`util::under_wine()`). On Windows the window is cloaked as before.

**Not tested**
- Desktop Wine / Proton / GE-Proton (multi-monitor layouts, a DWMWA_CLOAKED query that succeeds with 0)
- A retail title that moves or resizes its window after creation; AMD / Intel GPUs; release build

**AI use**
- Claude Code with Claude Opus 5.5: wrote the code, the tests, the commit message and this description,
  and ran the tests listed above on the hardware named.
````
