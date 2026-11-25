// SPDX-License-Identifier: MIT
//! Xbox Series X|S (Scarlett) runtime bindings for NEON VOID XSX.
//! Imports console DLLs (`kernelx.dll`, `xgameruntime.dll`, `d3d12_xs.dll`) so XCL /
//! TrinityXSX loads and runs this executable as a native Scarlett title on PS5.

use std::ffi::c_void;
use std::mem::{size_of, transmute_copy, ManuallyDrop};
use std::path::PathBuf;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice1;

type HRESULT = i32;

pub const SCREEN_W: u32 = 1920;
pub const SCREEN_H: u32 = 1080;
pub const MAX_VERTICES: usize = 120_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub col: [f32; 4],
}

// ---------------------------------------------------------------------------
// Console DLL imports (Scarlett PE)
// ---------------------------------------------------------------------------

#[link(name = "kernelx", kind = "raw-dylib")]
extern "system" {
    fn GetConsoleType() -> u32;
}

#[repr(C)]
struct XAsyncBlock {
    queue: *mut c_void,
    context: *mut c_void,
    callback: Option<unsafe extern "system" fn(*mut XAsyncBlock)>,
    internal: [u8; 32],
}

#[link(name = "xgameruntime", kind = "raw-dylib")]
extern "system" {
    fn XGameRuntimeInitialize() -> HRESULT;
    fn XSystemGetDeviceType() -> u32;
    fn XAsyncGetStatus(b: *mut XAsyncBlock, wait: bool) -> HRESULT;
    fn XUserAddAsync(options: u32, b: *mut XAsyncBlock) -> HRESULT;
    fn XUserAddResult(b: *mut XAsyncBlock, user: *mut *mut c_void) -> HRESULT;
    fn XUserGetGamertag(u: *mut c_void, comp: u32, size: usize, buf: *mut u8, used: *mut usize) -> HRESULT;
    fn XUserCloseHandle(u: *mut c_void);
    fn XPersistentLocalStorageGetPath(size: usize, buf: *mut u8, used: *mut usize) -> HRESULT;
}

#[link(name = "kernel32", kind = "raw-dylib")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> usize;
    fn GetProcAddress(h: usize, name: *const u8) -> usize;
    fn CreateEventW(sa: *const c_void, manual: i32, initial: i32, name: *const u16) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

// ---------------------------------------------------------------------------
// DualSense via XInput (`xinput1_4.dll` -> prospero-win DualSense bridge)
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct XInputGamepad {
    pub buttons: u16,
    pub left_trigger: u8,
    pub right_trigger: u8,
    pub thumb_lx: i16,
    pub thumb_ly: i16,
    pub thumb_rx: i16,
    pub thumb_ry: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XInputState {
    packet: u32,
    gamepad: XInputGamepad,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XInputVibration {
    left_motor: u16,
    right_motor: u16,
}

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;
type XInputSetStateFn = unsafe extern "system" fn(u32, *const XInputVibration) -> u32;

pub struct Pad {
    get_state: Option<XInputGetStateFn>,
    set_state: Option<XInputSetStateFn>,
    pub prev_buttons: u16,
    pub buttons: u16,
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub l2: f32,
    pub r2: f32,
    rumble_timer: f32,
    rumble_low: u16,
    rumble_high: u16,
}

pub const BTN_UP: u16 = 0x0001;
pub const BTN_DOWN: u16 = 0x0002;
pub const BTN_LEFT: u16 = 0x0004;
pub const BTN_RIGHT: u16 = 0x0008;
pub const BTN_R1: u16 = 0x0200;
pub const BTN_CROSS: u16 = 0x1000;
pub const BTN_SQUARE: u16 = 0x4000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl Pad {
    pub fn init() -> Self {
        unsafe {
            let mut h = LoadLibraryW(wide("xinput1_4.dll").as_ptr());
            if h == 0 {
                h = LoadLibraryW(wide("xinput1_3.dll").as_ptr());
            }
            let (get_state, set_state) = if h != 0 {
                let g = GetProcAddress(h, b"XInputGetState\0".as_ptr());
                let s = GetProcAddress(h, b"XInputSetState\0".as_ptr());
                (
                    if g != 0 { Some(std::mem::transmute(g)) } else { None },
                    if s != 0 { Some(std::mem::transmute(s)) } else { None },
                )
            } else {
                (None, None)
            };
            Self {
                get_state,
                set_state,
                prev_buttons: 0,
                buttons: 0,
                lx: 0.0,
                ly: 0.0,
                rx: 0.0,
                ry: 0.0,
                l2: 0.0,
                r2: 0.0,
                rumble_timer: 0.0,
                rumble_low: 0,
                rumble_high: 0,
            }
        }
    }

    pub fn poll(&mut self, dt: f32) {
        self.prev_buttons = self.buttons;
        if let Some(get) = self.get_state {
            let mut st = XInputState::default();
            if unsafe { get(0, &mut st) } == 0 {
                let g = st.gamepad;
                self.buttons = g.buttons;
                self.lx = deadzone(g.thumb_lx as f32 / 32767.0, 0.18);
                self.ly = deadzone(g.thumb_ly as f32 / 32767.0, 0.18);
                self.rx = deadzone(g.thumb_rx as f32 / 32767.0, 0.22);
                self.ry = deadzone(g.thumb_ry as f32 / 32767.0, 0.22);
                self.l2 = g.left_trigger as f32 / 255.0;
                self.r2 = g.right_trigger as f32 / 255.0;
            }
        }
        if self.rumble_timer > 0.0 {
            self.rumble_timer -= dt;
            if self.rumble_timer <= 0.0 {
                self.rumble_low = 0;
                self.rumble_high = 0;
                self.apply_rumble();
            }
        }
    }

    pub fn pressed(&self, mask: u16) -> bool {
        (self.buttons & mask) != 0 && (self.prev_buttons & mask) == 0
    }

    pub fn held(&self, mask: u16) -> bool {
        (self.buttons & mask) != 0
    }

    pub fn trigger_rumble(&mut self, low: f32, high: f32, duration: f32) {
        let l = (low.clamp(0.0, 1.0) * 65535.0) as u16;
        let h = (high.clamp(0.0, 1.0) * 65535.0) as u16;
        if l >= self.rumble_low || h >= self.rumble_high || self.rumble_timer <= 0.0 {
            self.rumble_low = l;
            self.rumble_high = h;
            self.rumble_timer = duration;
            self.apply_rumble();
        }
    }

    pub fn stop_rumble(&mut self) {
        self.rumble_timer = 0.0;
        self.rumble_low = 0;
        self.rumble_high = 0;
        self.apply_rumble();
    }

    fn apply_rumble(&self) {
        if let Some(set) = self.set_state {
            let v = XInputVibration {
                left_motor: self.rumble_low,
                right_motor: self.rumble_high,
            };
            unsafe {
                let _ = set(0, &v);
            }
        }
    }
}

fn deadzone(v: f32, dz: f32) -> f32 {
    if v.abs() < dz {
        0.0
    } else {
        ((v.abs() - dz) / (1.0 - dz)).copysign(v)
    }
}

// ---------------------------------------------------------------------------
// Console runtime info, active profile gamertag & Persistent Local Storage
// ---------------------------------------------------------------------------

pub struct ConsoleIdentity {
    pub gamertag: String,
    pub console_label: String,
    pub pls_dir: Option<PathBuf>,
}

pub fn init_console_runtime() -> ConsoleIdentity {
    unsafe {
        let _ = XGameRuntimeInitialize();
        let ctype = GetConsoleType();
        let dtype = XSystemGetDeviceType();
        let console_label = match (ctype, dtype) {
            (6, _) | (_, 6) => "XBOX SERIES X".to_string(),
            (5, _) | (_, 5) => "XBOX SERIES S".to_string(),
            _ => format!("SCARLETT ({ctype})"),
        };

        // Read active offline TrinityXSX gamertag via XUserAddAsync + XUserGetGamertag
        let mut gamertag = "PS5PLAYER".to_string();
        let mut block = XAsyncBlock {
            queue: std::ptr::null_mut(),
            context: std::ptr::null_mut(),
            callback: None,
            internal: [0u8; 32],
        };
        if XUserAddAsync(1, &mut block) >= 0 {
            let _ = XAsyncGetStatus(&mut block, true);
            let mut user = std::ptr::null_mut();
            if XUserAddResult(&mut block, &mut user) >= 0 && !user.is_null() {
                let mut buf = [0u8; 64];
                let mut used = 0usize;
                if XUserGetGamertag(user, 0, buf.len(), buf.as_mut_ptr(), &mut used) >= 0 {
                    let end = buf.iter().position(|&b| b == 0).unwrap_or(used.min(buf.len()));
                    let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
                    if !s.is_empty() {
                        gamertag = s.to_ascii_uppercase();
                    }
                }
                XUserCloseHandle(user);
            }
        }

        // Resolve console Persistent Local Storage (PLS) save root
        let mut pls_buf = [0u8; 512];
        let mut pls_used = 0usize;
        let pls_dir = if XPersistentLocalStorageGetPath(pls_buf.len(), pls_buf.as_mut_ptr(), &mut pls_used) >= 0 {
            let end = pls_buf.iter().position(|&b| b == 0).unwrap_or(pls_used.min(pls_buf.len()));
            let raw = String::from_utf8_lossy(&pls_buf[..end]).trim().to_string();
            if raw.is_empty() { None } else { Some(PathBuf::from(raw)) }
        } else {
            None
        };

        ConsoleIdentity { gamertag, console_label, pls_dir }
    }
}

// ---------------------------------------------------------------------------
// D3D12.X 1920x1080 60 FPS Renderer (d3d12_xs.dll + PresentX)
// ---------------------------------------------------------------------------

type D3D12CreateDeviceFn = unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void) -> HRESULT;
type D3D12SerializeRootSignatureFn = unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
type SetRootSignatureFn = unsafe extern "system" fn(*mut c_void, *mut c_void);
type IaSetVertexBuffersFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const D3D12_VERTEX_BUFFER_VIEW);
type OmSetRenderTargetsFn = unsafe extern "system" fn(*mut c_void, u32, *const D3D12_CPU_DESCRIPTOR_HANDLE, i32, *const D3D12_CPU_DESCRIPTOR_HANDLE);
type ClearRtvFn = unsafe extern "system" fn(*mut c_void, D3D12_CPU_DESCRIPTOR_HANDLE, *const [f32; 4], u32, *const RECT);
type SetFrameIntervalXFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32) -> HRESULT;
type ScheduleFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32) -> HRESULT;
type WaitFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32, *mut u64) -> HRESULT;
type PresentXFn = unsafe extern "system" fn(*mut c_void, u32, *const u8, *const u8) -> HRESULT;

const X_SET_GRAPHICS_ROOT_SIGNATURE: usize = 34;
const X_IA_SET_VERTEX_BUFFERS: usize = 48;
const X_OM_SET_RENDER_TARGETS: usize = 50;
const X_CLEAR_RENDER_TARGET_VIEW: usize = 52;

static VS_DXBC: &[u8] = include_bytes!("../shaders/quad_vs.dxbc");
static PS_DXBC: &[u8] = include_bytes!("../shaders/quad_ps.dxbc");

unsafe fn xs<F: Copy>(name: &str) -> Result<F, String> {
    let h = LoadLibraryW(wide("d3d12_xs.dll").as_ptr());
    if h == 0 {
        return Err("LoadLibraryW(d3d12_xs.dll) failed".into());
    }
    let f = GetProcAddress(h, format!("{name}\0").as_ptr());
    if f == 0 {
        return Err(format!("d3d12_xs.dll!{name} not found"));
    }
    Ok(transmute_copy(&f))
}

unsafe fn vslot<F: Copy>(obj: *mut c_void, index: usize) -> F {
    transmute_copy(&*(*(obj as *const *const usize)).add(index))
}

pub struct Renderer {
    dev: ID3D12Device,
    queue: ID3D12CommandQueue,
    bb: ID3D12Resource,
    _rtv_heap: ID3D12DescriptorHeap,
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    rs: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    alloc: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_ev: *mut c_void,
    fence_val: u64,
    vbuf: ID3D12Resource,
    vbuf_ptr: *mut Vertex,
    vbuf_gpu: u64,
    frame: u64,
    wait_event: WaitFrameEventXFn,
    present_x: PresentXFn,
    vram_stress: Vec<ID3D12Resource>,
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        unsafe {
            let create: D3D12CreateDeviceFn = xs("D3D12CreateDevice")?;
            let mut raw_dev = std::ptr::null_mut();
            let hr = create(std::ptr::null_mut(), 0xC000, &ID3D12Device::IID as *const _ as *const c_void, &mut raw_dev);
            if hr < 0 || raw_dev.is_null() {
                return Err(format!("D3D12CreateDevice hr={hr:#x}"));
            }
            let dev = ID3D12Device::from_raw(raw_dev);
            let queue: ID3D12CommandQueue = dev
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default())
                .map_err(|e| format!("CreateCommandQueue: {e}"))?;

            let d = dev.as_raw();
            let set_interval: SetFrameIntervalXFn = vslot(d, 78);
            let schedule: ScheduleFrameEventXFn = vslot(d, 79);
            let wait_event: WaitFrameEventXFn = vslot(d, 80);
            let present_x: PresentXFn = vslot(queue.as_raw(), 30);

            let output_raw = dev
                .cast::<IDXGIDevice1>()
                .ok()
                .and_then(|dxgi| dxgi.GetAdapter().ok())
                .and_then(|adapter| adapter.EnumOutputs(0).ok())
                .map_or(std::ptr::null_mut(), |o| o.as_raw());
            if set_interval(d, output_raw, 16667, 1, 0) < 0 {
                return Err("SetFrameIntervalX failed".into());
            }
            if schedule(d, 0, 0, std::ptr::null(), 0) < 0 {
                return Err("ScheduleFrameEventX failed".into());
            }

            // Back buffer 1920x1080 RGBA8
            let bb_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: SCREEN_W as u64,
                Height: SCREEN_H,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                ..Default::default()
            };
            let mut bb: Option<ID3D12Resource> = None;
            dev.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
                D3D12_HEAP_FLAG_NONE,
                &bb_desc,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                None,
                &mut bb,
            )
            .map_err(|e| format!("CreateCommittedResource(backbuffer): {e}"))?;
            let bb = bb.ok_or("backbuffer is null")?;

            let rtv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    ..Default::default()
                })
                .map_err(|e| format!("CreateDescriptorHeap(RTV): {e}"))?;
            let start_fn: unsafe extern "system" fn(*mut c_void) -> usize = vslot(rtv_heap.as_raw(), 9);
            let rtv = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: start_fn(rtv_heap.as_raw()) };
            dev.CreateRenderTargetView(&bb, None, rtv);

            // Empty root signature allowing IA input layout
            let rs_desc = D3D12_ROOT_SIGNATURE_DESC {
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
                ..Default::default()
            };
            let serialize: D3D12SerializeRootSignatureFn = xs("D3D12SerializeRootSignature")?;
            let (mut rs_blob, mut rs_err) = (std::ptr::null_mut(), std::ptr::null_mut());
            let hr = serialize(&rs_desc as *const _ as *const c_void, 1, &mut rs_blob, &mut rs_err);
            if hr < 0 || rs_blob.is_null() {
                return Err(format!("D3D12SerializeRootSignature hr={hr:#x}"));
            }
            let rs_blob = ID3DBlob::from_raw(rs_blob);
            let rs_bytes = std::slice::from_raw_parts(rs_blob.GetBufferPointer() as *const u8, rs_blob.GetBufferSize());
            let rs: ID3D12RootSignature = dev
                .CreateRootSignature(0, rs_bytes)
                .map_err(|e| format!("CreateRootSignature: {e}"))?;

            // Graphics PSO with alpha blending for glowing neon trails & rings
            let elements = [
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"POSITION\0".as_ptr()),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"COLOR\0".as_ptr()),
                    SemanticIndex: 0,
                    Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                    InputSlot: 0,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    InstanceDataStepRate: 0,
                },
            ];
            let mut pso_desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
            pso_desc.pRootSignature = transmute_copy(&rs);
            pso_desc.VS = D3D12_SHADER_BYTECODE {
                pShaderBytecode: VS_DXBC.as_ptr() as *const c_void,
                BytecodeLength: VS_DXBC.len(),
            };
            pso_desc.PS = D3D12_SHADER_BYTECODE {
                pShaderBytecode: PS_DXBC.as_ptr() as *const c_void,
                BytecodeLength: PS_DXBC.len(),
            };
            pso_desc.InputLayout = D3D12_INPUT_LAYOUT_DESC {
                pInputElementDescs: elements.as_ptr(),
                NumElements: elements.len() as u32,
            };
            let rt_blend = &mut pso_desc.BlendState.RenderTarget[0];
            rt_blend.BlendEnable = true.into();
            rt_blend.SrcBlend = D3D12_BLEND_SRC_ALPHA;
            rt_blend.DestBlend = D3D12_BLEND_INV_SRC_ALPHA;
            rt_blend.BlendOp = D3D12_BLEND_OP_ADD;
            rt_blend.SrcBlendAlpha = D3D12_BLEND_ONE;
            rt_blend.DestBlendAlpha = D3D12_BLEND_INV_SRC_ALPHA;
            rt_blend.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            rt_blend.RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
            pso_desc.SampleMask = u32::MAX;
            pso_desc.RasterizerState.FillMode = D3D12_FILL_MODE_SOLID;
            pso_desc.RasterizerState.CullMode = D3D12_CULL_MODE_NONE;
            pso_desc.RasterizerState.DepthClipEnable = false.into();
            pso_desc.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
            pso_desc.NumRenderTargets = 1;
            pso_desc.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
            pso_desc.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };

            let pso: ID3D12PipelineState = dev
                .CreateGraphicsPipelineState(&pso_desc)
                .map_err(|e| format!("CreateGraphicsPipelineState: {e}"))?;

            // Persistently mapped UPLOAD vertex buffer
            let vb_bytes = (MAX_VERTICES * size_of::<Vertex>()) as u64;
            let vb_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: vb_bytes,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_UNKNOWN,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                ..Default::default()
            };
            let mut vbuf: Option<ID3D12Resource> = None;
            dev.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_UPLOAD, ..Default::default() },
                D3D12_HEAP_FLAG_NONE,
                &vb_desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                &mut vbuf,
            )
            .map_err(|e| format!("CreateCommittedResource(vbuf): {e}"))?;
            let vbuf = vbuf.ok_or("vbuf is null")?;
            let mut vbuf_raw = std::ptr::null_mut();
            vbuf.Map(0, None, Some(&mut vbuf_raw))
                .map_err(|e| format!("vbuf.Map: {e}"))?;
            let vbuf_gpu = vbuf.GetGPUVirtualAddress();

            let alloc: ID3D12CommandAllocator = dev
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|e| format!("CreateCommandAllocator: {e}"))?;
            let list: ID3D12GraphicsCommandList = dev
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, Some(&pso))
                .map_err(|e| format!("CreateCommandList: {e}"))?;
            list.Close().map_err(|e| format!("list.Close: {e}"))?;
            let fence: ID3D12Fence = dev
                .CreateFence(0, D3D12_FENCE_FLAG_NONE)
                .map_err(|e| format!("CreateFence: {e}"))?;
            let fence_ev = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());

            Ok(Self {
                dev,
                queue,
                bb,
                _rtv_heap: rtv_heap,
                rtv,
                rs,
                pso,
                alloc,
                list,
                fence,
                fence_ev,
                fence_val: 0,
                vbuf,
                vbuf_ptr: vbuf_raw as *mut Vertex,
                vbuf_gpu,
                frame: 0,
                wait_event,
                present_x,
                vram_stress: Vec::new(),
            })
        }
    }

    /// Allocate or release real 256 MiB D3D12.X DEFAULT-heap VRAM chunks for live stress testing.
    pub fn set_vram_stress_chunks(&mut self, chunks_256mb: usize) -> usize {
        while self.vram_stress.len() > chunks_256mb {
            self.vram_stress.pop();
        }
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: 256 * 1024 * 1024,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        while self.vram_stress.len() < chunks_256mb {
            let mut res: Option<ID3D12Resource> = None;
            let ok = unsafe {
                self.dev.CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut res,
                )
            };
            if ok.is_ok() {
                if let Some(r) = res {
                    self.vram_stress.push(r);
                    continue;
                }
            }
            break;
        }
        self.vram_stress.len() * 256
    }

    pub fn present_frame(&mut self, clear: [f32; 4], vertices: &[Vertex]) -> Result<(), String> {
        unsafe {
            let mut token = 0u64;
            let hr = (self.wait_event)(self.dev.as_raw(), 0, 500, std::ptr::null(), 0, &mut token);
            if hr < 0 || token == 0 {
                token = self.frame + 1;
            }

            let count = vertices.len().min(MAX_VERTICES);
            if count > 0 {
                std::ptr::copy_nonoverlapping(vertices.as_ptr(), self.vbuf_ptr, count);
            }

            self.alloc.Reset().map_err(|e| format!("alloc.Reset: {e}"))?;
            self.list.Reset(&self.alloc, Some(&self.pso)).map_err(|e| format!("list.Reset: {e}"))?;

            let make_barrier = |before, after| D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                        pResource: transmute_copy(&self.bb),
                        Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                        StateBefore: before,
                        StateAfter: after,
                    }),
                },
            };

            if self.frame > 0 {
                self.list.ResourceBarrier(&[make_barrier(
                    D3D12_RESOURCE_STATE_PRESENT,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                )]);
            }

            let l = self.list.as_raw();
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv, 0, std::ptr::null());
            vslot::<ClearRtvFn>(l, X_CLEAR_RENDER_TARGET_VIEW)(l, self.rtv, &clear, 0, std::ptr::null());

            if count >= 3 {
                vslot::<SetRootSignatureFn>(l, X_SET_GRAPHICS_ROOT_SIGNATURE)(l, self.rs.as_raw());
                self.list.SetPipelineState(&self.pso);
                self.list.RSSetViewports(&[D3D12_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: SCREEN_W as f32,
                    Height: SCREEN_H as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]);
                self.list.RSSetScissorRects(&[RECT {
                    left: 0,
                    top: 0,
                    right: SCREEN_W as i32,
                    bottom: SCREEN_H as i32,
                }]);
                self.list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
                let view = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.vbuf_gpu,
                    SizeInBytes: (count * size_of::<Vertex>()) as u32,
                    StrideInBytes: size_of::<Vertex>() as u32,
                };
                vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                self.list.DrawInstanced(count as u32, 1, 0, 0);
            }

            self.list.ResourceBarrier(&[make_barrier(
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )]);
            self.list.Close().map_err(|e| format!("list.Close: {e}"))?;

            let cmd: ID3D12CommandList = self.list.cast().map_err(|e| format!("cast cmdlist: {e}"))?;
            self.queue.ExecuteCommandLists(&[Some(cmd)]);

            // Console PresentX plane descriptor
            let resources = [self.bb.as_raw()];
            let mut plane = [0u8; 0x40];
            plane[..8].copy_from_slice(&token.to_le_bytes());
            plane[8..12].copy_from_slice(&1u32.to_le_bytes());
            plane[0x10..0x18].copy_from_slice(&(resources.as_ptr() as u64).to_le_bytes());
            let hr = (self.present_x)(self.queue.as_raw(), 1, plane.as_ptr(), std::ptr::null());
            if hr < 0 {
                return Err(format!("PresentX hr={hr:#x}"));
            }

            // Wait for GPU frame completion so our dynamic vertex buffer & allocator can be reused cleanly
            self.fence_val += 1;
            self.queue.Signal(&self.fence, self.fence_val).map_err(|e| format!("Signal: {e}"))?;
            self.fence
                .SetEventOnCompletion(self.fence_val, HANDLE(self.fence_ev))
                .map_err(|e| format!("SetEventOnCompletion: {e}"))?;
            let _ = WaitForSingleObject(self.fence_ev, 5000);
            self.frame += 1;
            Ok(())
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            let _ = self.vbuf.Unmap(0, None);
            if !self.fence_ev.is_null() {
                CloseHandle(self.fence_ev);
            }
        }
    }
}
