# Status (2026-11-25)

Hardware: Windows 11, NVIDIA GeForce RTX 3060 Laptop GPU (+ Intel UHD), Rust 1.100; PS5 with
prospero-win v0.3.0.

## Windows

`cargo test --workspace --no-fail-fast -- --test-threads=4` in `xcl/` with the patches:

| Backend | Passed | Failed | Ignored | Failures |
|---|---|---|---|---|
| Windows D3D12 | 978 | 1 | 45 | `execute_indirect_reads_the_arguments_written_before_it` (also fails on upstream `main` on this GPU) |
| vkd3d-proton 3.1 (patched, `b_ndebug`) + DXVK 3.1.1 | 975 | 2 | 45 | `gpu_depth_buffer_created_for_unordered_access_is_placed`, `set_predication_buffer_state_and_kickoff` |

23 GPU tests report `SKIPPED` on this GPU (no wave64). The vkd3d-proton row was measured before
the PE-loader patch; the e2e test title passes on both backends.

What it took to get XCL onto vkd3d-proton:

- **Asserts.** vkd3d-proton's release builds keep asserts; its `impl_from_*()` helpers assert on the
  vtable pointer, which XCL replaces with an extended copy. Built with `-Db_ndebug=true`.
- **Object identity** (`patches/vkd3d-proton`). Command allocators, bundles and command lists are
  recognised by their vtable pointer even without asserts (`E_INVALIDARG` from
  CreateCommandList, a crash in ExecuteCommandLists). `VKD3D_VTBL_IS()` also accepts a vtable
  whose element [-1] is vkd3d's – where XCL keeps the original.
- **Swap chain.** Windows' `dxgi.dll` cannot make one for a vkd3d-proton queue (`E_NOTIMPL`):
  DXVK's `dxgi.dll` / `d3d11.dll` go next to `xcl.exe`.
- **Media Foundation** (`patches/xcl`). XCL's companion D3D11 device for MFResetDXGIDeviceManagerX
  made Windows' `d3d11.dll` dereference null on DXVK's adapter; skipped on vkd3d-proton (Windows'
  Media Foundation takes the D3D12 device directly there).
- **dxil-spirv** (`patches/dxil-spirv`), found with XCL's direct DXIL emitter and recompiler:
  - the `dx.op.udiv` helpers were typed by the {quotient, remainder} pair (invalid SPIR-V,
    NVIDIA's driver crashed compiling it);
  - the structured stride was read positionally from the resource tags, so a ByteAddressBuffer
    with 64-bit atomics (tags `{3, 1}`) got stride 1 and loads with an undef offset – a GPU hang
    and wrong atomic results;
  - `UAddc` / `USubb` swapped value and carry and used an invalid result type.

Left as they are:

- `gpu_depth_buffer_…`: vkd3d-proton does not refuse UNORDERED_ACCESS as an initial state without
  `ALLOW_UNORDERED_ACCESS`; the test checks that Windows does. XCL's own mapping works.
- Predication: vkd3d-proton implements it with VK_EXT_conditional_rendering, which does not cover
  copies; D3D12 predicates `CopyBufferRegion` too.

## PS5 (prospero-win)

1. Wine could not create the `default` prefix (`run_wineboot failed … c00000bb`, then
   `could not load kernel32.dll`): process creation is off on the console. A prefix made by Wine
   on a PC went to `/data/prospero-win/prefix`.
2. XCL started in one process (`xcl run --in-process`) and passed the test title's first checks:
   argv, static TLS (main and new threads), C++ exceptions through the `RtlPcToFileHeader` hook,
   kernelx, XMemVirtualAlloc in unified memory. vkd3d-proton made a device on
   `PS5 (radv 26.2.0)` (SM 6.8, DXR 1.1, descriptor buffer). Then the title faulted writing its
   own `.data`.
3. Cause: the console has 16 KiB pages. The loader protected `.rdata` and `.data` at 4 KiB
   granularity; both share a host page, the last VirtualProtect (`.rdata`, read-only) won. Found
   from Wine's `+virtual` views and the faulting instruction (the fault address is the 16 KiB
   host page). Fixed in `patches/xcl/0003`.
4. With that fix the test title runs to the end on the console: **49 passed, 1 failed**. Passing
   there: the triangle rendered on the PS5 GPU and read back, PresentX and the frame clock,
   console device and command-list methods, sparse VRAM heaps and uploads, UMA windows, textures
   in title memory and streaming pools, the raytracing surface, DirectStorage, XAudio2 with XMA2
   and Opus voices, MFPlat, the VFS and XGameRuntime (task queues, users, saves). The failure:
   `user32: … not cloaked` – Wine has no DWM, so DWMWA_CLOAK does nothing and the title's own
   window is not hidden (prospero-win shows the game's windows, so it may cover XCL's).
5. With `patches/xcl/0004` XCL takes the console GPU at feature level 11_0 ("PS5 (8160 MiB VRAM),
   feature level 11_0") instead of WARP, and the GPU checks still pass. The test title is not
   repeatable on one data folder: a second run sees the first run's overlay (its appended
   `hello.txt`, the whiteout of `delete_me.txt`) and fails two file checks; the overlay
   (`…/XCL/XCL.TestTitle/overlay`) has to be emptied between runs: `tools/ftp_rm_tree.py`.

`tools/vkcaps` on the console (`PS5`, Vulkan 1.4.354, RADV): sparse binding and residency for
buffers and 2D / 3D images are there, but `shaderResourceResidency` and
`residencyNonResidentStrict` are 0, so vkd3d-proton stops at tiled resources tier 1 and feature
level 11_0. `VK_EXT_mesh_shader` is there, `VK_KHR_fragment_shader_barycentric` is not;
`maxBufferSize` is 4 GiB − 4; descriptor buffer, shader object and fragment shader interlock are
there.

Open on the PS5:

- Feature level 12_0 is out of reach while RADV on the console lacks `shaderResourceResidency` and
  `residencyNonResidentStrict` (tiled resources tier 1); XCL runs at 11_0 there.
- vkd3d-proton disables mesh shaders without fragment-shader barycentrics (its UE5 workaround);
  XCL needs them for NGG programs and does not use SV_Barycentrics. `patches/vkd3d-proton/0002`
  opts `xcl.exe` in: on the console vkd3d logs "Detected game xcl.exe", XCL's "no mesh shaders"
  warning is gone, and the test title passes 49 of 50 (the cloaking check fails, as before).
- RADV's `maxBufferSize` is 4 GiB − 4; XCL's three sparse windows are 4 GiB buffers (vkd3d-proton
  warns and creates them).
