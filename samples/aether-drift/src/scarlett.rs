// SPDX-License-Identifier: MIT
//! Xbox Series X|S (Scarlett) AA D3D12.X PBR Renderer (Hardware Textures, Stencil-Masked
//! Planar Wet-Track Mirror Reflections, HDR Quarter-Res Bloom, ACES Tone Mapping),
//! DualSense XInput bridge, and XUser/PLS runtime.

use std::ffi::c_void;
use std::mem::{size_of, transmute_copy, ManuallyDrop};
use std::path::PathBuf;

use windows::core::{Interface, PCSTR};
use windows::Win32::Foundation::{HANDLE, RECT};
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::IDXGIDevice1;

use crate::assets::{GameAssets, RawTexture};
use crate::math::Mat4;
use crate::track::{build_sky_dome_mesh, StaticWorld};

type HRESULT = i32;

pub const SCREEN_W: u32 = 1920;
pub const SCREEN_H: u32 = 1080;
const BLOOM_W: u32 = SCREEN_W / 4;
const BLOOM_H: u32 = SCREEN_H / 4;

pub const MAX_CRAFT_VERTS: usize = 48_000;
pub const MAX_REFL_VERTS: usize = 48_000;
pub const MAX_FX3D_VERTS: usize = 48_000;
pub const MAX_HUD2D_VERTS: usize = 48_000;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex3D {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub col: [f32; 4],
    pub mat: [f32; 2], // x = metallic/wetness (or scroll flag for billboards), y = emissive
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex2D {
    pub pos: [f32; 2],
    pub col: [f32; 4],
}

#[repr(C, align(256))]
#[derive(Clone, Copy)]
struct FrameCB {
    view_proj: [[f32; 4]; 4],
    cam_pos_time: [f32; 4],
    fog_params: [f32; 4],
    sun_dir_glow: [f32; 4],
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
// DualSense Input + Throttled Rumble
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
    last_low: u16,
    last_high: u16,
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
                last_low: 0,
                last_high: 0,
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

    pub fn update_rumble(&mut self, continuous_low: f32, continuous_high: f32) {
        if let Some(set) = self.set_state {
            let (pl, ph) = if self.rumble_pulse > 0.0 {
                (self.pulse_low, self.pulse_high)
            } else {
                (0.0, 0.0)
            };
            let left_motor = (continuous_low.max(pl).clamp(0.0, 1.0) * 65535.0) as u16;
            let right_motor = (continuous_high.max(ph).clamp(0.0, 1.0) * 65535.0) as u16;
            if left_motor != self.last_low || right_motor != self.last_high {
                self.last_low = left_motor;
                self.last_high = right_motor;
                let v = XInputVibration { left_motor, right_motor };
                unsafe {
                    let _ = set(0, &v);
                }
            }
        }
    }
}

fn deadzone(v: f32, dz: f32) -> f32 {
    if v.abs() < dz { 0.0 } else { ((v.abs() - dz) / (1.0 - dz)).copysign(v) }
}

// ---------------------------------------------------------------------------
// Console Identity & PLS
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
// D3D12.X AA PBR + Mirror Reflections + HDR Bloom Renderer
// ---------------------------------------------------------------------------

type D3D12CreateDeviceFn = unsafe extern "system" fn(*mut c_void, u32, *const c_void, *mut *mut c_void) -> HRESULT;
type D3D12SerializeRootSignatureFn = unsafe extern "system" fn(*const c_void, u32, *mut *mut c_void, *mut *mut c_void) -> HRESULT;
type OmSetStencilRefFn = unsafe extern "system" fn(*mut c_void, u32);
type SetDescriptorHeapsFn = unsafe extern "system" fn(*mut c_void, u32, *const *mut c_void);
type SetRootSignatureFn = unsafe extern "system" fn(*mut c_void, *mut c_void);
type SetRootTableFn = unsafe extern "system" fn(*mut c_void, u32, u64);
type SetRootConstsFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u32, u32);
type SetRootCbvFn = unsafe extern "system" fn(*mut c_void, u32, u64);
type IaSetVertexBuffersFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const D3D12_VERTEX_BUFFER_VIEW);
type OmSetRenderTargetsFn = unsafe extern "system" fn(*mut c_void, u32, *const D3D12_CPU_DESCRIPTOR_HANDLE, i32, *const D3D12_CPU_DESCRIPTOR_HANDLE);
type ClearDsvFn = unsafe extern "system" fn(*mut c_void, D3D12_CPU_DESCRIPTOR_HANDLE, D3D12_CLEAR_FLAGS, f32, u8, u32, *const RECT);
type ClearRtvFn = unsafe extern "system" fn(*mut c_void, D3D12_CPU_DESCRIPTOR_HANDLE, *const [f32; 4], u32, *const RECT);
type SetFrameIntervalXFn = unsafe extern "system" fn(*mut c_void, *mut c_void, u32, u32, u32) -> HRESULT;
type ScheduleFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32) -> HRESULT;
type WaitFrameEventXFn = unsafe extern "system" fn(*mut c_void, u32, u32, *const u8, u32, *mut u64) -> HRESULT;
type PresentXFn = unsafe extern "system" fn(*mut c_void, u32, *const u8, *const u8) -> HRESULT;

const X_OM_SET_STENCIL_REF: usize = 24;
const X_SET_DESCRIPTOR_HEAPS: usize = 32;
const X_SET_GRAPHICS_ROOT_SIGNATURE: usize = 34;
const X_SET_GRAPHICS_ROOT_TABLE: usize = 36;
const X_SET_GRAPHICS_ROOT_32BIT_CONSTS: usize = 40;
const X_SET_GRAPHICS_ROOT_CBV: usize = 42;
const X_IA_SET_VERTEX_BUFFERS: usize = 48;
const X_OM_SET_RENDER_TARGETS: usize = 50;
const X_CLEAR_DEPTH_STENCIL_VIEW: usize = 51;
const X_CLEAR_RENDER_TARGET_VIEW: usize = 52;

// SRV slot indices in our shader-visible heap
const SRV_TRACK_ALBEDO: u64 = 0;
const SRV_TRACK_PUDDLE: u64 = 1;
const SRV_BILLBOARD: u64 = 2;
const SRV_CITY_FACADE: u64 = 3;
const SRV_SKY_TWILIGHT: u64 = 4;
const SRV_SCENE_HDR: u64 = 5;
const SRV_BLOOM_A: u64 = 6;
const SRV_BLOOM_B: u64 = 7;

// Embedded compiled DXBC shaders
static V2_WORLD_VS: &[u8] = include_bytes!("../shaders/v2_world_vs.dxbc");
static V2_SKY_VS: &[u8] = include_bytes!("../shaders/v2_sky_vs.dxbc");
static V2_SKY_PS: &[u8] = include_bytes!("../shaders/v2_sky_ps.dxbc");
static V2_TRACK_PS: &[u8] = include_bytes!("../shaders/v2_track_ps.dxbc");
static V2_CRAFT_PS: &[u8] = include_bytes!("../shaders/v2_craft_ps.dxbc");
static V2_REFL_PS: &[u8] = include_bytes!("../shaders/v2_refl_ps.dxbc");
static V2_CITY_PS: &[u8] = include_bytes!("../shaders/v2_city_ps.dxbc");
static V2_BILLBOARD_PS: &[u8] = include_bytes!("../shaders/v2_billboard_ps.dxbc");
static V2_MIST_PS: &[u8] = include_bytes!("../shaders/v2_mist_ps.dxbc");
static V2_FX_PS: &[u8] = include_bytes!("../shaders/v2_fx_ps.dxbc");
static V2_FULL_VS: &[u8] = include_bytes!("../shaders/v2_full_vs.dxbc");
static V2_BLOOM_EXT_PS: &[u8] = include_bytes!("../shaders/v2_bloom_extract_ps.dxbc");
static V2_BLOOM_H_PS: &[u8] = include_bytes!("../shaders/v2_bloom_blur_h_ps.dxbc");
static V2_BLOOM_V_PS: &[u8] = include_bytes!("../shaders/v2_bloom_blur_v_ps.dxbc");
static V2_COMPOSITE_PS: &[u8] = include_bytes!("../shaders/v2_composite_ps.dxbc");
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

fn create_rt_tex(dev: &ID3D12Device, w: u32, h: u32, fmt: DXGI_FORMAT, init_state: D3D12_RESOURCE_STATES) -> Result<ID3D12Resource, String> {
    unsafe {
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: w as u64,
            Height: h,
            DepthOrArraySize: 1,
            MipLevels: 1,
            Format: fmt,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
            ..Default::default()
        };
        let mut res: Option<ID3D12Resource> = None;
        dev.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
            D3D12_HEAP_FLAG_NONE,
            &desc,
            init_state,
            None,
            &mut res,
        )
        .map_err(|e| format!("CreateRT({w}x{h}): {e}"))?;
        res.ok_or_else(|| "RT null".into())
    }
}

struct GpuBucket {
    _buf: ID3D12Resource,
    gpu: u64,
    count: u32,
}

impl GpuBucket {
    fn from_verts(dev: &ID3D12Device, verts: &[Vertex3D]) -> Result<Self, String> {
        let bytes = (verts.len().max(1) * size_of::<Vertex3D>()) as u64;
        let (buf, ptr, gpu) = create_upload_buf(dev, bytes)?;
        if !verts.is_empty() {
            unsafe {
                std::ptr::copy_nonoverlapping(verts.as_ptr(), ptr as *mut Vertex3D, verts.len());
            }
        }
        Ok(Self { _buf: buf, gpu, count: verts.len() as u32 })
    }
}

struct UploadedStaticWorld {
    track: GpuBucket,
    rails: GpuBucket,
    city: GpuBucket,
    billboards: GpuBucket,
    refl: GpuBucket,
    mist: GpuBucket,
}

pub struct Renderer {
    dev: ID3D12Device,
    queue: ID3D12CommandQueue,
    bb: ID3D12Resource,
    scene_rt: ID3D12Resource,
    bloom_a: ID3D12Resource,
    bloom_b: ID3D12Resource,
    _depth: ID3D12Resource,
    _textures: Vec<ID3D12Resource>,
    _rtv_heap: ID3D12DescriptorHeap,
    _dsv_heap: ID3D12DescriptorHeap,
    srv_heap: ID3D12DescriptorHeap,
    rtv_bb: D3D12_CPU_DESCRIPTOR_HANDLE,
    rtv_scene: D3D12_CPU_DESCRIPTOR_HANDLE,
    rtv_bloom_a: D3D12_CPU_DESCRIPTOR_HANDLE,
    rtv_bloom_b: D3D12_CPU_DESCRIPTOR_HANDLE,
    dsv: D3D12_CPU_DESCRIPTOR_HANDLE,
    srv_gpu_start: u64,
    srv_inc: u64,
    rs_world: ID3D12RootSignature,
    rs_post: ID3D12RootSignature,
    rs_hud: ID3D12RootSignature,
    pso_sky: ID3D12PipelineState,
    pso_track: ID3D12PipelineState,
    pso_city: ID3D12PipelineState,
    pso_billboard: ID3D12PipelineState,
    pso_craft: ID3D12PipelineState,
    pso_refl: ID3D12PipelineState,
    pso_mist: ID3D12PipelineState,
    pso_fx: ID3D12PipelineState,
    pso_bloom_ext: ID3D12PipelineState,
    pso_bloom_h: ID3D12PipelineState,
    pso_bloom_v: ID3D12PipelineState,
    pso_composite: ID3D12PipelineState,
    pso_hud: ID3D12PipelineState,
    alloc: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_ev: *mut c_void,
    fence_val: u64,
    _cb_buf: ID3D12Resource,
    cb_ptr: *mut FrameCB,
    cb_gpu: u64,
    sky_bucket: GpuBucket,
    static_world: Option<UploadedStaticWorld>,
    _craft_buf: ID3D12Resource,
    craft_ptr: *mut Vertex3D,
    craft_gpu: u64,
    _dyn_refl_buf: ID3D12Resource,
    dyn_refl_ptr: *mut Vertex3D,
    dyn_refl_gpu: u64,
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

            // Render Targets: Backbuffer (RGBA8), Scene HDR (RGBA16F), Bloom A & B (Quarter-Res RGBA16F)
            let bb = create_rt_tex(&dev, SCREEN_W, SCREEN_H, DXGI_FORMAT_R8G8B8A8_UNORM, D3D12_RESOURCE_STATE_RENDER_TARGET)?;
            let scene_rt = create_rt_tex(&dev, SCREEN_W, SCREEN_H, DXGI_FORMAT_R16G16B16A16_FLOAT, D3D12_RESOURCE_STATE_RENDER_TARGET)?;
            let bloom_a = create_rt_tex(&dev, BLOOM_W, BLOOM_H, DXGI_FORMAT_R16G16B16A16_FLOAT, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)?;
            let bloom_b = create_rt_tex(&dev, BLOOM_W, BLOOM_H, DXGI_FORMAT_R16G16B16A16_FLOAT, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)?;

            // Depth-Stencil Buffer (D24_UNORM_S8_UINT -> hardware 8-bit stencil for wet-track mirror reflections!)
            let depth_desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                Width: SCREEN_W as u64,
                Height: SCREEN_H,
                DepthOrArraySize: 1,
                MipLevels: 1,
                Format: DXGI_FORMAT_D24_UNORM_S8_UINT,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Flags: D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
                ..Default::default()
            };
            let depth_clear = D3D12_CLEAR_VALUE {
                Format: DXGI_FORMAT_D24_UNORM_S8_UINT,
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
            .map_err(|e| format!("depth-stencil: {e}"))?;
            let depth = depth.ok_or("depth null")?;

            // Descriptor Heaps: RTV (4), DSV (1), Shader-Visible SRV (16)
            let rtv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 4,
                    ..Default::default()
                })
                .map_err(|e| format!("RTV heap: {e}"))?;
            let rtv_base = vslot::<unsafe extern "system" fn(*mut c_void) -> usize>(rtv_heap.as_raw(), 9)(rtv_heap.as_raw());
            let rtv_inc = dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_RTV) as usize;
            let rtv_bb = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: rtv_base };
            let rtv_scene = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: rtv_base + rtv_inc };
            let rtv_bloom_a = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: rtv_base + rtv_inc * 2 };
            let rtv_bloom_b = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: rtv_base + rtv_inc * 3 };
            dev.CreateRenderTargetView(&bb, None, rtv_bb);
            dev.CreateRenderTargetView(&scene_rt, None, rtv_scene);
            dev.CreateRenderTargetView(&bloom_a, None, rtv_bloom_a);
            dev.CreateRenderTargetView(&bloom_b, None, rtv_bloom_b);

            let dsv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                    NumDescriptors: 1,
                    ..Default::default()
                })
                .map_err(|e| format!("DSV heap: {e}"))?;
            let dsv = D3D12_CPU_DESCRIPTOR_HANDLE {
                ptr: vslot::<unsafe extern "system" fn(*mut c_void) -> usize>(dsv_heap.as_raw(), 9)(dsv_heap.as_raw()),
            };
            dev.CreateDepthStencilView(&depth, None, dsv);

            let srv_heap: ID3D12DescriptorHeap = dev
                .CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                    NumDescriptors: 16,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                    ..Default::default()
                })
                .map_err(|e| format!("SRV heap: {e}"))?;
            let srv_cpu_start = vslot::<unsafe extern "system" fn(*mut c_void) -> u64>(srv_heap.as_raw(), 9)(srv_heap.as_raw());
            let srv_gpu_start = vslot::<unsafe extern "system" fn(*mut c_void) -> u64>(srv_heap.as_raw(), 10)(srv_heap.as_raw());
            let srv_inc = dev.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV) as u64;

            let make_srv = |slot_idx: u64, res: &ID3D12Resource, fmt: DXGI_FORMAT| {
                let mut desc = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: fmt,
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    ..Default::default()
                };
                desc.Anonymous.Texture2D.MipLevels = 1;
                let cpu = D3D12_CPU_DESCRIPTOR_HANDLE { ptr: (srv_cpu_start + slot_idx * srv_inc) as usize };
                dev.CreateShaderResourceView(res, Some(&desc), cpu);
            };
            make_srv(SRV_SCENE_HDR, &scene_rt, DXGI_FORMAT_R16G16B16A16_FLOAT);
            make_srv(SRV_BLOOM_A, &bloom_a, DXGI_FORMAT_R16G16B16A16_FLOAT);
            make_srv(SRV_BLOOM_B, &bloom_b, DXGI_FORMAT_R16G16B16A16_FLOAT);

            // Command Allocator, List & Fence (also used for one-shot texture upload!)
            let alloc: ID3D12CommandAllocator = dev
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .map_err(|e| format!("alloc: {e}"))?;
            let list: ID3D12GraphicsCommandList = dev
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None)
                .map_err(|e| format!("list: {e}"))?;
            let fence: ID3D12Fence = dev
                .CreateFence(0, D3D12_FENCE_FLAG_NONE)
                .map_err(|e| format!("fence: {e}"))?;
            let fence_ev = CreateEventW(std::ptr::null(), 0, 0, std::ptr::null());

            // Decode & Upload all 5 embedded AA PBR textures + Twilight Sky Dome!
            let raw_texs = GameAssets::decode_textures();
            let mut textures = Vec::with_capacity(raw_texs.len());
            let mut staging_keep = Vec::with_capacity(raw_texs.len());
            for (idx, rt) in raw_texs.iter().enumerate() {
                let (tex_res, stg) = upload_texture_2d(&dev, &list, rt)?;
                make_srv(idx as u64, &tex_res, DXGI_FORMAT_R8G8B8A8_UNORM);
                textures.push(tex_res);
                staging_keep.push(stg);
            }
            list.Close().map_err(|e| format!("upload list.Close: {e}"))?;
            let cmd: ID3D12CommandList = list.cast().map_err(|e| format!("cast: {e}"))?;
            queue.ExecuteCommandLists(&[Some(cmd)]);
            let fence_val = 1u64;
            queue.Signal(&fence, fence_val).map_err(|e| format!("upload Signal: {e}"))?;
            fence.SetEventOnCompletion(fence_val, HANDLE(fence_ev)).map_err(|e| format!("upload SetEvent: {e}"))?;
            let _ = WaitForSingleObject(fence_ev, 5000);
            drop(staging_keep);

            // Root Signatures (with built-in Static Samplers s0=WRAP, s1=CLAMP!)
            let serialize: D3D12SerializeRootSignatureFn = xs("D3D12SerializeRootSignature")?;
            let static_samplers = [
                D3D12_STATIC_SAMPLER_DESC {
                    Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                    AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                    AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                    ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
                    MaxLOD: f32::MAX,
                    ShaderRegister: 0,
                    RegisterSpace: 0,
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                    ..Default::default()
                },
                D3D12_STATIC_SAMPLER_DESC {
                    Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
                    MaxLOD: f32::MAX,
                    ShaderRegister: 1,
                    RegisterSpace: 0,
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                    ..Default::default()
                },
            ];

            let r_t0 = [D3D12_DESCRIPTOR_RANGE { RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV, NumDescriptors: 1, BaseShaderRegister: 0, OffsetInDescriptorsFromTableStart: 0, ..Default::default() }];
            let r_t1 = [D3D12_DESCRIPTOR_RANGE { RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV, NumDescriptors: 1, BaseShaderRegister: 1, OffsetInDescriptorsFromTableStart: 0, ..Default::default() }];
            let r_t2 = [D3D12_DESCRIPTOR_RANGE { RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV, NumDescriptors: 1, BaseShaderRegister: 2, OffsetInDescriptorsFromTableStart: 0, ..Default::default() }];

            let world_params = [
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: r_t0.as_ptr() } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: r_t1.as_ptr() } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: r_t2.as_ptr() } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
            ];
            let rs_world = make_rs(&dev, serialize, &world_params, &static_samplers, D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT)?;

            let post_params = [
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_32BIT_CONSTANTS,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { Constants: D3D12_ROOT_CONSTANTS { ShaderRegister: 0, RegisterSpace: 0, Num32BitValues: 4 } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: r_t0.as_ptr() } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 { DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE { NumDescriptorRanges: 1, pDescriptorRanges: r_t1.as_ptr() } },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
            ];
            let rs_post = make_rs(&dev, serialize, &post_params, &static_samplers, D3D12_ROOT_SIGNATURE_FLAG_NONE)?;
            let rs_hud = make_rs(&dev, serialize, &[], &[], D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT)?;

            // Input Layout for 3D Vertex3D (56 bytes)
            let elem3d = [
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"POSITION\0".as_ptr()), Format: DXGI_FORMAT_R32G32B32_FLOAT, AlignedByteOffset: 0, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"NORMAL\0".as_ptr()), Format: DXGI_FORMAT_R32G32B32_FLOAT, AlignedByteOffset: 12, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()), SemanticIndex: 0, Format: DXGI_FORMAT_R32G32_FLOAT, AlignedByteOffset: 24, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"COLOR\0".as_ptr()), Format: DXGI_FORMAT_R32G32B32A32_FLOAT, AlignedByteOffset: 32, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"TEXCOORD\0".as_ptr()), SemanticIndex: 1, Format: DXGI_FORMAT_R32G32_FLOAT, AlignedByteOffset: 48, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
            ];

            let base_hdr_pso = |vs: &[u8], ps: &[u8]| -> D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                let mut d = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
                d.pRootSignature = transmute_copy(&rs_world);
                d.VS = D3D12_SHADER_BYTECODE { pShaderBytecode: vs.as_ptr() as *const c_void, BytecodeLength: vs.len() };
                d.PS = D3D12_SHADER_BYTECODE { pShaderBytecode: ps.as_ptr() as *const c_void, BytecodeLength: ps.len() };
                d.InputLayout = D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: elem3d.as_ptr(), NumElements: elem3d.len() as u32 };
                d.BlendState.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
                d.SampleMask = u32::MAX;
                d.RasterizerState.FillMode = D3D12_FILL_MODE_SOLID;
                d.RasterizerState.CullMode = D3D12_CULL_MODE_NONE;
                d.RasterizerState.DepthClipEnable = true.into();
                d.DepthStencilState.DepthEnable = true.into();
                d.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ALL;
                d.DepthStencilState.DepthFunc = D3D12_COMPARISON_FUNC_LESS;
                d.DSVFormat = DXGI_FORMAT_D24_UNORM_S8_UINT;
                d.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
                d.NumRenderTargets = 1;
                d.RTVFormats[0] = DXGI_FORMAT_R16G16B16A16_FLOAT;
                d.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
                d
            };

            // PSO: Sky Dome (depth test LESS_EQUAL, depth write OFF)
            let mut d_sky = base_hdr_pso(V2_SKY_VS, V2_SKY_PS);
            d_sky.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ZERO;
            d_sky.DepthStencilState.DepthFunc = D3D12_COMPARISON_FUNC_LESS_EQUAL;
            let pso_sky = dev.CreateGraphicsPipelineState(&d_sky).map_err(|e| format!("PSO(sky): {e}"))?;

            // PSO: Track Surface (writes Stencil = 1 so mirror reflections stay strictly on wet asphalt!)
            let mut d_track = base_hdr_pso(V2_WORLD_VS, V2_TRACK_PS);
            d_track.DepthStencilState.StencilEnable = true.into();
            d_track.DepthStencilState.StencilReadMask = 0xFF;
            d_track.DepthStencilState.StencilWriteMask = 0xFF;
            let st_write = D3D12_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D12_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                StencilPassOp: D3D12_STENCIL_OP_REPLACE,
                StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            };
            d_track.DepthStencilState.FrontFace = st_write;
            d_track.DepthStencilState.BackFace = st_write;
            let pso_track = dev.CreateGraphicsPipelineState(&d_track).map_err(|e| format!("PSO(track): {e}"))?;

            // PSO: City Skyscrapers, Billboards, Sculpted Blender Hovercrafts
            let pso_city = dev.CreateGraphicsPipelineState(&base_hdr_pso(V2_WORLD_VS, V2_CITY_PS)).map_err(|e| format!("PSO(city): {e}"))?;
            let pso_billboard = dev.CreateGraphicsPipelineState(&base_hdr_pso(V2_WORLD_VS, V2_BILLBOARD_PS)).map_err(|e| format!("PSO(billboard): {e}"))?;
            let pso_craft = dev.CreateGraphicsPipelineState(&base_hdr_pso(V2_WORLD_VS, V2_CRAFT_PS)).map_err(|e| format!("PSO(craft): {e}"))?;

            // PSO: True Wet-Track Mirror Reflections (Stencil == 1 ONLY, Depth OFF, Additive HDR blend!)
            let mut d_refl = base_hdr_pso(V2_WORLD_VS, V2_REFL_PS);
            d_refl.DepthStencilState.DepthEnable = false.into();
            d_refl.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ZERO;
            d_refl.DepthStencilState.StencilEnable = true.into();
            d_refl.DepthStencilState.StencilReadMask = 0xFF;
            d_refl.DepthStencilState.StencilWriteMask = 0x00;
            let st_test = D3D12_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D12_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                StencilPassOp: D3D12_STENCIL_OP_KEEP,
                StencilFunc: D3D12_COMPARISON_FUNC_EQUAL,
            };
            d_refl.DepthStencilState.FrontFace = st_test;
            d_refl.DepthStencilState.BackFace = st_test;
            let rb = &mut d_refl.BlendState.RenderTarget[0];
            rb.BlendEnable = true.into();
            rb.SrcBlend = D3D12_BLEND_ONE;
            rb.DestBlend = D3D12_BLEND_ONE;
            rb.BlendOp = D3D12_BLEND_OP_ADD;
            rb.SrcBlendAlpha = D3D12_BLEND_ONE;
            rb.DestBlendAlpha = D3D12_BLEND_ZERO;
            rb.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            let pso_refl = dev.CreateGraphicsPipelineState(&d_refl).map_err(|e| format!("PSO(refl): {e}"))?;

            // PSO: Volumetric Swirling Megacity Mist Banks (Depth test ON, write OFF, standard Alpha blend!)
            let mut d_mist = base_hdr_pso(V2_WORLD_VS, V2_MIST_PS);
            d_mist.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ZERO;
            let mb = &mut d_mist.BlendState.RenderTarget[0];
            mb.BlendEnable = true.into();
            mb.SrcBlend = D3D12_BLEND_SRC_ALPHA;
            mb.DestBlend = D3D12_BLEND_INV_SRC_ALPHA;
            mb.BlendOp = D3D12_BLEND_OP_ADD;
            mb.SrcBlendAlpha = D3D12_BLEND_ONE;
            mb.DestBlendAlpha = D3D12_BLEND_INV_SRC_ALPHA;
            mb.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            let pso_mist = dev.CreateGraphicsPipelineState(&d_mist).map_err(|e| format!("PSO(mist): {e}"))?;

            // PSO: Volumetric Additive 3D FX (Depth test ON, write OFF, SrcAlpha + One)
            let mut d_fx = base_hdr_pso(V2_WORLD_VS, V2_FX_PS);
            d_fx.DepthStencilState.DepthWriteMask = D3D12_DEPTH_WRITE_MASK_ZERO;
            let fb = &mut d_fx.BlendState.RenderTarget[0];
            fb.BlendEnable = true.into();
            fb.SrcBlend = D3D12_BLEND_SRC_ALPHA;
            fb.DestBlend = D3D12_BLEND_ONE;
            fb.BlendOp = D3D12_BLEND_OP_ADD;
            fb.SrcBlendAlpha = D3D12_BLEND_ONE;
            fb.DestBlendAlpha = D3D12_BLEND_ZERO;
            fb.BlendOpAlpha = D3D12_BLEND_OP_ADD;
            let pso_fx = dev.CreateGraphicsPipelineState(&d_fx).map_err(|e| format!("PSO(fx): {e}"))?;

            // Post-Processing PSOs (Bloom Extract, Blur H, Blur V, Final ACES Composite)
            let make_post_pso = |ps: &[u8], rt_fmt: DXGI_FORMAT| -> Result<ID3D12PipelineState, String> {
                let mut d = D3D12_GRAPHICS_PIPELINE_STATE_DESC::default();
                d.pRootSignature = transmute_copy(&rs_post);
                d.VS = D3D12_SHADER_BYTECODE { pShaderBytecode: V2_FULL_VS.as_ptr() as *const c_void, BytecodeLength: V2_FULL_VS.len() };
                d.PS = D3D12_SHADER_BYTECODE { pShaderBytecode: ps.as_ptr() as *const c_void, BytecodeLength: ps.len() };
                d.BlendState.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
                d.SampleMask = u32::MAX;
                d.RasterizerState.FillMode = D3D12_FILL_MODE_SOLID;
                d.RasterizerState.CullMode = D3D12_CULL_MODE_NONE;
                d.PrimitiveTopologyType = D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE;
                d.NumRenderTargets = 1;
                d.RTVFormats[0] = rt_fmt;
                d.SampleDesc = DXGI_SAMPLE_DESC { Count: 1, Quality: 0 };
                dev.CreateGraphicsPipelineState(&d).map_err(|e| format!("PSO(post): {e}"))
            };
            let pso_bloom_ext = make_post_pso(V2_BLOOM_EXT_PS, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
            let pso_bloom_h = make_post_pso(V2_BLOOM_H_PS, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
            let pso_bloom_v = make_post_pso(V2_BLOOM_V_PS, DXGI_FORMAT_R16G16B16A16_FLOAT)?;
            let pso_composite = make_post_pso(V2_COMPOSITE_PS, DXGI_FORMAT_R8G8B8A8_UNORM)?;

            // PSO: 2D Glassmorphism HUD (drawn onto RGBA8 backbuffer after ACES composite!)
            let elem2d = [
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"POSITION\0".as_ptr()), Format: DXGI_FORMAT_R32G32_FLOAT, AlignedByteOffset: 0, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
                D3D12_INPUT_ELEMENT_DESC { SemanticName: PCSTR(b"COLOR\0".as_ptr()), Format: DXGI_FORMAT_R32G32B32A32_FLOAT, AlignedByteOffset: 8, InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA, ..Default::default() },
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
            let pso_hud = dev.CreateGraphicsPipelineState(&pso_h).map_err(|e| format!("PSO(hud): {e}"))?;

            // Persistent-mapped dynamic buffers + Sky Dome sphere
            let (cb_buf, cb_raw, cb_gpu) = create_upload_buf(&dev, size_of::<FrameCB>() as u64)?;
            let sky_bucket = GpuBucket::from_verts(&dev, &build_sky_dome_mesh())?;
            let (craft_buf, craft_raw, craft_gpu) = create_upload_buf(&dev, (MAX_CRAFT_VERTS * size_of::<Vertex3D>()) as u64)?;
            let (dyn_refl_buf, dyn_refl_raw, dyn_refl_gpu) = create_upload_buf(&dev, (MAX_REFL_VERTS * size_of::<Vertex3D>()) as u64)?;
            let (fx3d_buf, fx3d_raw, fx3d_gpu) = create_upload_buf(&dev, (MAX_FX3D_VERTS * size_of::<Vertex3D>()) as u64)?;
            let (hud2d_buf, hud2d_raw, hud2d_gpu) = create_upload_buf(&dev, (MAX_HUD2D_VERTS * size_of::<Vertex2D>()) as u64)?;

            Ok(Self {
                dev,
                queue,
                bb,
                scene_rt,
                bloom_a,
                bloom_b,
                _depth: depth,
                _textures: textures,
                _rtv_heap: rtv_heap,
                _dsv_heap: dsv_heap,
                srv_heap,
                rtv_bb,
                rtv_scene,
                rtv_bloom_a,
                rtv_bloom_b,
                dsv,
                srv_gpu_start,
                srv_inc,
                rs_world,
                rs_post,
                rs_hud,
                pso_sky,
                pso_track,
                pso_city,
                pso_billboard,
                pso_craft,
                pso_refl,
                pso_mist,
                pso_fx,
                pso_bloom_ext,
                pso_bloom_h,
                pso_bloom_v,
                pso_composite,
                pso_hud,
                alloc,
                list,
                fence,
                fence_ev,
                fence_val,
                _cb_buf: cb_buf,
                cb_ptr: cb_raw as *mut FrameCB,
                cb_gpu,
                sky_bucket,
                static_world: None,
                _craft_buf: craft_buf,
                craft_ptr: craft_raw as *mut Vertex3D,
                craft_gpu,
                _dyn_refl_buf: dyn_refl_buf,
                dyn_refl_ptr: dyn_refl_raw as *mut Vertex3D,
                dyn_refl_gpu,
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

    pub fn upload_static_world(&mut self, world: &StaticWorld) -> Result<(), String> {
        self.static_world = Some(UploadedStaticWorld {
            track: GpuBucket::from_verts(&self.dev, &world.track_verts)?,
            rails: GpuBucket::from_verts(&self.dev, &world.rail_verts)?,
            city: GpuBucket::from_verts(&self.dev, &world.city_verts)?,
            billboards: GpuBucket::from_verts(&self.dev, &world.billboard_verts)?,
            refl: GpuBucket::from_verts(&self.dev, &world.refl_verts)?,
            mist: GpuBucket::from_verts(&self.dev, &world.mist_verts)?,
        });
        Ok(())
    }

    #[inline]
    fn srv(&self, slot: u64) -> u64 {
        self.srv_gpu_start + slot * self.srv_inc
    }

    pub fn present_frame(
        &mut self,
        view_proj: Mat4,
        cam_pos: [f32; 3],
        time: f32,
        fog_rgb: [f32; 3],
        boost_glow: f32,
        craft3d: &[Vertex3D],
        dyn_refl3d: &[Vertex3D],
        fx3d: &[Vertex3D],
        hud2d: &[Vertex2D],
    ) -> Result<(), String> {
        unsafe {
            let mut token = 0u64;
            let hr = (self.wait_event)(self.dev.as_raw(), 0, 500, std::ptr::null(), 0, &mut token);
            if hr < 0 || token == 0 {
                token = self.frame + 1;
            }

            // Update hardware GPU FrameCB
            *self.cb_ptr = FrameCB {
                view_proj: view_proj.0,
                cam_pos_time: [cam_pos[0], cam_pos[1], cam_pos[2], time],
                fog_params: [fog_rgb[0], fog_rgb[1], fog_rgb[2], 0.00125],
                sun_dir_glow: [0.46, 0.52, 0.72, boost_glow],
            };

            let craft_n = craft3d.len().min(MAX_CRAFT_VERTS);
            if craft_n > 0 {
                std::ptr::copy_nonoverlapping(craft3d.as_ptr(), self.craft_ptr, craft_n);
            }
            let drefl_n = dyn_refl3d.len().min(MAX_REFL_VERTS);
            if drefl_n > 0 {
                std::ptr::copy_nonoverlapping(dyn_refl3d.as_ptr(), self.dyn_refl_ptr, drefl_n);
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
            self.list.Reset(&self.alloc, Some(&self.pso_sky)).map_err(|e| format!("list.Reset: {e}"))?;

            let l = self.list.as_raw();
            let barrier = |res: &ID3D12Resource, before, after| D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                        pResource: transmute_copy(res),
                        Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                        StateBefore: before,
                        StateAfter: after,
                    }),
                },
            };

            if self.frame > 0 {
                self.list.ResourceBarrier(&[
                    barrier(&self.bb, D3D12_RESOURCE_STATE_PRESENT, D3D12_RESOURCE_STATE_RENDER_TARGET),
                    barrier(&self.scene_rt, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                ]);
            }

            let heaps = [self.srv_heap.as_raw()];
            vslot::<SetDescriptorHeapsFn>(l, X_SET_DESCRIPTOR_HEAPS)(l, 1, heaps.as_ptr());

            let set_vp = |w: u32, h: u32| {
                self.list.RSSetViewports(&[D3D12_VIEWPORT {
                    TopLeftX: 0.0,
                    TopLeftY: 0.0,
                    Width: w as f32,
                    Height: h as f32,
                    MinDepth: 0.0,
                    MaxDepth: 1.0,
                }]);
                self.list.RSSetScissorRects(&[RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }]);
            };
            let draw_vb3 = |gpu: u64, count: u32| {
                if count >= 3 {
                    let view = D3D12_VERTEX_BUFFER_VIEW {
                        BufferLocation: gpu,
                        SizeInBytes: (count as usize * size_of::<Vertex3D>()) as u32,
                        StrideInBytes: size_of::<Vertex3D>() as u32,
                    };
                    vslot::<IaSetVertexBuffersFn>(l, X_IA_SET_VERTEX_BUFFERS)(l, 0, 1, &view);
                    self.list.DrawInstanced(count, 1, 0, 0);
                }
            };

            // =====================================================================
            // SCENE HDR RENDER PASS (1920x1080 RGBA16F + D24_S8 Hardware Stencil)
            // =====================================================================
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv_scene, 0, &self.dsv);
            let clear = [fog_rgb[0] * 0.65, fog_rgb[1] * 0.65, fog_rgb[2] * 0.85, 1.0];
            vslot::<ClearRtvFn>(l, X_CLEAR_RENDER_TARGET_VIEW)(l, self.rtv_scene, &clear, 0, std::ptr::null());
            vslot::<ClearDsvFn>(l, X_CLEAR_DEPTH_STENCIL_VIEW)(
                l,
                self.dsv,
                D3D12_CLEAR_FLAG_DEPTH | D3D12_CLEAR_FLAG_STENCIL,
                1.0,
                0,
                0,
                std::ptr::null(),
            );
            set_vp(SCREEN_W, SCREEN_H);
            self.list.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);

            vslot::<SetRootSignatureFn>(l, X_SET_GRAPHICS_ROOT_SIGNATURE)(l, self.rs_world.as_raw());
            vslot::<SetRootCbvFn>(l, X_SET_GRAPHICS_ROOT_CBV)(l, 0, self.cb_gpu);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 2, self.srv(SRV_TRACK_PUDDLE));
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 3, self.srv(SRV_SKY_TWILIGHT));

            // 1. Twilight Storm-Cloud Sky Dome
            self.list.SetPipelineState(&self.pso_sky);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_SKY_TWILIGHT));
            draw_vb3(self.sky_bucket.gpu, self.sky_bucket.count);

            if let Some(sw) = &self.static_world {
                // 2. Cyberpunk Megacity Blender Skyscraper Spires (textured illuminated windows!)
                self.list.SetPipelineState(&self.pso_city);
                vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_CITY_FACADE));
                draw_vb3(sw.city.gpu, sw.city.count);

                // 3. Wet Metallic Track Surface (albedo + puddle reflection + writes Stencil = 1!)
                vslot::<OmSetStencilRefFn>(l, X_OM_SET_STENCIL_REF)(l, 1);
                self.list.SetPipelineState(&self.pso_track);
                vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_TRACK_ALBEDO));
                draw_vb3(sw.track.gpu, sw.track.count);

                // 4. TRUE PLANAR WET-TRACK MIRROR REFLECTIONS (Stencil == 1 ONLY! Ships, plumes, rail LEDs, billboards!)
                self.list.SetPipelineState(&self.pso_refl);
                draw_vb3(sw.refl.gpu, sw.refl.count);
                draw_vb3(self.dyn_refl_gpu, drefl_n as u32);

                // 5. Trackside Metallic Rails, Neon LED Tubes, Boost Chevrons & Overhead Arches
                self.list.SetPipelineState(&self.pso_craft);
                draw_vb3(sw.rails.gpu, sw.rails.count);

                // 6. Glowing Cyberpunk Billboards (NEO-KYOTO, VORTEX ENERGY, CHRONOS TECN, AETHER-1) & Animated Turn Chevrons
                self.list.SetPipelineState(&self.pso_billboard);
                vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_BILLBOARD));
                draw_vb3(sw.billboards.gpu, sw.billboards.count);
            }

            // 7. All 8 Sculpted Blender 5.2 Hovercrafts (PBR metallic clearcoat + sky reflections + glowing nacelle rims!)
            self.list.SetPipelineState(&self.pso_craft);
            draw_vb3(self.craft_gpu, craft_n as u32);

            // 7b. Layered Volumetric Swirling Megacity Mist Cloud Banks weaving between skyscrapers!
            if let Some(sw) = &self.static_world {
                self.list.SetPipelineState(&self.pso_mist);
                draw_vb3(sw.mist.gpu, sw.mist.count);
            }

            // 8. Volumetric Additive FX (blazing dual plasma plumes, wake ribbons, rain streaks, sparks)
            self.list.SetPipelineState(&self.pso_fx);
            draw_vb3(self.fx3d_gpu, fx_n as u32);

            // =====================================================================
            // HDR BLOOM POST-PROCESSING (Quarter-Res Bright Extract + Separable Blur)
            // =====================================================================
            self.list.ResourceBarrier(&[
                barrier(&self.scene_rt, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
                barrier(&self.bloom_a, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
            ]);

            vslot::<SetRootSignatureFn>(l, X_SET_GRAPHICS_ROOT_SIGNATURE)(l, self.rs_post.as_raw());
            set_vp(BLOOM_W, BLOOM_H);

            // 9a. Bright-pass extract -> bloom_a
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv_bloom_a, 0, std::ptr::null());
            self.list.SetPipelineState(&self.pso_bloom_ext);
            let ext_c = [1.0f32 / SCREEN_W as f32, 1.0f32 / SCREEN_H as f32, 0.0f32, 0.0f32];
            vslot::<SetRootConstsFn>(l, X_SET_GRAPHICS_ROOT_32BIT_CONSTS)(l, 0, 4, ext_c.as_ptr() as *const u32, 0);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_SCENE_HDR));
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 2, self.srv(SRV_SCENE_HDR));
            self.list.DrawInstanced(3, 1, 0, 0);

            // 9b. Horizontal Gaussian blur (bloom_a -> bloom_b)
            self.list.ResourceBarrier(&[
                barrier(&self.bloom_a, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
                barrier(&self.bloom_b, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
            ]);
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv_bloom_b, 0, std::ptr::null());
            self.list.SetPipelineState(&self.pso_bloom_h);
            let blur_c = [1.0f32 / BLOOM_W as f32, 1.0f32 / BLOOM_H as f32, 0.0f32, 0.0f32];
            vslot::<SetRootConstsFn>(l, X_SET_GRAPHICS_ROOT_32BIT_CONSTS)(l, 0, 4, blur_c.as_ptr() as *const u32, 0);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_BLOOM_A));
            self.list.DrawInstanced(3, 1, 0, 0);

            // 9c. Vertical Gaussian blur (bloom_b -> bloom_a)
            self.list.ResourceBarrier(&[
                barrier(&self.bloom_b, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
                barrier(&self.bloom_a, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
            ]);
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv_bloom_a, 0, std::ptr::null());
            self.list.SetPipelineState(&self.pso_bloom_v);
            vslot::<SetRootConstsFn>(l, X_SET_GRAPHICS_ROOT_32BIT_CONSTS)(l, 0, 4, blur_c.as_ptr() as *const u32, 0);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_BLOOM_B));
            self.list.DrawInstanced(3, 1, 0, 0);

            // 9d. Final Composite (Scene HDR + Bloom Glow + Speed Chromatic Vignette + ACES Tonemap -> Backbuffer!)
            self.list.ResourceBarrier(&[
                barrier(&self.bloom_a, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
            ]);
            set_vp(SCREEN_W, SCREEN_H);
            vslot::<OmSetRenderTargetsFn>(l, X_OM_SET_RENDER_TARGETS)(l, 1, &self.rtv_bb, 0, std::ptr::null());
            self.list.SetPipelineState(&self.pso_composite);
            let comp_c = [1.0f32 / SCREEN_W as f32, 1.0f32 / SCREEN_H as f32, 0.78f32 + boost_glow * 0.30, boost_glow];
            vslot::<SetRootConstsFn>(l, X_SET_GRAPHICS_ROOT_32BIT_CONSTS)(l, 0, 4, comp_c.as_ptr() as *const u32, 0);
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 1, self.srv(SRV_SCENE_HDR));
            vslot::<SetRootTableFn>(l, X_SET_GRAPHICS_ROOT_TABLE)(l, 2, self.srv(SRV_BLOOM_A));
            self.list.DrawInstanced(3, 1, 0, 0);

            // =====================================================================
            // 10. Crisp 2D Glassmorphism Racing HUD (drawn post-tonemap onto Backbuffer!)
            // =====================================================================
            if hud_n >= 3 {
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
                &self.bb,
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

unsafe fn make_rs(
    dev: &ID3D12Device,
    serialize: D3D12SerializeRootSignatureFn,
    params: &[D3D12_ROOT_PARAMETER],
    samplers: &[D3D12_STATIC_SAMPLER_DESC],
    flags: D3D12_ROOT_SIGNATURE_FLAGS,
) -> Result<ID3D12RootSignature, String> {
    let desc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: params.len() as u32,
        pParameters: if params.is_empty() { std::ptr::null() } else { params.as_ptr() },
        NumStaticSamplers: samplers.len() as u32,
        pStaticSamplers: if samplers.is_empty() { std::ptr::null() } else { samplers.as_ptr() },
        Flags: flags,
    };
    let (mut blob, mut err) = (std::ptr::null_mut(), std::ptr::null_mut());
    if serialize(&desc as *const _ as *const c_void, 1, &mut blob, &mut err) < 0 || blob.is_null() {
        return Err("D3D12SerializeRootSignature failed".into());
    }
    let blob = ID3DBlob::from_raw(blob);
    dev.CreateRootSignature(0, std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()))
        .map_err(|e| format!("CreateRootSignature: {e}"))
}

unsafe fn upload_texture_2d(
    dev: &ID3D12Device,
    list: &ID3D12GraphicsCommandList,
    rt: &RawTexture,
) -> Result<(ID3D12Resource, ID3D12Resource), String> {
    let tex_desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: rt.width as u64,
        Height: rt.height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        ..Default::default()
    };
    let mut tex: Option<ID3D12Resource> = None;
    dev.CreateCommittedResource(
        &D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() },
        D3D12_HEAP_FLAG_NONE,
        &tex_desc,
        D3D12_RESOURCE_STATE_COPY_DEST,
        None,
        &mut tex,
    )
    .map_err(|e| format!("CreateTex({}x{}): {e}", rt.width, rt.height))?;
    let tex = tex.ok_or("tex null")?;

    let row_bytes = (rt.width * 4) as usize;
    let row_pitch = (row_bytes + 255) & !255;
    let total_bytes = (row_pitch * rt.height as usize) as u64;
    let (stg, ptr, _) = create_upload_buf(dev, total_bytes)?;
    for y in 0..rt.height as usize {
        std::ptr::copy_nonoverlapping(
            rt.rgba.as_ptr().add(y * row_bytes),
            ptr.add(y * row_pitch),
            row_bytes,
        );
    }

    let dst_loc = D3D12_TEXTURE_COPY_LOCATION {
        pResource: transmute_copy(&tex),
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
    };
    let src_loc = D3D12_TEXTURE_COPY_LOCATION {
        pResource: transmute_copy(&stg),
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
            PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                Offset: 0,
                Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                    Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                    Width: rt.width,
                    Height: rt.height,
                    Depth: 1,
                    RowPitch: row_pitch as u32,
                },
            },
        },
    };
    list.CopyTextureRegion(&dst_loc, 0, 0, 0, &src_loc, None);

    let bar = D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: transmute_copy(&tex),
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: D3D12_RESOURCE_STATE_COPY_DEST,
                StateAfter: D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            }),
        },
    };
    list.ResourceBarrier(&[bar]);
    Ok((tex, stg))
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
