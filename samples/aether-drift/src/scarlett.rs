// SPDX-License-Identifier: MIT
//! Xbox Series X|S (Scarlett) 3D D3D12.X renderer, DualSense XInput bridge, and XUser/PLS runtime.

use std::ffi::c_void;
use std::mem::{size_of, transmute_copy, ManuallyDrop};
use std::path::PathBuf;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice1;

use crate::math::Mat4;

type HRESULT = i32;

pub const SCREEN_W: u32 = 1920;
pub const SCREEN_H: u32 = 1080;
pub const MAX_DYN3D_VERTS: usize = 65_536;
pub const MAX_FX3D_VERTS: usize = 48_000;
pub const MAX_HUD2D_VERTS: usize = 48_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex3D {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub col: [f32; 4],
    pub mat: [f32; 2], // x = wetness/metallic, y = emissive
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex2D {
    pub pos: [f32; 2],
    pub col: [f32; 4],
}

#[repr(C, align(256))]
#[derive(Clone, Copy)]
pub struct FrameCB {
    pub view_proj: [[f32; 4]; 4],
    pub cam_pos_time: [f32; 4],
    pub fog_params: [f32; 4],
    pub sun_dir_glow: [f32; 4],
}

// ---------------------------------------------------------------------------
// Scarlett Console Imports
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

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

// ---------------------------------------------------------------------------
// DualSense Input + Rumble
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct XInputGamepad {
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    thumb_lx: i16,
    thumb_ly: i16,
    thumb_rx: i16,
    thumb_ry: i16,
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

pub const BTN_UP: u16 = 0x0001;
pub const BTN_DOWN: u16 = 0x0002;
pub const BTN_LEFT: u16 = 0x0004;
pub const BTN_RIGHT: u16 = 0x0008;
pub const BTN_START: u16 = 0x0010;
pub const BTN_L1: u16 = 0x0100;
pub const BTN_R1: u16 = 0x0200;
pub const BTN_CROSS: u16 = 0x1000;
pub const BTN_CIRCLE: u16 = 0x2000;
pub const BTN_SQUARE: u16 = 0x4000;
pub const BTN_TRIANGLE: u16 = 0x8000;

pub struct Pad {
    get_state: Option<XInputGetStateFn>,
    set_state: Option<XInputSetStateFn>,
    pub prev: u16,
    pub buttons: u16,
    pub lx: f32,
    pub ly: f32,
    pub l2: f32,
    pub r2: f32,
    rumble_pulse: f32,
    pulse_low: f32,
    pulse_high: f32,
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
                prev: 0,
                buttons: 0,
                lx: 0.0,
                ly: 0.0,
                l2: 0.0,
                r2: 0.0,
                rumble_pulse: 0.0,
                pulse_low: 0.0,
                pulse_high: 0.0,
            }
        }
    }

    pub fn poll(&mut self, dt: f32) {
        self.prev = self.buttons;
        if let Some(get) = self.get_state {
            let mut st = XInputState::default();
            if unsafe { get(0, &mut st) } == 0 {
                let g = st.gamepad;
                self.buttons = g.buttons;
                self.lx = deadzone(g.thumb_lx as f32 / 32767.0, 0.14);
                self.ly = deadzone(g.thumb_ly as f32 / 32767.0, 0.14);
                self.l2 = g.left_trigger as f32 / 255.0;
                self.r2 = g.right_trigger as f32 / 255.0;
            }
        }
        if self.rumble_pulse > 0.0 {
            self.rumble_pulse = (self.rumble_pulse - dt).max(0.0);
        }
    }

    pub fn pressed(&self, m: u16) -> bool {
        (self.buttons & m) != 0 && (self.prev & m) == 0
    }

    pub fn held(&self, m: u16) -> bool {
        (self.buttons & m) != 0
    }

    pub fn pulse(&mut self, low: f32, high: f32, secs: f32) {
        if low >= self.pulse_low || high >= self.pulse_high || self.rumble_pulse <= 0.0 {
            self.pulse_low = low.clamp(0.0, 1.0);
            self.pulse_high = high.clamp(0.0, 1.0);
            self.rumble_pulse = secs;
        }
    }

    pub fn update_rumble(&self, continuous_low: f32, continuous_high: f32) {
        if let Some(set) = self.set_state {
            let (pl, ph) = if self.rumble_pulse > 0.0 {
                (self.pulse_low, self.pulse_high)
            } else {
                (0.0, 0.0)
            };
            let v = XInputVibration {
                left_motor: (continuous_low.max(pl).clamp(0.0, 1.0) * 65535.0) as u16,
                right_motor: (continuous_high.max(ph).clamp(0.0, 1.0) * 65535.0) as u16,
            };
            unsafe {
                let _ = set(0, &v);
            }
        }
    }
}

fn deadzone(v: f32, dz: f32) -> f32 {
    if v.abs() < dz { 0.0 } else { ((v.abs() - dz) / (1.0 - dz)).copysign(v) }
}

// ---------------------------------------------------------------------------
// Console identity & PLS
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
            (6, _) | (_, 6) => "XBOX SERIES X".into(),
            (5, _) | (_, 5) => "XBOX SERIES S".into(),
            _ => format!("SCARLETT ({ctype})"),
        };

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
// D3D12.X 3D Renderer (Opaque PBR + Additive Glow FX + 2D Alpha HUD + D32 Depth)
// ---------------------------------------------------------------------------

type D3D12CreateDeviceFn = unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void) -> HRESULT;
type D3D12SerializeRootSignatureFn = unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
type SetRootSignatureFn = unsafe extern "system" fn(*mut c_void, *mut c_void);
type SetRootCbvFn = unsafe extern "system" fn(*mut c_void, u32, u64);
type IaSetVertexBuffersFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const D3D12_VERTEX_BUFFER_VIEW);
type OmSetRenderTargetsFn = unsafe extern "system" fn(*mut c_void, u32, *const D3D12_CPU_DESCRIPTOR_HANDLE, i32, *const D3D12_CPU_DESCRIPTOR_HANDLE);
type ClearDsvFn = unsafe extern "system" fn(*mut c_void, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CLEAR_FLAGS, f32, u8, u32, *const RECT);
type ClearRtvFn = unsafe extern "system" fn(*mut c_void, D3D12_CPU_DESCRIPTOR_HANDLE, *const [f32; 4], u32, *const RECT);
type SetFrameIntervalXFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32) -> HRESULT;
type ScheduleFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32) -> HRESULT;
type WaitFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32, *mut u64) -> HRESULT;
type PresentXFn = unsafe extern "system" fn(*mut c_void, u32, *const u8, *const u8) -> HRESULT;

const X_SET_GRAPHICS_ROOT_SIGNATURE: usize = 34;
const X_SET_GRAPHICS_ROOT_CBV: usize = 42;
const X_IA_SET_VERTEX_BUFFERS: usize = 48;
const X_OM_SET_RENDER_TARGETS: usize = 50;
const X_CLEAR_DEPTH_STENCIL_VIEW: usize = 51;
const X_CLEAR_RENDER_TARGET_VIEW: usize = 52;

static WORLD_VS: &[u8] = include_bytes!("../shaders/world3d_vs.dxbc");
static WORLD_PS: &[u8] = include_bytes!("../shaders/world3d_ps.dxbc");
static HUD_VS: &[u8] = include_bytes!("../shaders/hud2d_vs.dxbc");
static HUD_PS: &[u8] = include_bytes!("../shaders/hud2d_ps.dxbc");

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

fn create_upload_buf(dev: &ID3D12Device, bytes: u64) -> Result<(ID3D12Resource, *mut u8, u64), String> {
    unsafe {
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: bytes.max(256),
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: DXGI_FORMAT_UNKNOWN,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        let mut res: Option<ID3D12Resource> = None;
        dev.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_UPLOAD, ..Default::default() },
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut res,
        )
        .map_err(|e| format!("CreateCommittedResource(upload {bytes}): {e}"))?;
        let res = res.ok_or("upload buffer null")?;
        let mut ptr = std::ptr::null_mut();
        res.Map(0, None, Some(&mut ptr)).map_err(|e| format!("Map: {e}"))?;
        let gpu = res.GetGPUVirtualAddress();
        Ok((res, ptr as *mut u8, gpu))
    }
}

pub struct Renderer {
    dev: ID3D12Device,
    queue: ID3D12CommandQueue,
    bb: ID3D12Resource,
    _depth: ID3D12Resource,
    _rtv_heap: ID3D12DescriptorHeap,
    _dsv_heap: ID3D12DescriptorHeap,
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    dsv: D3D12_CPU_DESCRIPTOR_HANDLE,
    rs_world: ID3D12RootSignature,
    rs_hud: ID3D12RootSignature,
    pso_world: ID3D12PipelineState,
    pso_fx: ID3D12PipelineState,
    pso_hud: ID3D12PipelineState,
    alloc: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_ev: *mut c_void,
    fence_val: u64,
    _cb_buf: ID3D12Resource,
    cb_ptr: *mut FrameCB,
    cb_gpu: u64,
    static_buf: Option<(ID3D12Resource, u64, u32)>,
    _dyn3d_buf: ID3D12Resource,
    dyn3d_ptr: *mut Vertex3D,
    dyn3d_gpu: u64,
    _fx3d_buf: ID3D12Resource,
    fx3d_ptr: *mut Vertex3D,
    fx3d_gpu: u64,
    _hud2d_buf: ID3D12Resource,
    hud2d_ptr: *mut Vertex2D,
    hud2d_gpu: u64,
    frame: u64,
    wait_event: WaitFrameEventXFn,
    present_x: PresentXFn,
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

            // 1920x1080 RGBA8 backbuffer
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
            .map_err(|e| format!("backbuffer: {e}"))?;
            let bb = bb.ok_or("bb null")?;

            // 1920x1080 D32_FLOAT hardware depth buffer
            let depth_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: SCREEN_W as u64,
                Height: SCREEN_H,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_D32_FLOAT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
                ..Default::default()
            };
            let depth_clear = D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_D32_FLOAT,
                Anonymous: D3D12_CLEAR_VALUE_0 {
                    DepthStencil: D3D12_DEPTH_STENCIL_VALUE { Depth: 1.0, Stencil: 0 },
                },
            };
            let mut depth: Option<ID3D12Resource> = None;
            dev.CreateCommittedResource(
                &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
                D3D12_HEAP_FLAG_NONE,
                &depth_desc,
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                Some(&depth_clear),
                &mut depth,
            )
            .map_err(|e| format!("depth buffer: {e}"))?;
            let depth = depth.ok_or("depth null")?;

            let rtv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    ..Default::default()
                })
                .map_err(|e| format!("RTV heap: {e}"))?;
            let rtv_start: unsafe extern "system" fn(*mut c_void) -> usize = vslot(rtv_heap.as_raw(), 9);
            let rtv = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: rtv_start(rtv_heap.as_raw()) };
            dev.CreateRenderTargetView(&bb, None, rtv);

            let dsv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                    NumDescriptors: 1,
                    ..Default::default()
                })
                .map_err(|e| format!("DSV heap: {e}"))?;
            let dsv_start: unsafe extern "system" fn(*mut c_void) -> usize = vslot(dsv_heap.as_raw(), 9);
            let dsv = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: dsv_start(dsv_heap.as_raw()) };
            dev.CreateDepthStencilView(&depth, None, dsv);

            // Root signature 1: 3D world with 1 root CBV (register b0)
            let serialize: D3D12SerializeRootSignatureFn = xs("D3D12SerializeRootSignature")?;
            let cbv_param = D3D12_ROOT_PARAMETER {
                ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                Anonymous: D3D12_ROOT_PARAMETER_0 {
                    Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 },
                },
                ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
            };
            let rs_world_desc = D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: 1,
                pParameters: &cbv_param,
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
                ..Default::default()
            };
            let (mut b1, mut e1) = (std::ptr::null_mut(), std::ptr::null_mut());
            if serialize(&rs_world_desc as *const _ as *const c_void, 1, &mut b1, &mut e1) < 0 || b1.is_null() {
                return Err("Serialize world RS failed".into());
            }
            let b1 = ID3DBlob::from_raw(b1);
            let rs_world: ID3D12RootSignature = dev
                .CreateRootSignature(0, std::slice::from_raw_parts(b1.GetBufferPointer() as *const u8, b1.GetBufferSize()))
                .map_err(|e| format!("CreateRootSignature(world): {e}"))?;

            // Root signature 2: 2D HUD (empty)
            let rs_hud_desc = D3D12_ROOT_SIGNATURE_DESC {
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
                ..Default::default()
            };
            let (mut b2, mut e2) = (std::ptr::null_mut(), std::ptr::null_mut());
            if serialize(&rs_hud_desc as *const _ as *const c_void, 1, &mut b2, &mut e2) < 0 || b2.is_null() {
                return Err("Serialize hud RS failed".into());
            }
            let b2 = ID3DBlob::from_raw(b2);
            let rs_hud: ID3D12RootSignature = dev
                .CreateRootSignature(0, std::slice::from_raw_parts(b2.GetBufferPointer() as *const u8, b2.GetBufferSize()))
                .map_err(|e| format!("CreateRootSignature(hud): {e}"))?;

            // 3D Input Layout
            let elem3d = [
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"POSITION\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32B32_FLOAT,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"NORMAL\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32B32_FLOAT,
                    AlignedByteOffset: 12,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"COLOR\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                    AlignedByteOffset: 24,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    AlignedByteOffset: 40,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
            ];

            // PSO 1: Opaque 3D World with depth test & write
            let mut pso_w = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
            pso_w.pRootSignature = transmute_copy(&rs_world);
            pso_w.VS = D3D12_SHADER_BYTECODE { pShaderBytecode: WORLD_VS.as_ptr() as *const c_void, BytecodeLength: WORLD_VS.len() };
            pso_w.PS = D3D12_SHADER_BYTECODE { pShaderBytecode: WORLD_PS.as_ptr() as *const c_void, BytecodeLength: WORLD_PS.len() };
            pso_w.InputLayout = D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elem3d.as_ptr(), NumElements: elem3d.len() as u32 };
            pso_w.BlendState.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
            pso_w.SampleMask = u32::MAX;
            pso_w.RasterizerState.FillMode = D3D12_FILL_MODE_SOLID;
            pso_w.RasterizerState.CullMode = D3D12_CULL_MODE_NONE;
            pso_w.RasterizerState.DepthClipEnable = true.into();
            pso_w.DepthStencilState.DepthEnable = true.into();
            pso_w.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ALL;
            pso_w.DepthStencilState.DepthFunc = D3D12_COMPARISON_FUNC_LESS;
            pso_w.DSVFormat = DXGI_FORMAT_D32_FLOAT;
            pso_w.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
            pso_w.NumRenderTargets = 1;
            pso_w.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
            pso_w.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
            let pso_world: ID3D12PipelineState = dev
                .CreateGraphicsPipelineState(&pso_w)
                .map_err(|e| format!("PSO(world): {e}"))?;

            // PSO 2: Additive 3D Glow FX (depth test ON, depth write OFF, additive blend!)
            let mut pso_f = pso_w.clone();
            pso_f.pRootSignature = transmute_copy(&rs_world);
            pso_f.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ZERO;
            let fb = &mut pso_f.BlendState.RenderTarget[0];
            fb.BlendEnable = true.into();
            fb.SrcBlend = D3D12_BLEND_SRC_ALPHA;
            fb.DestBlend = D3D12_BLEND_ONE;
            fb.BlendOp = D3D12_BLEND_OP_ADD;
            fb.SrcBlendAlpha = D3D12_BLEND_ONE;
            fb.DestBlendAlpha = D3D12_BLEND_ZERO;
            fb.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            let pso_fx: ID3D12PipelineState = dev
                .CreateGraphicsPipelineState(&pso_f)
                .map_err(|e| format!("PSO(fx): {e}"))?;

            // PSO 3: 2D Alpha HUD (no depth)
            let elem2d = [
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"POSITION\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32_FLOAT,
                    AlignedByteOffset: 0,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
                D3D12_INPUT_ELEMENT_DESC {
                    SemanticName: PCSTR(b"COLOR\0".as_ptr()),
                    Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
                    AlignedByteOffset: 8,
                    InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
                    ..Default::default()
                },
            ];
            let mut pso_h = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
            pso_h.pRootSignature = transmute_copy(&rs_hud);
            pso_h.VS = D3D12_SHADER_BYTECODE { pShaderBytecode: HUD_VS.as_ptr() as *const c_void, BytecodeLength: HUD_VS.len() };
            pso_h.PS = D3D12_SHADER_BYTECODE { pShaderBytecode: HUD_PS.as_ptr() as *const c_void, BytecodeLength: HUD_PS.len() };
            pso_h.InputLayout = D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elem2d.as_ptr(), NumElements: elem2d.len() as u32 };
            let hb = &mut pso_h.BlendState.RenderTarget[0];
            hb.BlendEnable = true.into();
            hb.SrcBlend = D3D12_BLEND_SRC_ALPHA;
            hb.DestBlend = D3D12_BLEND_INV_SRC_ALPHA;
            hb.BlendOp = D3D12_BLEND_OP_ADD;
            hb.SrcBlendAlpha = D3D12_BLEND_ONE;
            hb.DestBlendAlpha = D3D12_BLEND_INV_SRC_ALPHA;
            hb.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            hb.RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
            pso_h.SampleMask = u32::MAX;
            pso_h.RasterizerState.FillMode = D3D12_FILL_MODE_SOLID;
            pso_h.RasterizerState.CullMode = D3D12_CULL_MODE_NONE;
            pso_h.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
            pso_h.NumRenderTargets = 1;
            pso_h.RTVFormats[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
            pso_h.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
            let pso_hud: ID3D12PipelineState = dev
                .CreateGraphicsPipelineState(&pso_h)
                .map_err(|e| format!("PSO(hud): {e}"))?;

            let (cb_buf, cb_raw, cb_gpu) = create_upload_buf(&dev, size_of::<FrameCB>() as u64)?;
            let (dyn3d_buf, dyn3d_raw, dyn3d_gpu) = create_upload_buf(&dev, (MAX_DYN3D_VERTS * size_of::<Vertex3D>()) as u64)?;
            let (fx3d_buf, fx3d_raw, fx3d_gpu) = create_upload_buf(&dev, (MAX_FX3D_VERTS * size_of::<Vertex3D>()) as u64)?;
            let (hud2d_buf, hud2d_raw, hud2d_gpu) = create_upload_buf(&dev, (MAX_HUD2D_VERTS * size_of::<Vertex2D>()) as u64)?;

            let alloc: ID3D12CommandAllocator = dev
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|e| format!("alloc: {e}"))?;
            let list: ID3D12GraphicsCommandList = dev
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, Some(&pso_world))
                .map_err(|e| format!("list: {e}"))?;
            list.Close().map_err(|e| format!("list.Close: {e}"))?;
            let fence: ID3D12Fence = dev
                .CreateFence(0, D3D12_FENCE_FLAG_NONE)
                .map_err(|e| format!("fence: {e}"))?;
            let fence_ev = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());

            Ok(Self {
                dev,
                queue,
                bb,
                _depth: depth,
                _rtv_heap: rtv_heap,
                _dsv_heap: dsv_heap,
                rtv,
                dsv,
                rs_world,
                rs_hud,
                pso_world,
                pso_fx,
                pso_hud,
                alloc,
                list,
                fence,
                fence_ev,
                fence_val: 0,
                _cb_buf: cb_buf,
                cb_ptr: cb_raw as *mut FrameCB,
                cb_gpu,
                static_buf: None,
                _dyn3d_buf: dyn3d_buf,
                dyn3d_ptr: dyn3d_raw as *mut Vertex3D,
                dyn3d_gpu,
                _fx3d_buf: fx3d_buf,
                fx3d_ptr: fx3d_raw as *mut Vertex3D,
                fx3d_gpu,
                _hud2d_buf: hud2d_buf,
                hud2d_ptr: hud2d_raw as *mut Vertex2D,
                hud2d_gpu,
                frame: 0,
                wait_event,
                present_x,
            })
        }
    }

    pub fn upload_static_world(&mut self, verts: &[Vertex3D]) -> Result<(), String> {
        let bytes = (verts.len() * size_of::<Vertex3D>()) as u64;
        let (buf, ptr, gpu) = create_upload_buf(&self.dev, bytes)?;
        unsafe {
            std::ptr::copy_nonoverlapping(verts.as_ptr(), ptr as *mut Vertex3D, verts.len());
            let _ = buf.Unmap(0, None);
        }
        self.static_buf = Some((buf, gpu, verts.len() as u32));
        Ok(())
    }

    pub fn present_frame(
        &mut self,
        view_proj: Mat4,
        cam_pos: [f32; 3],
        time: f32,
        fog_rgb: [f32; 3],
        boost_glow: f32,
        dyn3d: &[Vertex3D],
        fx3d: &[Vertex3D],
        hud2d: &[Vertex2D],
    ) -> Result<(), String> {
        unsafe {
            let mut token = 0u64;
            let hr = (self.wait_event)(self.dev.as_raw(), 0, 500, std::ptr::null(), 0, &mut token);
            if hr < 0 || token == 0 {
                token = self.frame + 1;
            }

            // Update constant buffer
            *self.cb_ptr = FrameCB {
                view_proj: view_proj.0,
                cam_pos_time: [cam_pos[0], cam_pos[1], cam_pos[2], time],
                fog_params: [fog_rgb[0], fog_rgb[1], fog_rgb[2], 0.00145],
                sun_dir_glow: [0.42, 0.72, 0.55, boost_glow],
            };

            let dyn_n = dyn3d.len().min(MAX_DYN3D_VERTS);
            if dyn_n > 0 {
                std::ptr::copy_nonoverlapping(dyn3d.as_ptr(), self.dyn3d_ptr, dyn_n);
            }
            let fx_n = fx3d.len().min(MAX_FX3D_VERTS);
            if fx_n > 0 {
                std::ptr::copy_nonoverlapping(fx3d.as_ptr(), self.fx3d_ptr, fx_n);
            }
            let hud_n = hud2d.len().min(MAX_HUD2D_VERTS);
            if hud_n > 0 {
                std::ptr::copy_nonoverlapping(hud2d.as_ptr(), self.hud2d_ptr, hud_n);
            }

            self.alloc.Reset().map_err(|e| format!("alloc.Reset: {e}"))?;
            self.list.Reset(&self.alloc, Some(&self.pso_world)).map_err(|e| format!("list.Reset: {e}"))?;

            let barrier = |before, after| D3D12_RESOURCE_BARRIER {
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
                self.list.ResourceBarrier(&[barrier(
                    D3D12_RESOURCE_STATE_PRESENT,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                )]);
            }

            let l = self.list.as_raw();
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv, 0, &self.dsv);
            let clear = [fog_rgb[0] * 0.75, fog_rgb[1] * 0.75, fog_rgb[2] * 0.85, 1.0];
            vslot::<ClearRtvFn>(l, X_CLEAR_RENDER_TARGET_VIEW)(l, self.rtv, &clear, 0, std::ptr::null());
            vslot::<ClearDsvFn>(l, X_CLEAR_DEPTH_STENCIL_VIEW)(l, self.dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, 0, std::ptr::null());

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

            // PASS 1: Opaque 3D World (static circuit + megacity, then dynamic 8 ships)
            vslot::<SetRootSignatureFn>(l, X_SET_GRAPHICS_ROOT_SIGNATURE)(l, self.rs_world.as_raw());
            vslot::<SetRootCbvFn>(l, X_SET_GRAPHICS_ROOT_CBV)(l, 0, self.cb_gpu);
            self.list.SetPipelineState(&self.pso_world);

            if let Some((_, gpu, count)) = &self.static_buf {
                if *count >= 3 {
                    let view = D3D12_VERTEX_BUFFER_VIEW {
                        BufferLocation: *gpu,
                        SizeInBytes: (*count as usize * size_of::<Vertex3D>()) as u32,
                        StrideInBytes: size_of::<Vertex3D>() as u32,
                    };
                    vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                    self.list.DrawInstanced(*count, 1, 0, 0);
                }
            }
            if dyn_n >= 3 {
                let view = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.dyn3d_gpu,
                    SizeInBytes: (dyn_n * size_of::<Vertex3D>()) as u32,
                    StrideInBytes: size_of::<Vertex3D>() as u32,
                };
                vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                self.list.DrawInstanced(dyn_n as u32, 1, 0, 0);
            }

            // PASS 2: Additive 3D FX (thruster ribbons, rain streaks, boost rings, sparks)
            if fx_n >= 3 {
                self.list.SetPipelineState(&self.pso_fx);
                let view = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.fx3d_gpu,
                    SizeInBytes: (fx_n * size_of::<Vertex3D>()) as u32,
                    StrideInBytes: size_of::<Vertex3D>() as u32,
                };
                vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                self.list.DrawInstanced(fx_n as u32, 1, 0, 0);
            }

            // PASS 3: 2D Sci-Fi Racing HUD (alpha blended, no depth)
            if hud_n >= 3 {
                vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv, 0, std::ptr::null());
                vslot::<SetRootSignatureFn>(l, X_SET_GRAPHICS_ROOT_SIGNATURE)(l, self.rs_hud.as_raw());
                self.list.SetPipelineState(&self.pso_hud);
                let view = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.hud2d_gpu,
                    SizeInBytes: (hud_n * size_of::<Vertex2D>()) as u32,
                    StrideInBytes: size_of::<Vertex2D>() as u32,
                };
                vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                self.list.DrawInstanced(hud_n as u32, 1, 0, 0);
            }

            self.list.ResourceBarrier(&[barrier(
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_PRESENT,
            )]);
            self.list.Close().map_err(|e| format!("list.Close: {e}"))?;

            let cmd: ID3D12CommandList = self.list.cast().map_err(|e| format!("cast: {e}"))?;
            self.queue.ExecuteCommandLists(&[Some(cmd)]);

            let resources = [self.bb.as_raw()];
            let mut plane = [0u8; 0x40];
            plane[..8].copy_from_slice(&token.to_le_bytes());
            plane[8..12].copy_from_slice(&1u32.to_le_bytes());
            plane[0x10..0x18].copy_from_slice(&(resources.as_ptr() as u64).to_le_bytes());
            let hr = (self.present_x)(self.queue.as_raw(), 1, plane.as_ptr(), std::ptr::null());
            if hr < 0 {
                return Err(format!("PresentX hr={hr:#x}"));
            }

            self.fence_val += 1;
            self.queue.Signal(&self.fence, self.fence_val).map_err(|e| format!("Signal: {e}"))?;
            self.fence
                .SetEventOnCompletion(self.fence_val, HANDLE(self.fence_ev))
                .map_err(|e| format!("SetEvent: {e}"))?;
            let _ = WaitForSingleObject(self.fence_ev, 5000);
            self.frame += 1;
            Ok(())
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        unsafe {
            if !self.fence_ev.is_null() {
                CloseHandle(self.fence_ev);
            }
        }
    }
}
