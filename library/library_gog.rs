//! Open-source GOG offline installer importer, ICO->PNG cover extractor, PE architecture/API
//! detector, and prospero-win dynamic PC game handoff for TrinityXSX.
//!
//! Uses only open-source components:
//! - In-process `trinity_innoextract.dll` (built from `dscharrer/innoextract`, zlib license)
//!   loaded via `LoadLibraryW` so extraction works inside PS5 single-process Wine
//!   (`WINELOADERNOEXEC=1`) without fork/exec.
//! - Pure-Rust `goggame-*.info` metadata parsing, `.ico` embedded PNG extraction,
//!   `MicrosoftGame.config` XML generation, and PE32/PE32+ import inspection.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegFlushKey, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_WOW64_32KEY, KEY_WOW64_64KEY, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};

use crate::{library, library_i18n::Lang};

const PS5_STORAGE_ROOT: &str = "Z:\\data\\homebrew\\PPSA83601";
const PC_PROFILE_ID: &str = "trinityxsx-pc-game";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeLaunchKind {
    /// Imports console-only DLLs (`d3d12_xs.dll`, `xgameplatform.dll`, `xcrdapi.dll`, etc.)
    XboxSeries,
    /// Standard Windows PC executable (e.g. GOG DRM-free game) — best launched via prospero-win
    PcNative { is_64bit: bool, prefer_opengl: bool, has_gamepad_api: bool },
}

impl PeLaunchKind {
    pub fn badge(self) -> &'static str {
        match self {
            PeLaunchKind::XboxSeries => "XSX",
            PeLaunchKind::PcNative { is_64bit: true, .. } => "PC64",
            PeLaunchKind::PcNative { is_64bit: false, .. } => "PC32",
        }
    }
}

/// Preferred DualSense controller mapping when launching a PC/GOG game on PS5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PcInputMode {
    /// Auto-select Gamepad (XInput) when the executable imports an XInput-capable API,
    /// or Mouse + WASD emulation when it only expects mouse/keyboard input.
    #[default]
    Auto,
    /// Always expose the DualSense as an Xbox 360 XInput gamepad.
    Gamepad,
    /// Left stick = mouse cursor, D-pad = WASD, X/R2 = LMB, Square/L2 = RMB (RTS, RPG, strategy).
    MouseKeyboard,
    /// Classic Arrow-key platformer/arcade preset: D-pad = Arrows, Right stick = mouse camera,
    /// X/R2 = Space, Square = C, Circle = Backspace, Triangle = Enter, R1 = Ctrl, L1 = Shift, L2 = Z.
    ArrowsAction,
    /// Classic 3D/FPS keyboard+mouse preset: D-pad = WASD, Right stick = mouse look,
    /// R2/X = LMB, L2/Square = RMB, L1 = Space, R1 = E, Circle = Ctrl, Triangle = R.
    FpsWasd,
}

impl PcInputMode {
    pub fn code(self) -> &'static str {
        match self {
            PcInputMode::Auto => "auto",
            PcInputMode::Gamepad => "gamepad",
            PcInputMode::MouseKeyboard => "mouse_kb",
            PcInputMode::ArrowsAction => "arrows",
            PcInputMode::FpsWasd => "fps",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "gamepad" | "xinput" => PcInputMode::Gamepad,
            "mouse_kb" | "mouse" | "keyboard" => PcInputMode::MouseKeyboard,
            "arrows" | "arcade" | "platformer" => PcInputMode::ArrowsAction,
            "fps" | "wasd_fps" => PcInputMode::FpsWasd,
            _ => PcInputMode::Auto,
        }
    }

    pub fn next(self) -> Self {
        match self {
            PcInputMode::Auto => PcInputMode::Gamepad,
            PcInputMode::Gamepad => PcInputMode::MouseKeyboard,
            PcInputMode::MouseKeyboard => PcInputMode::ArrowsAction,
            PcInputMode::ArrowsAction => PcInputMode::FpsWasd,
            PcInputMode::FpsWasd => PcInputMode::Auto,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            PcInputMode::Auto => PcInputMode::FpsWasd,
            PcInputMode::Gamepad => PcInputMode::Auto,
            PcInputMode::MouseKeyboard => PcInputMode::Gamepad,
            PcInputMode::ArrowsAction => PcInputMode::MouseKeyboard,
            PcInputMode::FpsWasd => PcInputMode::ArrowsAction,
        }
    }

    /// Resolve `Auto` into a concrete handoff input mode (`Gamepad` vs `MouseKeyboard`),
    /// preserving any explicit user preset selection.
    pub fn resolve(self, has_gamepad_api: bool) -> Self {
        match self {
            PcInputMode::Auto => {
                if has_gamepad_api {
                    PcInputMode::Gamepad
                } else {
                    PcInputMode::MouseKeyboard
                }
            }
            explicit => explicit,
        }
    }

    /// True when the resolved mode drives keyboard/mouse bindings instead of XInput.
    pub fn resolve_use_mouse_kb(self, has_gamepad_api: bool) -> bool {
        self.resolve(has_gamepad_api) != PcInputMode::Gamepad
    }
}

/// Only APIs backed by prospero-win's DualSense XInput bridge count as gamepad-ready.
/// Legacy DirectInput (`dinput.dll` / `dinput8.dll`) is widely imported solely for PC keyboard
/// and mouse input and is not bridged to XInput on PS5, so DirectInput-only games default to
/// DualSense Mouse & Keyboard mode under `Auto`.
fn is_gamepad_import(dll_lower: &str) -> bool {
    dll_lower.starts_with("xinput")
        || matches!(dll_lower, "sdl2.dll" | "sdl3.dll" | "unityplayer.dll" | "gameinput.dll")
}

#[derive(Debug, Clone)]
pub struct GogInstaller {
    pub display_name: String,
    pub slug: String,
    pub setup_exe: PathBuf,
    pub slice_files: Vec<PathBuf>,
    pub total_bytes: u64,
    pub install_parent: PathBuf,
}

impl GogInstaller {
    /// Returns true if this installer's target game directory already contains a finalized
    /// `MicrosoftGame.config` (either at root or under `app/`).
    pub fn is_installed(&self) -> bool {
        let dest = self.install_parent.join(&self.slug);
        dest.join("MicrosoftGame.config").is_file()
            || dest.join("app").join("MicrosoftGame.config").is_file()
            || (dest.is_dir() && !find_pc_executables(&dest).is_empty())
    }

    /// Delete the installer executable and all `.bin` slices from disk, returning the number
    /// of removed files.
    pub fn remove_installer_files(&self) -> usize {
        let mut removed = 0;
        if fs::remove_file(&self.setup_exe).is_ok() {
            removed += 1;
        }
        for bin in &self.slice_files {
            if fs::remove_file(bin).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

#[derive(Debug, Clone)]
pub struct FinalizedGog {
    pub name: String,
    pub identity: String,
    pub executable_rel: String,
    pub has_cover: bool,
    pub pe_kind: PeLaunchKind,
}

fn read_u16_le(buf: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(off..off + 2)?.try_into().ok()?))
}

fn read_u32_le(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(off..off + 4)?.try_into().ok()?))
}

/// Read a PE file's bitness (`true` = PE32+, `false` = PE32) and lowercased imported DLL names.
fn read_pe_imports(pe_path: &Path) -> Option<(bool, Vec<String>)> {
    let mut f = File::open(pe_path).ok()?;
    let mut header = vec![0u8; 64 * 1024];
    let n = f.read(&mut header).unwrap_or(0);
    header.truncate(n);
    if header.len() < 0x100 || &header[0..2] != b"MZ" {
        return None;
    }
    let pe_off = read_u32_le(&header, 0x3C)? as usize;
    if header.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
        return None;
    }
    let num_sections = read_u16_le(&header, pe_off + 6).unwrap_or(0) as usize;
    let opt_size = read_u16_le(&header, pe_off + 20).unwrap_or(0) as usize;
    let opt_off = pe_off + 24;
    let magic = read_u16_le(&header, opt_off).unwrap_or(0);
    let (is_64bit, data_dir_off) = match magic {
        0x20B => (true, opt_off + 112), // PE32+
        0x10B => (false, opt_off + 96), // PE32
        _ => return None,
    };

    let import_rva = read_u32_le(&header, data_dir_off + 8).unwrap_or(0);
    let sec_off = opt_off + opt_size;
    let mut sections = Vec::with_capacity(num_sections.min(64));
    for i in 0..num_sections.min(64) {
        let s = sec_off + i * 40;
        let Some(va) = read_u32_le(&header, s + 12) else { break };
        let Some(vsize) = read_u32_le(&header, s + 8) else { break };
        let Some(raw_size) = read_u32_le(&header, s + 16) else { break };
        let Some(raw_ptr) = read_u32_le(&header, s + 20) else { break };
        sections.push((va, vsize.max(raw_size), raw_ptr, raw_size));
    }

    let rva_to_off = |rva: u32| -> Option<usize> {
        for &(va, span, raw_ptr, raw_size) in &sections {
            if rva >= va && rva < va.saturating_add(span) {
                let delta = rva - va;
                if delta < raw_size {
                    return Some((raw_ptr + delta) as usize);
                }
            }
        }
        None
    };

    let mut imports = Vec::new();
    if import_rva != 0 {
        if let Some(imp_file_off) = rva_to_off(import_rva) {
            let mut desc_buf = vec![0u8; 256 * 20];
            if f.seek(SeekFrom::Start(imp_file_off as u64)).is_ok() {
                let n = f.read(&mut desc_buf).unwrap_or(0);
                desc_buf.truncate(n);
                let mut name_buf = [0u8; 128];
                let mut idx = 0usize;
                while idx * 20 + 20 <= desc_buf.len() {
                    let desc = idx * 20;
                    let Some(name_rva) = read_u32_le(&desc_buf, desc + 12) else { break };
                    let ft_rva = read_u32_le(&desc_buf, desc + 16).unwrap_or(0);
                    if name_rva == 0 && ft_rva == 0 {
                        break;
                    }
                    if let Some(name_off) = rva_to_off(name_rva) {
                        if f.seek(SeekFrom::Start(name_off as u64)).is_ok() {
                            let rn = f.read(&mut name_buf).unwrap_or(0);
                            let slice = &name_buf[..rn];
                            let end = slice.iter().position(|&b| b == 0).unwrap_or(rn);
                            if end > 0 {
                                if let Ok(s) = std::str::from_utf8(&slice[..end]) {
                                    imports.push(s.to_ascii_lowercase());
                                }
                            }
                        }
                    }
                    idx += 1;
                }
            }
        }
    }
    Some((is_64bit, imports))
}

/// Read exported symbol names from a PE32/PE32+ file using targeted seeks (no full-file read).
fn read_pe_export_names(pe_path: &Path) -> Option<Vec<String>> {
    let mut f = File::open(pe_path).ok()?;
    let mut header = vec![0u8; 64 * 1024];
    let n = f.read(&mut header).unwrap_or(0);
    header.truncate(n);
    if header.len() < 0x100 || &header[0..2] != b"MZ" {
        return None;
    }
    let pe_off = read_u32_le(&header, 0x3C)? as usize;
    if header.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
        return None;
    }
    let num_sections = read_u16_le(&header, pe_off + 6).unwrap_or(0) as usize;
    let opt_size = read_u16_le(&header, pe_off + 20).unwrap_or(0) as usize;
    let opt_off = pe_off + 24;
    let magic = read_u16_le(&header, opt_off).unwrap_or(0);
    let data_dir_off = match magic {
        0x20B => opt_off + 112,
        0x10B => opt_off + 96,
        _ => return None,
    };
    let exp_rva = read_u32_le(&header, data_dir_off).unwrap_or(0);
    if exp_rva == 0 {
        return Some(Vec::new());
    }
    let sec_off = opt_off + opt_size;
    let mut sections = Vec::with_capacity(num_sections.min(64));
    for i in 0..num_sections.min(64) {
        let s = sec_off + i * 40;
        let Some(va) = read_u32_le(&header, s + 12) else { break };
        let Some(vsize) = read_u32_le(&header, s + 8) else { break };
        let Some(raw_size) = read_u32_le(&header, s + 16) else { break };
        let Some(raw_ptr) = read_u32_le(&header, s + 20) else { break };
        sections.push((va, vsize.max(raw_size), raw_ptr, raw_size));
    }
    let rva_to_off = |rva: u32| -> Option<usize> {
        for &(va, span, raw_ptr, raw_size) in &sections {
            if rva >= va && rva < va.saturating_add(span) {
                let delta = rva - va;
                if delta < raw_size {
                    return Some((raw_ptr + delta) as usize);
                }
            }
        }
        None
    };

    let exp_off = rva_to_off(exp_rva)?;
    let mut dir_buf = [0u8; 40];
    f.seek(SeekFrom::Start(exp_off as u64)).ok()?;
    f.read_exact(&mut dir_buf).ok()?;
    let num_names = read_u32_le(&dir_buf, 24).unwrap_or(0).min(256) as usize;
    let names_rva = read_u32_le(&dir_buf, 32).unwrap_or(0);
    if num_names == 0 || names_rva == 0 {
        return Some(Vec::new());
    }
    let names_off = rva_to_off(names_rva)?;
    let mut rvas_buf = vec![0u8; num_names * 4];
    f.seek(SeekFrom::Start(names_off as u64)).ok()?;
    f.read_exact(&mut rvas_buf).ok()?;

    let mut out = Vec::with_capacity(num_names);
    let mut name_buf = [0u8; 128];
    for i in 0..num_names {
        let Some(nrva) = read_u32_le(&rvas_buf, i * 4) else { break };
        if let Some(noff) = rva_to_off(nrva) {
            if f.seek(SeekFrom::Start(noff as u64)).is_ok() {
                let rn = f.read(&mut name_buf).unwrap_or(0);
                let slice = &name_buf[..rn];
                let end = slice.iter().position(|&b| b == 0).unwrap_or(rn);
                if end > 0 {
                    if let Ok(s) = std::str::from_utf8(&slice[..end]) {
                        out.push(s.to_owned());
                    }
                }
            }
        }
    }
    Some(out)
}

/// Build a minimal valid PE32 DLL whose export table forwards all standard DirectDraw symbols
/// (`DirectDrawCreate`, `DirectDrawCreateEx`, `DirectDrawEnumerate*`, etc.) to `ddraw.dll`.
/// Used to neutrally bypass Win10/11-only `DDrawCompat` shims (e.g. GOG Win11 `xdd.dll`) under
/// Wine without modifying the game executable.
pub fn build_ddraw_forwarder_pe32(dll_name: &str) -> Vec<u8> {
    let mut exports: Vec<&str> = vec![
        "AcquireDDThreadLock",
        "CompleteCreateSysmemSurface",
        "D3DParseUnknownCommand",
        "DDGetAttachedSurfaceLcl",
        "DDInternalLock",
        "DDInternalUnlock",
        "DSoundHelp",
        "DirectDrawCreate",
        "DirectDrawCreateClipper",
        "DirectDrawCreateEx",
        "DirectDrawEnumerateA",
        "DirectDrawEnumerateExA",
        "DirectDrawEnumerateExW",
        "DirectDrawEnumerateW",
        "DllCanUnloadNow",
        "DllGetClassObject",
        "GetDDSurfaceLocal",
        "GetOLEThunkData",
        "GetSurfaceFromDC",
        "RegisterSpecialCase",
        "ReleaseDDThreadLock",
        "SetAppCompatData",
    ];
    exports.sort_unstable();
    let count = exports.len();

    // Lay out `.edata` at RVA 0x2000:
    //   0x00..0x28: IMAGE_EXPORT_DIRECTORY (40 bytes)
    //   eat_off:    AddressOfFunctions (count * 4)
    //   ent_off:    AddressOfNames     (count * 4)
    //   eot_off:    AddressOfNameOrdinals (count * 2)
    //   strings:    dll_name\0, export names\0, "ddraw.<name>\0" forwarder targets
    const EDATA_RVA: u32 = 0x2000;
    let eat_off = 40usize;
    let ent_off = eat_off + count * 4;
    let eot_off = ent_off + count * 4;
    let str_start = (eot_off + count * 2 + 3) & !3;

    let mut edata = vec![0u8; str_start];
    let dll_name_rva = EDATA_RVA + edata.len() as u32;
    edata.extend_from_slice(dll_name.as_bytes());
    edata.push(0);

    let mut name_rvas = Vec::with_capacity(count);
    for name in &exports {
        name_rvas.push(EDATA_RVA + edata.len() as u32);
        edata.extend_from_slice(name.as_bytes());
        edata.push(0);
    }

    let mut fwd_rvas = Vec::with_capacity(count);
    for name in &exports {
        fwd_rvas.push(EDATA_RVA + edata.len() as u32);
        edata.extend_from_slice(b"ddraw.");
        edata.extend_from_slice(name.as_bytes());
        edata.push(0);
    }

    let edata_vsize = edata.len() as u32;
    // Populate IMAGE_EXPORT_DIRECTORY
    edata[12..16].copy_from_slice(&dll_name_rva.to_le_bytes()); // Name
    edata[16..20].copy_from_slice(&1u32.to_le_bytes()); // Base = 1
    edata[20..24].copy_from_slice(&(count as u32).to_le_bytes()); // NumberOfFunctions
    edata[24..28].copy_from_slice(&(count as u32).to_le_bytes()); // NumberOfNames
    edata[28..32].copy_from_slice(&(EDATA_RVA + eat_off as u32).to_le_bytes());
    edata[32..36].copy_from_slice(&(EDATA_RVA + ent_off as u32).to_le_bytes());
    edata[36..40].copy_from_slice(&(EDATA_RVA + eot_off as u32).to_le_bytes());
    for i in 0..count {
        edata[eat_off + i * 4..eat_off + i * 4 + 4].copy_from_slice(&fwd_rvas[i].to_le_bytes());
        edata[ent_off + i * 4..ent_off + i * 4 + 4].copy_from_slice(&name_rvas[i].to_le_bytes());
        edata[eot_off + i * 2..eot_off + i * 2 + 2].copy_from_slice(&(i as u16).to_le_bytes());
    }

    let edata_raw_size = (edata.len() + 0x1FF) & !0x1FF;
    let edata_vspan = (edata.len() + 0xFFF) & !0xFFF;
    let size_of_image = 0x2000u32 + edata_vspan as u32;

    let mut pe = vec![0u8; 0x400 + edata_raw_size];
    // DOS header
    pe[0..2].copy_from_slice(b"MZ");
    pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());

    // PE signature + COFF header (0x80..0x98)
    pe[0x80..0x84].copy_from_slice(b"PE\0\0");
    pe[0x84..0x86].copy_from_slice(&0x014Cu16.to_le_bytes()); // i386
    pe[0x86..0x88].copy_from_slice(&2u16.to_le_bytes()); // 2 sections (.text, .edata)
    pe[0x94..0x96].copy_from_slice(&0x00E0u16.to_le_bytes()); // SizeOfOptionalHeader
    pe[0x96..0x98].copy_from_slice(&0x2102u16.to_le_bytes()); // EXECUTABLE | 32BIT | DLL

    // PE32 Optional Header (0x98..0x178)
    pe[0x98..0x9A].copy_from_slice(&0x010Bu16.to_le_bytes()); // PE32 magic
    pe[0x9A] = 14;
    pe[0x9C..0xA0].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfCode
    pe[0xA0..0xA4].copy_from_slice(&(edata_raw_size as u32).to_le_bytes()); // SizeOfInitializedData
    pe[0xA8..0xAC].copy_from_slice(&0x1000u32.to_le_bytes()); // AddressOfEntryPoint
    pe[0xAC..0xB0].copy_from_slice(&0x1000u32.to_le_bytes()); // BaseOfCode
    pe[0xB0..0xB4].copy_from_slice(&EDATA_RVA.to_le_bytes()); // BaseOfData
    pe[0xB4..0xB8].copy_from_slice(&0x1000_0000u32.to_le_bytes()); // ImageBase
    pe[0xB8..0xBC].copy_from_slice(&0x1000u32.to_le_bytes()); // SectionAlignment
    pe[0xBC..0xC0].copy_from_slice(&0x0200u32.to_le_bytes()); // FileAlignment
    pe[0xC0..0xC2].copy_from_slice(&5u16.to_le_bytes()); // MajorOperatingSystemVersion
    pe[0xC2..0xC4].copy_from_slice(&1u16.to_le_bytes()); // MinorOperatingSystemVersion
    pe[0xC4..0xC6].copy_from_slice(&1u16.to_le_bytes()); // MajorImageVersion
    pe[0xC8..0xCA].copy_from_slice(&5u16.to_le_bytes()); // MajorSubsystemVersion
    pe[0xCA..0xCC].copy_from_slice(&1u16.to_le_bytes()); // MinorSubsystemVersion
    pe[0xD0..0xD4].copy_from_slice(&size_of_image.to_le_bytes()); // SizeOfImage
    pe[0xD4..0xD8].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfHeaders
    pe[0xDC..0xDE].copy_from_slice(&2u16.to_le_bytes()); // Subsystem = WINDOWS_GUI
    pe[0xDE..0xE0].copy_from_slice(&0x0100u16.to_le_bytes()); // DllCharacteristics = NX_COMPAT
    pe[0xE0..0xE4].copy_from_slice(&0x0010_0000u32.to_le_bytes()); // SizeOfStackReserve
    pe[0xE4..0xE8].copy_from_slice(&0x0000_1000u32.to_le_bytes()); // SizeOfStackCommit
    pe[0xE8..0xEC].copy_from_slice(&0x0010_0000u32.to_le_bytes()); // SizeOfHeapReserve
    pe[0xEC..0xF0].copy_from_slice(&0x0000_1000u32.to_le_bytes()); // SizeOfHeapCommit
    pe[0xF4..0xF8].copy_from_slice(&16u32.to_le_bytes()); // NumberOfRvaAndSizes
                                                          // DataDirectory[0] = Export Table (at 0x98 + 96 = 0xF8)
    pe[0xF8..0xFC].copy_from_slice(&EDATA_RVA.to_le_bytes());
    pe[0xFC..0x100].copy_from_slice(&edata_vsize.to_le_bytes());

    // Section 0: .text (at 0x178)
    pe[0x178..0x17D].copy_from_slice(b".text");
    pe[0x180..0x184].copy_from_slice(&8u32.to_le_bytes()); // VirtualSize
    pe[0x184..0x188].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress
    pe[0x188..0x18C].copy_from_slice(&0x0200u32.to_le_bytes()); // SizeOfRawData
    pe[0x18C..0x190].copy_from_slice(&0x0200u32.to_le_bytes()); // PointerToRawData
    pe[0x19C..0x1A0].copy_from_slice(&0x6000_0020u32.to_le_bytes()); // CODE | EXECUTE | READ

    // Section 1: .edata (at 0x1A0)
    pe[0x1A0..0x1A6].copy_from_slice(b".edata");
    pe[0x1A8..0x1AC].copy_from_slice(&edata_vsize.to_le_bytes()); // VirtualSize
    pe[0x1AC..0x1B0].copy_from_slice(&EDATA_RVA.to_le_bytes()); // VirtualAddress
    pe[0x1B0..0x1B4].copy_from_slice(&(edata_raw_size as u32).to_le_bytes()); // SizeOfRawData
    pe[0x1B4..0x1B8].copy_from_slice(&0x0400u32.to_le_bytes()); // PointerToRawData
    pe[0x1C4..0x1C8].copy_from_slice(&0x4000_0040u32.to_le_bytes()); // INITIALIZED_DATA | READ

    // .text body: `BOOL WINAPI DllMain(HINSTANCE, DWORD, LPVOID) { return TRUE; }`
    // x86: `mov eax, 1; ret 0x0C`
    pe[0x200..0x208].copy_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00, 0xC2, 0x0C, 0x00]);

    // .edata body
    pe[0x400..0x400 + edata.len()].copy_from_slice(&edata);
    pe
}

/// Detect and neutrally replace a renamed Win10/11-only `DDrawCompat` shim (`xdd.dll`) with a
/// tiny PE32 forwarder to Wine's built-in `ddraw.dll`. Preserves the original file once as
/// `xdd.dll.ddrawcompat.bak` and leaves the game executable completely untouched.
pub fn replace_ddrawcompat_xdd_if_needed(exe_path: &Path) -> bool {
    let Some((is_64bit, exe_imports)) = read_pe_imports(exe_path) else { return false };
    if is_64bit || !exe_imports.iter().any(|d| d == "xdd.dll") {
        return false;
    }
    let Some(dir) = exe_path.parent() else { return false };
    let Ok(rd) = fs::read_dir(dir) else { return false };
    let xdd_path = rd
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("xdd.dll")));
    let Some(xdd_path) = xdd_path else { return false };

    // Skip already-replaced tiny forwarders (< 64 KiB)
    let size = fs::metadata(&xdd_path).map(|m| m.len()).unwrap_or(0);
    if size < 64 * 1024 {
        return false;
    }
    let Some((xdd_is_64, xdd_imports)) = read_pe_imports(&xdd_path) else { return false };
    if xdd_is_64 {
        return false;
    }
    let imports_dwm = xdd_imports.iter().any(|d| d == "dwmapi.dll");
    let imports_d3d9 = xdd_imports.iter().any(|d| d == "d3d9.dll");
    if !imports_dwm || imports_d3d9 {
        return false;
    }
    let Some(xdd_exports) = read_pe_export_names(&xdd_path) else { return false };
    if !xdd_exports.iter().any(|e| e == "DirectDrawCreate") {
        return false;
    }

    let backup = dir.join("xdd.dll.ddrawcompat.bak");
    if !backup.exists() && fs::copy(&xdd_path, &backup).is_err() {
        return false;
    }
    let forwarder = build_ddraw_forwarder_pe32("xdd.dll");
    fs::write(&xdd_path, forwarder).is_ok()
}

/// Inspect sibling `.dll` modules next to the executable for XInput / controller API imports.
/// Handles modular engines (e.g. CryEngine, Unity, Unreal, SDL2) where a small stub `.exe`
/// delegates engine and controller initialization to a sibling DLL in the same folder.
fn sibling_dlls_have_gamepad_api(exe_path: &Path) -> bool {
    let Some(dir) = exe_path.parent() else { return false };
    let Ok(rd) = fs::read_dir(dir) else { return false };
    let mut checked = 0usize;
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_file() || !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) {
            continue;
        }
        let low = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if is_gamepad_import(&low) {
            return true;
        }
        checked += 1;
        if checked > 64 {
            break;
        }
        if let Some((_, dll_imports)) = read_pe_imports(&p) {
            if dll_imports.iter().any(|d| is_gamepad_import(d)) {
                return true;
            }
        }
    }
    false
}

/// True when the executable's directory ships a local `ddraw.dll` that wraps DirectDraw onto
/// Direct3D 9 (e.g. GOG's bundled `dxcfg`/`ddraw.dll` wrapper) rather than OpenGL. Enabling
/// `ddraw=n,b` for such wrappers moves presentation from Wine's CPU GDI path onto DXVK / GPU.
pub fn has_local_d3d9_ddraw_wrapper(exe_path: &Path) -> bool {
    let Some(dir) = exe_path.parent() else { return false };
    let Ok(rd) = fs::read_dir(dir) else { return false };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() && entry.file_name().to_string_lossy().eq_ignore_ascii_case("ddraw.dll") {
            let Some((_, imports)) = read_pe_imports(&p) else { return false };
            let has_d3d9 = imports.iter().any(|d| d == "d3d9.dll");
            let has_gl = imports.iter().any(|d| d == "opengl32.dll");
            return has_d3d9 && !has_gl;
        }
    }
    false
}

/// Inspect a PE executable's architecture (PE32 vs PE32+) and import table to determine
/// whether it is a native Xbox Series X|S binary or a standard Windows PC/GOG game.
pub fn detect_pe_kind(exe_path: &Path) -> PeLaunchKind {
    let Some((is_64bit, imports)) = read_pe_imports(exe_path) else {
        return PeLaunchKind::XboxSeries;
    };

    let is_console = imports.iter().any(|dll| {
        matches!(
            dll.as_str(),
            "d3d12_xs.dll"
                | "d3d12_x.dll"
                | "xgameplatform.dll"
                | "xcrdapi.dll"
                | "dstorage_xs.dll"
                | "kernelx.dll"
                | "xgameruntime.dll"
                | "xcrt.dll"
        )
    });
    if is_console {
        return PeLaunchKind::XboxSeries;
    }

    let has_dx_or_vk = imports
        .iter()
        .any(|d| d.starts_with("d3d") || d == "dxgi.dll" || d == "vulkan-1.dll" || d == "unityplayer.dll");
    let has_gl = imports.iter().any(|d| d == "opengl32.dll");
    let has_unity_sibling = exe_path.parent().is_some_and(|p| p.join("UnityPlayer.dll").is_file());
    let prefer_opengl = has_gl && !has_dx_or_vk && !has_unity_sibling;
    let has_gamepad_api =
        has_unity_sibling || imports.iter().any(|d| is_gamepad_import(d)) || sibling_dlls_have_gamepad_api(exe_path);

    PeLaunchKind::PcNative { is_64bit, prefer_opengl, has_gamepad_api }
}

fn contains_inno_signature(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else { return false };
    let mut buf = vec![0u8; 2 * 1024 * 1024];
    let Ok(n) = f.read(&mut buf) else { return false };
    buf.truncate(n);
    const SIG: &[u8] = b"Inno Setup Setup Data";
    buf.windows(SIG.len()).any(|w| w == SIG)
}

fn prettify_installer_name(stem: &str) -> (String, String) {
    let mut clean = stem.trim().to_owned();
    let lower = clean.to_ascii_lowercase();
    if lower.starts_with("setup_") {
        clean = clean[6..].to_owned();
    }
    // Strip trailing GOG build metadata like _(64bit)_(89650) or _1.0.30000_(64bit)
    if let Some(pos) = clean.find("_(") {
        clean.truncate(pos);
    }
    let display = clean
        .split('_')
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), ch.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let display = if display.is_empty() { stem.to_owned() } else { display };
    let slug: String = display
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() { "gog-game".into() } else { slug };
    (display, slug)
}

fn collect_installers_in_dir(scan_dir: &Path, install_parent: &Path, out: &mut Vec<GogInstaller>) {
    let Ok(rd) = fs::read_dir(scan_dir) else { return };
    let mut exes = Vec::new();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
            let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fname.to_ascii_lowercase().starts_with("setup_") && contains_inno_signature(&p) {
                exes.push(p);
            }
        }
    }
    exes.sort();
    for setup_exe in exes {
        if out.iter().any(|existing| existing.setup_exe == setup_exe) {
            continue;
        }
        let stem = setup_exe.file_stem().and_then(|s| s.to_str()).unwrap_or("gog_game");
        let mut total_bytes = fs::metadata(&setup_exe).map(|m| m.len()).unwrap_or(0);
        let mut slice_files = Vec::new();
        for idx in 1..=99u32 {
            let bin = scan_dir.join(format!("{stem}-{idx}.bin"));
            if let Ok(meta) = fs::metadata(&bin) {
                total_bytes = total_bytes.saturating_add(meta.len());
                slice_files.push(bin);
            } else {
                break;
            }
        }
        let (display_name, slug) = prettify_installer_name(stem);
        out.push(GogInstaller {
            display_name,
            slug,
            setup_exe,
            slice_files,
            total_bytes,
            install_parent: install_parent.to_path_buf(),
        });
    }
}

fn resolve_stored_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed.starts_with('/') {
        PathBuf::from(format!("Z:{}", trimmed.replace('/', "\\")))
    } else {
        PathBuf::from(trimmed)
    }
}

/// Scan configured library folders (`<folder>/installers/`, `<folder>/`) and common PS5 USB/ext
/// installer directories for DRM-free GOG Inno Setup offline installers.
pub fn scan_gog_installers(library_paths: &[String]) -> Vec<GogInstaller> {
    let mut out = Vec::new();
    let default_parent = library_paths
        .iter()
        .map(|s| resolve_stored_path(s))
        .find(|p| p.is_dir())
        .unwrap_or_else(|| PathBuf::from("Z:\\mnt\\ext1\\xboxseries"));

    for raw in library_paths {
        let base = resolve_stored_path(raw);
        if !base.is_dir() {
            continue;
        }
        let sub = base.join("installers");
        if sub.is_dir() {
            collect_installers_in_dir(&sub, &base, &mut out);
        }
        // Also scan 1-level subdirectories inside installers/ (e.g. installers/game-folder/setup_*.exe)
        if let Ok(rd) = fs::read_dir(&sub) {
            for ent in rd.flatten() {
                if ent.path().is_dir() {
                    collect_installers_in_dir(&ent.path(), &base, &mut out);
                }
            }
        }
        collect_installers_in_dir(&base, &base, &mut out);
    }

    for extra in ["Z:\\mnt\\ext1\\installers", "Z:\\mnt\\usb0\\installers", "Z:\\mnt\\usb0"] {
        let p = Path::new(extra);
        if p.is_dir() {
            collect_installers_in_dir(p, &default_parent, &mut out);
        }
    }
    out
}

/// Extract the largest embedded PNG image from a Windows `.ico` file (GOG icons embed 256x256 PNGs).
pub fn extract_png_from_ico(ico_bytes: &[u8]) -> Option<Vec<u8>> {
    if ico_bytes.len() < 6 {
        return None;
    }
    let reserved = read_u16_le(ico_bytes, 0)?;
    let ico_type = read_u16_le(ico_bytes, 2)?;
    let count = read_u16_le(ico_bytes, 4)? as usize;
    if reserved != 0 || ico_type != 1 || count == 0 || ico_bytes.len() < 6 + count * 16 {
        return None;
    }
    const PNG_SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    let mut best: Option<(u32, &[u8])> = None;
    for i in 0..count {
        let e = 6 + i * 16;
        let w = if ico_bytes[e] == 0 { 256u32 } else { ico_bytes[e] as u32 };
        let h = if ico_bytes[e + 1] == 0 { 256u32 } else { ico_bytes[e + 1] as u32 };
        let size = read_u32_le(ico_bytes, e + 8)? as usize;
        let off = read_u32_le(ico_bytes, e + 12)? as usize;
        let Some(slice) = ico_bytes.get(off..off.saturating_add(size)) else { continue };
        if slice.starts_with(PNG_SIG) {
            let area = w * h;
            if best.is_none_or(|(b_area, _)| area > b_area) {
                best = Some((area, slice));
            }
        }
    }
    best.map(|(_, s)| s.to_vec())
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) >= 0x20 => out.push(c),
            _ => {}
        }
    }
    out
}

fn is_exe_file_task(task: &serde_json::Value) -> bool {
    let is_file = task.get("type").and_then(|x| x.as_str()).is_none_or(|t| t.eq_ignore_ascii_case("FileTask"));
    let has_exe =
        task.get("path").and_then(|x| x.as_str()).is_some_and(|p| p.trim().to_ascii_lowercase().ends_with(".exe"));
    is_file && has_exe
}

/// Resolve a GOG relative executable path (`Bin//witcher2.exe`) case-insensitively against disk
/// so `MicrosoftGame.config` records the exact on-disk casing (`bin\witcher2.exe`).
fn resolve_relative_ci(base: &Path, rel: &str) -> Option<String> {
    let parts: Vec<&str> = rel.split(['/', '\\']).filter(|s| !s.is_empty() && *s != ".").collect();
    if parts.is_empty() {
        return None;
    }
    let mut cur = base.to_path_buf();
    let mut resolved = Vec::with_capacity(parts.len());
    for part in parts {
        if part == ".." {
            return None;
        }
        if let Ok(rd) = fs::read_dir(&cur) {
            if let Some(found) = rd.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part)) {
                let actual = found.file_name().to_string_lossy().into_owned();
                cur = found.path();
                resolved.push(actual);
                continue;
            }
        }
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            resolved.push(part.to_owned());
            continue;
        }
        return None;
    }
    cur.is_file().then(|| resolved.join("\\"))
}

fn find_gog_info(dir: &Path) -> Option<(String, String, String)> {
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s.starts_with("goggame-") && s.ends_with(".info") {
            let text = fs::read_to_string(entry.path()).ok()?;
            let v: serde_json::Value = serde_json::from_str(&text).ok()?;
            let game_name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().to_owned();
            let game_id = v.get("gameId").and_then(|x| x.as_str()).unwrap_or("00000001").trim().to_owned();
            let tasks = v.get("playTasks").and_then(|x| x.as_array());
            let mut exe_rel = String::new();
            if let Some(arr) = tasks {
                // Prefer a direct `category == "game"` executable over a `.NET` / external `launcher`
                // task (e.g. The Witcher 2 lists Launcher.exe as primary and Bin//witcher2.exe as game).
                let chosen = arr
                    .iter()
                    .find(|t| {
                        is_exe_file_task(t)
                            && t.get("category")
                                .and_then(|c| c.as_str())
                                .is_some_and(|c| c.eq_ignore_ascii_case("game"))
                            && t.get("isPrimary").and_then(|b| b.as_bool()) == Some(true)
                    })
                    .or_else(|| {
                        arr.iter().find(|t| {
                            is_exe_file_task(t)
                                && t.get("category")
                                    .and_then(|c| c.as_str())
                                    .is_some_and(|c| c.eq_ignore_ascii_case("game"))
                        })
                    })
                    .or_else(|| {
                        arr.iter()
                            .find(|t| is_exe_file_task(t) && t.get("isPrimary").and_then(|b| b.as_bool()) == Some(true))
                    })
                    .or_else(|| arr.iter().find(|t| is_exe_file_task(t)))
                    .or_else(|| arr.first());
                if let Some(task) = chosen {
                    if let Some(p) = task.get("path").and_then(|x| x.as_str()) {
                        exe_rel = p.trim().to_owned();
                    }
                }
            }
            if !exe_rel.is_empty() {
                return Some((game_name, game_id, exe_rel));
            }
        }
    }
    None
}

fn is_utility_exe_name(low: &str) -> bool {
    const SKIP: &[&str] = &[
        "unins",
        "crashhandler",
        "setup",
        "redist",
        "vcredist",
        "dxsetup",
        "dplay",
        "config",
        "options",
        "register",
        "netcheck",
        "spectate",
        "greetz",
        "clokspl",
        "performance",
        "usercontent",
        "updater",
        "launcher",
    ];
    SKIP.iter().any(|k| low.contains(k))
}

fn is_mz_file(path: &Path) -> bool {
    let Ok(mut f) = File::open(path) else { return false };
    let mut magic = [0u8; 2];
    f.read_exact(&mut magic).is_ok() && &magic == b"MZ"
}

/// Find candidate Windows PC game executables (`.exe` with valid `MZ` header) in `dir`
/// and 1-level subdirectories, ordered largest first and excluding common utility/uninstaller EXEs.
pub fn find_pc_executables(dir: &Path) -> Vec<String> {
    let mut primary: Vec<(u64, String)> = Vec::new();
    let mut secondary: Vec<(u64, String)> = Vec::new();

    let mut scan_dir = |scan: &Path, prefix: &str| {
        let Ok(rd) = fs::read_dir(scan) else { return };
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() || !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let low = name.to_ascii_lowercase();
            if low.contains("unins")
                || low.starts_with("setup_")
                || low.contains("dxsetup")
                || low.contains("redist")
                || low.contains("crashhandler")
            {
                continue;
            }
            if !is_mz_file(&p) {
                continue;
            }
            let sz = fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let rel = if prefix.is_empty() { name } else { format!("{prefix}\\{name}") };
            if is_utility_exe_name(&low) {
                secondary.push((sz, rel));
            } else {
                primary.push((sz, rel));
            }
        }
    };

    scan_dir(dir, "");
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let sub = entry.file_name().to_string_lossy().to_string();
            let low = sub.to_ascii_lowercase();
            if low.starts_with('.')
                || low.starts_with("__")
                || low == "tmp"
                || low == "userappdata"
                || low == "commonappdata"
            {
                continue;
            }
            scan_dir(&p, &sub);
        }
    }

    primary.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    secondary.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    primary.into_iter().chain(secondary).map(|(_, rel)| rel).collect()
}

fn find_fallback_exe(dir: &Path) -> Option<String> {
    find_pc_executables(dir).into_iter().next()
}

/// Discover an already-unpacked PC/GOG game folder that has no `MicrosoftGame.config`:
/// checks `goggame-*.info` (at root or `app/`) first, then falls back to `find_pc_executables`.
/// Also extracts an embedded `.ico` PNG cover into `Cover.png` when possible.
pub fn discover_unpacked_pc_game(dir: &Path, folder_name: &str, source_label: &str) -> Option<library::Game> {
    let (name, identity, exe_rel) = if let Some((gname, gid, rel)) = find_gog_info(dir) {
        let clean = resolve_relative_ci(dir, &rel)?;
        let n = if gname.is_empty() { folder_name.to_owned() } else { gname };
        let digits: String = gid.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
        (n, format!("GOG.{}", if digits.is_empty() { "Title" } else { &digits }), clean)
    } else if let Some((gname, gid, rel)) = find_gog_info(&dir.join("app")) {
        let clean = resolve_relative_ci(&dir.join("app"), &rel)?;
        let n = if gname.is_empty() { folder_name.to_owned() } else { gname };
        let digits: String = gid.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
        (n, format!("GOG.{}", if digits.is_empty() { "Title" } else { &digits }), format!("app\\{clean}"))
    } else {
        let rel = find_pc_executables(dir).into_iter().next()?;
        let slug: String =
            folder_name.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '.').take(36).collect();
        let id = format!("PC.{}", if slug.is_empty() { "Game" } else { &slug });
        let pretty = folder_name.replace(['-', '_'], " ");
        let pretty = pretty.split_whitespace().collect::<Vec<_>>().join(" ");
        let n = if pretty.is_empty() { folder_name.to_owned() } else { pretty };
        (n, id, rel)
    };

    let exe_path = dir.join(&exe_rel);
    if !exe_path.is_file() {
        return None;
    }

    let mut hash: u32 = 0x811C_9DC5;
    for b in identity.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let title_id = format!("{hash:08X}");

    let cover_path = dir.join("Cover.png");
    let logo = if cover_path.is_file() { Some(cover_path) } else { extract_any_ico_cover(dir).then_some(cover_path) };

    Some(library::Game {
        name,
        identity,
        version: "1.0.0.0".into(),
        title_id,
        executable: exe_path,
        root_dir: dir.to_path_buf(),
        folder: source_label.to_owned(),
        logo,
    })
}

fn extract_any_ico_cover(dir: &Path) -> bool {
    let mut ico_candidates = Vec::new();
    for scan in [dir.to_path_buf(), dir.join("app")] {
        if let Ok(rd) = fs::read_dir(scan) {
            for ent in rd.flatten() {
                let s = ent.file_name().to_string_lossy().to_ascii_lowercase();
                if s.ends_with(".ico") {
                    let priority = if s.starts_with("goggame-") {
                        0u8
                    } else if s.contains("high") || s.contains("game") {
                        1u8
                    } else {
                        2u8
                    };
                    ico_candidates.push((priority, ent.path()));
                }
            }
        }
    }
    ico_candidates.sort_by_key(|(p, _)| *p);
    for (_, ico) in ico_candidates {
        if let Ok(bytes) = fs::read(&ico) {
            if let Some(png) = extract_png_from_ico(&bytes) {
                if fs::write(dir.join("Cover.png"), png).is_ok() {
                    return true;
                }
            }
        }
    }
    false
}

/// Convert an unpacked GOG game directory into a first-class TrinityXSX library title:
/// reads `goggame-*.info`, extracts `Cover.png` from `goggame-*.ico`, writes
/// `MicrosoftGame.config`, removes Inno temp directories, and detects PE architecture/API.
pub fn finalize_gog_game(unpacked_dir: &Path, fallback_name: &str) -> Result<FinalizedGog, String> {
    // Always place MicrosoftGame.config and Cover.png in `unpacked_dir` so the library scanner
    // finds the title at depth 1 even for classic GOG installers that unpack into `app/`.
    let (parsed_name, game_id, exe_rel_clean) = if let Some((n, id, rel)) = find_gog_info(unpacked_dir) {
        let clean = resolve_relative_ci(unpacked_dir, &rel)
            .unwrap_or_else(|| rel.split(['/', '\\']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\\"));
        (n, id, clean)
    } else if let Some((n, id, rel)) = find_gog_info(&unpacked_dir.join("app")) {
        let clean = resolve_relative_ci(&unpacked_dir.join("app"), &rel)
            .unwrap_or_else(|| rel.split(['/', '\\']).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\\"));
        (n, id, format!("app\\{clean}"))
    } else {
        let exe = find_fallback_exe(unpacked_dir)
            .ok_or_else(|| "No game executable found in extracted GOG folder".to_owned())?;
        (fallback_name.to_owned(), "00000001".to_owned(), exe)
    };
    let name = if parsed_name.is_empty() { fallback_name.to_owned() } else { parsed_name };
    let exe_path = unpacked_dir.join(&exe_rel_clean);
    if !exe_path.is_file() {
        return Err(format!("Primary executable missing: {}", exe_path.display()));
    }

    // Extract Cover.png from goggame-*.ico or any embedded-PNG .ico (in root or app/)
    let has_cover = unpacked_dir.join("Cover.png").is_file() || extract_any_ico_cover(unpacked_dir);

    // Build clean TrinityXSX/GDK-compatible identity + TitleId (hex)
    let digits: String = game_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(24).collect();
    let identity = format!("GOG.{}", if digits.is_empty() { "Title" } else { &digits });
    let mut hash: u32 = 0x811C_9DC5;
    for b in identity.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let title_id = format!("{hash:08X}");

    let cover_attr = if has_cover { " Square150x150Logo=\"Cover.png\"" } else { "" };
    let manifest = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <Game configVersion=\"1\">\n\
           <Identity Name=\"{id}\" Publisher=\"CN=GOG\" Version=\"1.0.0.0\" />\n\
           <TitleId>{tid}</TitleId>\n\
           <ExecutableList>\n\
             <Executable Name=\"{exe}\" TargetDeviceFamily=\"Scarlett\" Id=\"Game\" />\n\
           </ExecutableList>\n\
           <ShellVisuals DefaultDisplayName=\"{name}\" PublisherDisplayName=\"GOG\"{cover} />\n\
         </Game>\n",
        id = xml_escape(&identity),
        tid = title_id,
        exe = xml_escape(&exe_rel_clean),
        name = xml_escape(&name),
        cover = cover_attr,
    );
    fs::write(unpacked_dir.join("MicrosoftGame.config"), manifest)
        .map_err(|e| format!("Cannot write MicrosoftGame.config: {e}"))?;

    // Remove Inno Setup scratch directories (`__redist`, `tmp`) to keep folder clean
    let _ = fs::remove_dir_all(unpacked_dir.join("__redist"));
    let _ = fs::remove_dir_all(unpacked_dir.join("tmp"));

    let pe_kind = detect_pe_kind(&exe_path);
    let is_64bit = matches!(pe_kind, PeLaunchKind::PcNative { is_64bit: true, .. });
    let ui_lang = library::load_ui_settings(&library::ui_settings_file()).language;
    let _ = sync_gog_info_language(unpacked_dir, ui_lang);
    let _ = apply_gog_script_registry(unpacked_dir, is_64bit);
    let _ = replace_ddrawcompat_xdd_if_needed(&exe_path);
    Ok(FinalizedGog { name, identity, executable_rel: exe_rel_clean, has_cover, pe_kind })
}

fn gog_lang_spec(lang: Lang) -> (&'static str, &'static str, &'static [&'static str]) {
    match lang {
        Lang::Pl => ("Polish", "pl-PL", &["polish", "pl-pl", "pl_pl", "_pl."]),
        Lang::Pt => ("Portuguese (Brazilian)", "pt-BR", &["portuguese", "pt-br", "pt_br", "brazilian"]),
        Lang::Es => ("Spanish", "es-ES", &["spanish", "es-es", "es_es"]),
        Lang::En => ("English", "en-US", &["english", "en-us", "en_us"]),
    }
}

fn dir_has_lang_token(dir: &Path, tokens: &[&str]) -> bool {
    let Ok(rd) = fs::read_dir(dir) else { return false };
    for entry in rd.flatten() {
        let low = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if tokens.iter().any(|tok| low.contains(tok)) {
            return true;
        }
    }
    false
}

fn game_has_language_assets(game_dir: &Path, tokens: &[&str]) -> bool {
    if dir_has_lang_token(game_dir, tokens) {
        return true;
    }
    const LOC_DIRS: &[&str] = &["localization", "localisation", "loc", "lang", "languages", "data", "app", "gamesdk"];
    let Ok(rd) = fs::read_dir(game_dir) else { return false };
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let low = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if LOC_DIRS.iter().any(|d| *d == low) && dir_has_lang_token(&p, tokens) {
            return true;
        }
    }
    false
}

fn find_gog_info_file(dir: &Path) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() {
            let low = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if low.starts_with("goggame-") && low.ends_with(".info") {
                return Some(p);
            }
        }
    }
    None
}

/// Multi-language GOG Inno Setup installers pack one `goggame-*.info` per language; extracting
/// without a language filter leaves the last language in the archive (often `zh-Hans` / Chinese).
/// Reconcile `goggame-*.info` with the active TrinityXSX UI language (when matching localization
/// assets exist in the game folder) or fall back to English when an unrelated language won.
pub fn sync_gog_info_language(game_dir: &Path, ui_lang: Lang) -> Option<String> {
    let info_path = find_gog_info_file(game_dir).or_else(|| find_gog_info_file(&game_dir.join("app")))?;
    let text = fs::read_to_string(&info_path).ok()?;
    let mut val: serde_json::Value = serde_json::from_str(&text).ok()?;
    let obj = val.as_object_mut()?;

    let cur_lang = obj.get("language").and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let cur_code = obj
        .get("languages")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();

    let (wanted_name, wanted_code, wanted_tokens) = gog_lang_spec(ui_lang);
    let already_wanted = cur_lang.eq_ignore_ascii_case(wanted_name) || cur_code.eq_ignore_ascii_case(wanted_code);
    if already_wanted {
        return None;
    }

    let (target_name, target_code) = if ui_lang != Lang::En && game_has_language_assets(game_dir, wanted_tokens) {
        (wanted_name, wanted_code)
    } else {
        let cur_is_english =
            cur_lang.eq_ignore_ascii_case("english") || cur_code.to_ascii_lowercase().starts_with("en");
        if cur_is_english {
            return None;
        }
        ("English", "en-US")
    };

    obj.insert("language".into(), serde_json::Value::String(target_name.to_owned()));
    obj.insert("languages".into(), serde_json::json!([target_code]));
    let formatted = serde_json::to_string_pretty(&val).ok()?;
    fs::write(&info_path, formatted).ok()?;
    Some(format!("{target_name} ({target_code})"))
}

/// Walk up to 5 ancestor directories from an executable to find the root folder containing
/// `goggame-*.info` or `goggame-*.script` (handles deep trees like `Binaries/.../Release/Game.exe`).
fn find_gog_root_dir(exe_path: &Path) -> Option<PathBuf> {
    let mut cur = exe_path.parent()?;
    for _ in 0..=5 {
        if find_gog_info_file(cur).is_some() || find_gog_script_in_dir(cur).is_some() {
            return Some(cur.to_path_buf());
        }
        let Some(parent) = cur.parent() else { break };
        cur = parent;
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GogRegRoot {
    LocalMachine,
    CurrentUser,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GogRegValue {
    String(String),
    Dword(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GogRegEntry {
    root: GogRegRoot,
    subkey: String,
    name: String,
    value: GogRegValue,
}

fn parse_gog_dword(raw: &str) -> Option<u32> {
    let trimmed = raw.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        return u32::from_str_radix(hex, 16).ok();
    }
    u32::from_str_radix(trimmed, 16).ok().or_else(|| trimmed.parse::<u32>().ok())
}

/// Parse standard GOG Galaxy `goggame-*.script` JSON `"setRegistry"` install actions into
/// concrete registry entries, expanding `{app}` to `app_dir_win` and skipping Windows UAC
/// compatibility-layer shims (`AppCompatFlags\Layers`).
fn parse_gog_script_registry(script_json: &str, app_dir_win: &str) -> Vec<GogRegEntry> {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(script_json) else {
        return Vec::new();
    };
    let Some(actions) = val.get("actions").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let app_clean = app_dir_win.trim_end_matches(['\\', '/']);
    let mut out = Vec::new();
    for item in actions {
        let Some(install) = item.get("install") else { continue };
        if !install.get("action").and_then(|v| v.as_str()).is_some_and(|a| a.eq_ignore_ascii_case("setRegistry")) {
            continue;
        }
        let Some(args) = install.get("arguments") else { continue };
        if args.get("deleteSubkeys").and_then(|v| v.as_bool()).unwrap_or(false) {
            continue;
        }
        let Some(root_str) = args.get("root").and_then(|v| v.as_str()) else { continue };
        let root = match root_str.trim().to_ascii_uppercase().as_str() {
            "HKEY_LOCAL_MACHINE" | "HKLM" => GogRegRoot::LocalMachine,
            "HKEY_CURRENT_USER" | "HKCU" => GogRegRoot::CurrentUser,
            _ => continue,
        };
        let Some(subkey) = args.get("subkey").and_then(|v| v.as_str()).map(str::trim) else { continue };
        if subkey.is_empty() || subkey.to_ascii_lowercase().contains("appcompatflags\\layers") {
            continue;
        }
        let Some(raw_name) = args.get("valueName").and_then(|v| v.as_str()) else { continue };
        let Some(raw_data) = args.get("valueData").and_then(|v| v.as_str()) else { continue };
        let value_type = args.get("valueType").and_then(|v| v.as_str()).unwrap_or("string");
        let name = raw_name.replace("{app}", app_clean);
        let value = if value_type.eq_ignore_ascii_case("dword") {
            let Some(n) = parse_gog_dword(raw_data) else { continue };
            GogRegValue::Dword(n)
        } else {
            GogRegValue::String(raw_data.replace("{app}", app_clean))
        };
        out.push(GogRegEntry { root, subkey: subkey.to_owned(), name, value });
    }
    out
}

fn to_wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn write_registry_entry(entry: &GogRegEntry, sam: u32) -> bool {
    let root_hkey: HKEY = match entry.root {
        GogRegRoot::LocalMachine => HKEY_LOCAL_MACHINE,
        GogRegRoot::CurrentUser => HKEY_CURRENT_USER,
    };
    let subkey_w = to_wide_z(&entry.subkey);
    let name_w = to_wide_z(&entry.name);
    let mut hkey: HKEY = std::ptr::null_mut();
    unsafe {
        let rc = RegCreateKeyExW(
            root_hkey,
            subkey_w.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            sam,
            std::ptr::null(),
            &mut hkey,
            std::ptr::null_mut(),
        );
        if rc != 0 || hkey.is_null() {
            return false;
        }
        let set_rc = match &entry.value {
            GogRegValue::String(s) => {
                let data_w = to_wide_z(s);
                let byte_len = (data_w.len() * std::mem::size_of::<u16>()) as u32;
                RegSetValueExW(hkey, name_w.as_ptr(), 0, REG_SZ, data_w.as_ptr() as *const u8, byte_len)
            }
            GogRegValue::Dword(n) => {
                let bytes = n.to_le_bytes();
                RegSetValueExW(hkey, name_w.as_ptr(), 0, REG_DWORD, bytes.as_ptr(), bytes.len() as u32)
            }
        };
        let _ = RegCloseKey(hkey);
        set_rc == 0
    }
}

fn find_gog_script_in_dir(dir: &Path) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_file() {
            let low = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if low.starts_with("goggame-") && low.ends_with(".script") {
                return Some(p);
            }
        }
    }
    None
}

/// Apply standard GOG `goggame-*.script` `setRegistry` install actions into the current Wine
/// prefix registry so unpacked GOG games find their expected `InstallDir` / configuration keys.
/// Writes both 32-bit (WoW64) and 64-bit HKLM views for 32-bit games.
pub fn apply_gog_script_registry(game_dir: &Path, is_64bit: bool) -> usize {
    let Some(script_path) = find_gog_script_in_dir(game_dir).or_else(|| find_gog_script_in_dir(&game_dir.join("app")))
    else {
        return 0;
    };
    let Ok(text) = fs::read_to_string(&script_path) else {
        return 0;
    };
    let app_dir_win = strip_verbatim_prefix(&game_dir.to_string_lossy());
    let entries = parse_gog_script_registry(&text, &app_dir_win);
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(actions) = val.get("actions").and_then(|v| v.as_array()) {
            for item in actions {
                let Some(install) = item.get("install") else { continue };
                if !install
                    .get("action")
                    .and_then(|v| v.as_str())
                    .is_some_and(|a| a.eq_ignore_ascii_case("supportData"))
                {
                    continue;
                }
                let Some(args) = install.get("arguments") else { continue };
                if !args.get("type").and_then(|v| v.as_str()).is_some_and(|t| t.eq_ignore_ascii_case("folder")) {
                    continue;
                }
                let Some(target) = args.get("target").and_then(|v| v.as_str()) else { continue };
                if let Some(rel) = target.strip_prefix("{app}/").or_else(|| target.strip_prefix("{app}\\")) {
                    if !rel.contains("..") && !rel.contains(':') && !rel.is_empty() {
                        let _ = fs::create_dir_all(game_dir.join(rel.replace('/', "\\")));
                    }
                }
            }
        }
    }
    let mut written = 0usize;
    let mut touched_hklm = false;
    let mut touched_hkcu = false;
    for entry in &entries {
        let ok = match entry.root {
            GogRegRoot::LocalMachine => {
                let wow32 = !is_64bit && write_registry_entry(entry, KEY_WRITE | KEY_WOW64_32KEY);
                let native64 = write_registry_entry(entry, KEY_WRITE | KEY_WOW64_64KEY);
                if wow32 || native64 {
                    touched_hklm = true;
                }
                wow32 || native64
            }
            GogRegRoot::CurrentUser => {
                let res = write_registry_entry(entry, KEY_WRITE);
                if res {
                    touched_hkcu = true;
                }
                res
            }
        };
        if ok {
            written += 1;
        }
    }
    // Synchronously flush modified Wine registry hives to disk before the library exits into
    // `sceSystemServiceLoadExec`, which replaces the process before wineserver's lazy flush runs.
    unsafe {
        if touched_hklm {
            let _ = RegFlushKey(HKEY_LOCAL_MACHINE);
        }
        if touched_hkcu {
            let _ = RegFlushKey(HKEY_CURRENT_USER);
        }
    }
    written
}

/// Extract a GOG offline installer in-process using bundled `trinity_innoextract.dll` (zlib).
/// Works under PS5 `WINELOADERNOEXEC=1` with live atomic byte progress updates.
pub fn extract_gog_installer_in_process(
    setup_exe: &Path,
    output_dir: &Path,
    done_bytes: Arc<AtomicU64>,
    total_bytes: Arc<AtomicU64>,
) -> Result<(), String> {
    type ProgressFn = unsafe extern "C" fn(u64, u64, *mut c_void);
    type ExtractFn = unsafe extern "C" fn(
        *const c_char,
        *const c_char,
        Option<ProgressFn>,
        *mut c_void,
        *mut c_char,
        usize,
    ) -> c_int;

    struct ProgressCtx {
        done: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
    }

    unsafe extern "C" fn on_progress(done: u64, total: u64, ctx: *mut c_void) {
        if !ctx.is_null() {
            let p = &*(ctx as *const ProgressCtx);
            p.done.store(done, Ordering::Relaxed);
            if total > 0 {
                p.total.store(total, Ordering::Relaxed);
            }
        }
    }

    let mut dll_candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dll_candidates.push(dir.join("trinity_innoextract.dll"));
        }
    }
    dll_candidates.push(PathBuf::from("C:\\XCL\\trinity_innoextract.dll"));
    let dll_path = dll_candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| "trinity_innoextract.dll not found next to xcl-library.exe (update to v0.2.7+)".to_owned())?;

    let dll_w: Vec<u16> = dll_path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    let setup_clean = strip_verbatim_prefix(&setup_exe.to_string_lossy());
    let out_clean = strip_verbatim_prefix(&output_dir.to_string_lossy());
    let setup_c = CString::new(setup_clean).map_err(|_| "Invalid installer path".to_owned())?;
    let out_c = CString::new(out_clean).map_err(|_| "Invalid output path".to_owned())?;

    unsafe {
        let module = LoadLibraryW(dll_w.as_ptr());
        if module.is_null() {
            return Err(format!("Cannot load {}: {}", dll_path.display(), io::Error::last_os_error()));
        }
        let sym = GetProcAddress(module, c"trinity_inno_extract_utf8".as_ptr().cast());
        let Some(sym) = sym else {
            FreeLibrary(module);
            return Err("trinity_innoextract.dll missing export trinity_inno_extract_utf8".into());
        };
        let extract: ExtractFn = std::mem::transmute(sym);
        let ctx = ProgressCtx { done: done_bytes, total: total_bytes };
        let mut err_buf = [0u8; 512];
        let rc = extract(
            setup_c.as_ptr(),
            out_c.as_ptr(),
            Some(on_progress),
            &ctx as *const ProgressCtx as *mut c_void,
            err_buf.as_mut_ptr() as *mut c_char,
            err_buf.len(),
        );
        FreeLibrary(module);
        if rc != 0 {
            let msg = CStr::from_ptr(err_buf.as_ptr() as *const c_char).to_string_lossy().into_owned();
            return Err(if msg.is_empty() { format!("innoextract failed (rc={rc})") } else { msg });
        }
    }
    Ok(())
}

fn strip_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("\\\\?\\") {
        rest.to_owned()
    } else {
        path.to_owned()
    }
}

/// Returns true when running inside PS5 prospero-win storage with dynamic profile support.
pub fn ps5_pc_handoff_available() -> bool {
    Path::new(PS5_STORAGE_ROOT).join("profiles").is_dir()
}

/// True when the selected executable is a setup/configuration/options utility rather than a
/// fullscreen game binary. Such Win32 dialog tools need `view = desktop` (so dropdown lists and
/// child popups extending outside the small dialog box are not clipped) and mouse + arrow-key
/// controls even if they import `dinput8.dll` to probe controllers.
pub fn is_config_utility_exe(exe_path: &Path) -> bool {
    let Some(low) = exe_path.file_name().and_then(|s| s.to_str()).map(str::to_ascii_lowercase) else {
        return false;
    };
    const CONFIG_HINTS: &[&str] = &["setup", "config", "options", "settings", "launcher"];
    CONFIG_HINTS.iter().any(|k| low.contains(k))
}

/// Write a dynamic `prospero-win` `.profile` (`trinityxsx-pc-game.profile`) + `pc_launch.flag`
/// so `pw_runner.bin` launches a standard 32-bit or 64-bit PC/GOG game directly under Wine +
/// DXVK / vkd3d-proton / Zink with zero XCL UMA overhead and Touchpad Return-to-Library support.
pub fn write_pc_handoff(
    game_name: &str,
    exe_path: &Path,
    is_64bit: bool,
    prefer_opengl: bool,
    resolved_mode: PcInputMode,
) -> Result<(), String> {
    let root = Path::new(PS5_STORAGE_ROOT);
    let profiles_dir = root.join("profiles");
    if !profiles_dir.is_dir() {
        return Err("PS5 profiles directory not available".into());
    }
    let clean_exe = strip_verbatim_prefix(&exe_path.to_string_lossy());
    let work_dir =
        exe_path.parent().map(|p| strip_verbatim_prefix(&p.to_string_lossy())).unwrap_or_else(|| "C:\\XCL".into());

    // Sanitize display name for prospero-win profile parser (<= 90 ASCII/UTF-8 bytes, single line)
    let safe_name: String =
        game_name.chars().filter(|c| !c.is_control() && *c != '\n' && *c != '\r').take(80).collect();
    let safe_name = if safe_name.trim().is_empty() { "PC Game" } else { safe_name.trim() };

    let lib_profile = library::runtime_profile_file();
    let fg = library::read_framegen_setting(&lib_profile);
    let show_fps = library::read_show_fps_setting(&lib_profile);
    let fg_val = match fg {
        2 => "2",
        3 => "3",
        4 => "4",
        _ => "off",
    };
    let fps_val = if show_fps { "true" } else { "false" };
    let arch = if is_64bit { "pe64" } else { "pe32" };
    let gfx = if prefer_opengl { "zink" } else { "dxvk" };
    let use_local_ddraw = !prefer_opengl && has_local_d3d9_ddraw_wrapper(exe_path);
    if !is_64bit && replace_ddrawcompat_xdd_if_needed(exe_path) {
        println!(
            "TrinityXSX: replaced DDrawCompat xdd.dll with DirectDraw forwarder (backup: xdd.dll.ddrawcompat.bak)"
        );
    }

    // Apply standard GOG metadata fixes (language sync for multi-language installers +
    // `goggame-*.script` registry/folder actions), walking up to 5 levels for deep binary trees.
    if let Some(gog_root) = find_gog_root_dir(exe_path) {
        let ui_lang = library::load_ui_settings(&library::ui_settings_file()).language;
        if let Some(lang_label) = sync_gog_info_language(&gog_root, ui_lang) {
            println!("TrinityXSX: synced GOG game language to {lang_label}");
        }
        let applied = apply_gog_script_registry(&gog_root, is_64bit);
        if applied > 0 {
            println!("TrinityXSX: applied {applied} GOG script registry key(s)");
        }
    }

    // Ensure 64-bit vkd3d-proton D3D12 DLLs from C:\XCL are installed in system32 for PC games
    // launched outside C:\XCL (replace Wine's built-in D3D12 stubs whenever sizes differ).
    // Keep prospero-win's PS5-tuned DXVK v2.6.2-2-g1Queue (d3d11.dll / dxgi.dll) in system32
    // untouched, as upstream DXVK 3.1.1 dereferences NULL vkGetMemoryWin32HandleKHR on PS5 RADV.
    if is_64bit {
        let sys32 = root.join("prefix").join("drive_c").join("windows").join("system32");
        let xcl_dir = root.join("prefix").join("drive_c").join("XCL");
        if sys32.is_dir() && xcl_dir.is_dir() {
            for dll in ["d3d12.dll", "d3d12core.dll"] {
                let src = xcl_dir.join(dll);
                let dst = sys32.join(dll);
                if let Ok(src_meta) = fs::metadata(&src) {
                    let dst_matches = fs::metadata(&dst).is_ok_and(|m| m.len() == src_meta.len());
                    if src_meta.is_file() && !dst_matches {
                        let _ = fs::copy(&src, &dst);
                    }
                }
            }
        }
    }

    let is_config_tool = is_config_utility_exe(exe_path);
    let profile_text = format_pc_handoff_profile(
        safe_name,
        &clean_exe,
        &work_dir,
        arch,
        gfx,
        fg_val,
        fps_val,
        resolved_mode,
        is_config_tool,
        use_local_ddraw,
    );

    let profile_file = format!("{PC_PROFILE_ID}.profile");
    fs::write(profiles_dir.join(&profile_file), profile_text)
        .map_err(|e| format!("Cannot write PC game profile: {e}"))?;
    ensure_profile_listed(&profiles_dir, &profile_file).map_err(|e| format!("Cannot update profiles.lst: {e}"))?;
    fs::write(root.join("pc_launch.flag"), format!("{PC_PROFILE_ID}\n"))
        .map_err(|e| format!("Cannot write pc_launch.flag: {e}"))?;
    Ok(())
}

fn pc_input_section(mode: PcInputMode, is_config_tool: bool) -> &'static str {
    if is_config_tool {
        return "[input]\n\
                mode = keyboard\n\
                mouse = left_stick\n\
                mouse_speed = 900\n\
                cross = mouse_left\n\
                r2 = mouse_left\n\
                square = mouse_right\n\
                l2 = mouse_right\n\
                circle = escape\n\
                options = escape\n\
                triangle = enter\n\
                up = up\n\
                down = down\n\
                left = left\n\
                right = right\n\
                l1 = space\n\
                r1 = tab\n";
    }
    match mode {
        PcInputMode::Gamepad => {
            "[input]\n\
                                 mode = xinput\n"
        }
        PcInputMode::ArrowsAction => {
            "[input]\n\
                                      mode = keyboard\n\
                                      mouse = right_stick\n\
                                      mouse_speed = 900\n\
                                      up = up\n\
                                      down = down\n\
                                      left = left\n\
                                      right = right\n\
                                      cross = space\n\
                                      r2 = space\n\
                                      square = c\n\
                                      triangle = enter\n\
                                      circle = backspace\n\
                                      options = escape\n\
                                      r1 = ctrl\n\
                                      l1 = shift\n\
                                      l2 = z\n"
        }
        PcInputMode::FpsWasd => {
            "[input]\n\
                                 mode = keyboard\n\
                                 mouse = right_stick\n\
                                 mouse_speed = 900\n\
                                 up = w\n\
                                 down = s\n\
                                 left = a\n\
                                 right = d\n\
                                 r2 = mouse_left\n\
                                 cross = mouse_left\n\
                                 l2 = mouse_right\n\
                                 square = mouse_right\n\
                                 l1 = space\n\
                                 r1 = e\n\
                                 circle = ctrl\n\
                                 triangle = r\n\
                                 options = escape\n"
        }
        PcInputMode::MouseKeyboard | PcInputMode::Auto => {
            "[input]\n\
                                                           mode = keyboard\n\
                                                           mouse = left_stick\n\
                                                           mouse_speed = 900\n\
                                                           cross = mouse_left\n\
                                                           r2 = mouse_left\n\
                                                           square = mouse_right\n\
                                                           l2 = mouse_right\n\
                                                           circle = escape\n\
                                                           options = escape\n\
                                                           triangle = enter\n\
                                                           up = w\n\
                                                           down = s\n\
                                                           left = a\n\
                                                           right = d\n\
                                                           l1 = space\n\
                                                           r1 = tab\n"
        }
    }
}

/// Format the dynamic `prospero-win` `.profile` for PC/GOG titles:
/// - `windows.gaming.input=d` disables Wine's unimplemented WGI semi-stub so Unity 6 and
///   modern Input System games immediately fall back to DualSense XInput.
/// - `xinput1_1,xinput1_2,xinput1_3,xinput1_4,xinput9_1_0=b` ensures Wine's DualSense-backed
///   builtin XInput is always used even when a game bundles a native Windows `xinput*.dll`.
/// - `thread_scheduling = true` enables `SCHED_RR` so Wine/DXVK worker threads do not starve.
/// - `menu = true` enables `DXVK_HUD=fps,menu` so the Touchpad Quick Menu renders live over DXVK.
/// - Config/setup utilities use `view = desktop` so Win32 dropdown popups extending below small
///   dialog windows remain visible.
#[allow(clippy::too_many_arguments)]
fn format_pc_handoff_profile(
    safe_name: &str,
    clean_exe: &str,
    work_dir: &str,
    arch: &str,
    gfx: &str,
    fg_val: &str,
    fps_val: &str,
    resolved_mode: PcInputMode,
    is_config_tool: bool,
    use_local_ddraw: bool,
) -> String {
    let input_block = pc_input_section(resolved_mode, is_config_tool);
    let view_mode = if is_config_tool { "desktop" } else { "window" };
    let dx_list = if use_local_ddraw {
        "d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9,d3d8,ddraw"
    } else {
        "d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9,d3d8"
    };
    format!(
        "; Generated dynamically by TrinityXSX for PC/GOG game handoff\n\
         [application]\n\
         id = {PC_PROFILE_ID}\n\
         name = {safe_name}\n\
         executable = {clean_exe}\n\
         working_directory = {work_dir}\n\
         dll_overrides = {dx_list}=n,b;xinput1_1,xinput1_2,xinput1_3,xinput1_4,xinput9_1_0=b;windows.gaming.input=d\n\
         prefix = default\n\
         runtime = wine-wow64\n\
         architecture = {arch}\n\
         graphics = {gfx}\n\
         \n\
         [runtime]\n\
         thread_scheduling = true\n\
         \n\
         [display]\n\
         desktop = 1920x1080\n\
         view = {view_mode}\n\
         framegen = {fg_val}\n\
         show_fps = {fps_val}\n\
         menu = true\n\
         \n\
         {input_block}\
         \n\
         [debug]\n\
         winedebug = err+all\n\
         env = VKD3D_SHADER_CACHE_PATH=C:/XCL/cache/pc-game\n\
         env = DXVK_LOG_PATH=C:/XCL\n"
    )
}

/// Ensure `profile_file` is present in `profiles.lst` so `pw_wine_library_load` (indexed mode)
/// discovers dynamic PC/GOG handoff profiles.
fn ensure_profile_listed(profiles_dir: &Path, profile_file: &str) -> io::Result<()> {
    let lst = profiles_dir.join("profiles.lst");
    let mut lines: Vec<String> = fs::read_to_string(&lst)
        .unwrap_or_else(|_| "trinityxsx-library.profile\n".to_owned())
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    if !lines.iter().any(|l| l.eq_ignore_ascii_case(profile_file)) {
        lines.push(profile_file.to_owned());
        let mut out = lines.join("\n");
        out.push('\n');
        fs::write(&lst, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prettify_gog_installer_name() {
        let (name, slug) = prettify_installer_name("setup_hollow_knight_silksong_1.0.30000_(64bit)_(89650)");
        assert!(name.starts_with("Hollow Knight Silksong"));
        assert!(slug.starts_with("hollow-knight-silksong"));
    }

    #[test]
    fn extract_png_from_synthetic_ico() {
        let fake_png = b"\x89PNG\r\n\x1a\nFAKEPNGDATA";
        let mut ico = Vec::new();
        ico.extend_from_slice(&0u16.to_le_bytes()); // reserved
        ico.extend_from_slice(&1u16.to_le_bytes()); // icon
        ico.extend_from_slice(&1u16.to_le_bytes()); // count
        ico.push(0); // 256w
        ico.push(0); // 256h
        ico.push(0);
        ico.push(0);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(fake_png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(fake_png);
        assert_eq!(extract_png_from_ico(&ico).as_deref(), Some(fake_png.as_slice()));
    }

    #[test]
    fn finalize_gog_creates_manifest_and_cover() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-gog-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("app")).unwrap();
        fs::write(
            tmp.join("goggame-12345.info"),
            r#"{"gameId":"12345","name":"Test & Demo","playTasks":[{"isPrimary":true,"path":"Game.exe"}]}"#,
        )
        .unwrap();
        // Minimal dummy MZ file
        fs::write(tmp.join("Game.exe"), b"MZ\x90\x00").unwrap();
        let fake_png = b"\x89PNG\r\n\x1a\nTEST";
        let mut ico = vec![0, 0, 1, 0, 1, 0, 0, 0, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(fake_png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend_from_slice(fake_png);
        fs::write(tmp.join("app").join("goggame-12345.ico"), ico).unwrap();

        let res = finalize_gog_game(&tmp, "Fallback").unwrap();
        assert_eq!(res.name, "Test & Demo");
        assert_eq!(res.identity, "GOG.12345");
        assert!(res.has_cover);
        assert!(tmp.join("Cover.png").is_file());
        let xml = fs::read_to_string(tmp.join("MicrosoftGame.config")).unwrap();
        assert!(xml.contains("Test &amp; Demo"));
        assert!(xml.contains("Game.exe"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_profile_listed_appends_once() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-lst-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("profiles.lst"), "trinityxsx-library.profile\n").unwrap();
        ensure_profile_listed(&tmp, "trinityxsx-pc-game.profile").unwrap();
        ensure_profile_listed(&tmp, "trinityxsx-pc-game.profile").unwrap();
        let text = fs::read_to_string(tmp.join("profiles.lst")).unwrap();
        assert_eq!(text, "trinityxsx-library.profile\ntrinityxsx-pc-game.profile\n");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pc_handoff_profile_enables_xinput_fallback_sched_and_menu() {
        let profile = format_pc_handoff_profile(
            "Hollow Knight: Silksong",
            "Z:\\mnt\\ext1\\xboxseries\\silksong\\Hollow Knight Silksong.exe",
            "Z:\\mnt\\ext1\\xboxseries\\silksong",
            "pe64",
            "dxvk",
            "off",
            "true",
            PcInputMode::Gamepad,
            false,
            false,
        );
        assert!(profile.contains(
            "dll_overrides = d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9,d3d8=n,b;xinput1_1,xinput1_2,xinput1_3,xinput1_4,xinput9_1_0=b;windows.gaming.input=d"
        ));
        assert!(profile.contains("view = window\n"));
        assert!(profile.contains("[runtime]\nthread_scheduling = true"));
        assert!(profile.contains("show_fps = true\nmenu = true"));
        assert!(profile.contains("[input]\nmode = xinput\n"));
    }

    #[test]
    fn pc_handoff_profile_emits_mouse_keyboard_bindings_when_requested() {
        let profile = format_pc_handoff_profile(
            "Classic Strategy",
            "Z:\\mnt\\ext1\\xboxseries\\rts\\game.exe",
            "Z:\\mnt\\ext1\\xboxseries\\rts",
            "pe32",
            "dxvk",
            "off",
            "true",
            PcInputMode::MouseKeyboard,
            false,
            true,
        );
        assert!(profile.contains("dll_overrides = d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9,d3d8,ddraw=n,b;"));
        assert!(profile.contains("[input]\nmode = keyboard\nmouse = left_stick\nmouse_speed = 900\n"));
        assert!(profile.contains("cross = mouse_left\n"));
        assert!(profile.contains("square = mouse_right\n"));
        assert!(profile.contains("circle = escape\n"));
        assert!(profile.contains("up = w\n"));
    }

    #[test]
    fn pc_handoff_profile_emits_arrows_action_and_fps_presets() {
        let arrows = format_pc_handoff_profile(
            "Classic Platformer",
            "Z:\\games\\plat\\game.exe",
            "Z:\\games\\plat",
            "pe32",
            "dxvk",
            "off",
            "true",
            PcInputMode::ArrowsAction,
            false,
            false,
        );
        assert!(arrows.contains("view = window\n"));
        assert!(arrows.contains("mouse = right_stick\n"));
        assert!(arrows.contains("up = up\ndown = down\nleft = left\nright = right\n"));
        assert!(arrows.contains("cross = space\nr2 = space\nsquare = c\n"));
        assert!(arrows.contains("r1 = ctrl\nl1 = shift\nl2 = z\n"));

        let fps = format_pc_handoff_profile(
            "Classic FPS",
            "Z:\\games\\fps\\game.exe",
            "Z:\\games\\fps",
            "pe32",
            "dxvk",
            "off",
            "true",
            PcInputMode::FpsWasd,
            false,
            false,
        );
        assert!(fps.contains("mouse = right_stick\n"));
        assert!(fps.contains("up = w\ndown = s\nleft = a\nright = d\n"));
        assert!(fps.contains("r2 = mouse_left\ncross = mouse_left\nl2 = mouse_right\n"));
    }

    #[test]
    fn pc_handoff_profile_uses_desktop_view_and_arrow_keys_for_config_utilities() {
        assert!(is_config_utility_exe(Path::new("Z:\\games\\foo\\R3_Setup_DX8.exe")));
        assert!(is_config_utility_exe(Path::new("Z:\\games\\foo\\options.exe")));
        assert!(is_config_utility_exe(Path::new("Z:\\games\\foo\\cnc-ddraw config.exe")));
        assert!(!is_config_utility_exe(Path::new("Z:\\games\\foo\\Rayman3.exe")));
        let profile = format_pc_handoff_profile(
            "Setup Utility",
            "Z:\\games\\foo\\R3_Setup_DX8.exe",
            "Z:\\games\\foo",
            "pe32",
            "dxvk",
            "off",
            "true",
            PcInputMode::Gamepad,
            true,
            false,
        );
        assert!(profile.contains("view = desktop\n"));
        assert!(profile.contains("[input]\nmode = keyboard\nmouse = left_stick\nmouse_speed = 900\n"));
        assert!(profile.contains("up = up\ndown = down\nleft = left\nright = right\n"));
    }

    #[test]
    fn sync_gog_info_language_fixes_overwritten_multilang_archive_and_respects_assets() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-lang-sync-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("Localization")).unwrap();
        fs::write(tmp.join("Localization").join("polish.pak"), b"PK").unwrap();
        fs::write(tmp.join("Localization").join("english.pak"), b"PK").unwrap();
        let info = tmp.join("goggame-1158493447.info");
        fs::write(&info, r#"{"gameId":"1158493447","language":"Chinese (Simplified)","languages":["zh-Hans"]}"#)
            .unwrap();

        // Polish UI + polish.pak -> switches to Polish
        let updated = sync_gog_info_language(&tmp, Lang::Pl);
        assert_eq!(updated.as_deref(), Some("Polish (pl-PL)"));
        let txt = fs::read_to_string(&info).unwrap();
        assert!(txt.contains("\"Polish\"") && txt.contains("\"pl-PL\""));
        // Calling again is a no-op
        assert_eq!(sync_gog_info_language(&tmp, Lang::Pl), None);

        // Spanish UI without Spanish assets -> falls back to English
        let fallback = sync_gog_info_language(&tmp, Lang::Es);
        assert_eq!(fallback.as_deref(), Some("English (en-US)"));
        let txt2 = fs::read_to_string(&info).unwrap();
        assert!(txt2.contains("\"English\"") && txt2.contains("\"en-US\""));
        // Already English with no Spanish assets -> no-op
        assert_eq!(sync_gog_info_language(&tmp, Lang::Es), None);

        // Deep binary path finds GOG root up to 5 levels
        let deep_exe = tmp.join("Binaries").join("Danielle").join("x64-GOG").join("Release").join("Game.exe");
        fs::create_dir_all(deep_exe.parent().unwrap()).unwrap();
        fs::write(&deep_exe, b"MZ").unwrap();
        assert_eq!(find_gog_root_dir(&deep_exe).as_deref(), Some(tmp.as_path()));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pc_input_mode_resolves_auto_and_explicit_overrides() {
        assert_eq!(PcInputMode::Auto.resolve(true), PcInputMode::Gamepad);
        assert_eq!(PcInputMode::Auto.resolve(false), PcInputMode::MouseKeyboard);
        assert_eq!(PcInputMode::ArrowsAction.resolve(true), PcInputMode::ArrowsAction);
        assert_eq!(PcInputMode::FpsWasd.resolve(false), PcInputMode::FpsWasd);
        assert!(!PcInputMode::Auto.resolve_use_mouse_kb(true));
        assert!(PcInputMode::Auto.resolve_use_mouse_kb(false));
        assert!(!PcInputMode::Gamepad.resolve_use_mouse_kb(false));
        assert!(PcInputMode::MouseKeyboard.resolve_use_mouse_kb(true));
        assert!(PcInputMode::ArrowsAction.resolve_use_mouse_kb(true));
        assert!(PcInputMode::FpsWasd.resolve_use_mouse_kb(true));
        assert_eq!(PcInputMode::parse("mouse_kb"), PcInputMode::MouseKeyboard);
        assert_eq!(PcInputMode::parse("gamepad"), PcInputMode::Gamepad);
        assert_eq!(PcInputMode::parse("arrows"), PcInputMode::ArrowsAction);
        assert_eq!(PcInputMode::parse("fps"), PcInputMode::FpsWasd);
        assert_eq!(PcInputMode::parse("unknown"), PcInputMode::Auto);
        assert_eq!(PcInputMode::Auto.next(), PcInputMode::Gamepad);
        assert_eq!(PcInputMode::Gamepad.next(), PcInputMode::MouseKeyboard);
        assert_eq!(PcInputMode::MouseKeyboard.next(), PcInputMode::ArrowsAction);
        assert_eq!(PcInputMode::ArrowsAction.next(), PcInputMode::FpsWasd);
        assert_eq!(PcInputMode::FpsWasd.next(), PcInputMode::Auto);
        assert_eq!(PcInputMode::Auto.prev(), PcInputMode::FpsWasd);
    }

    fn build_minimal_pe64_with_imports(imports: &[&str]) -> Vec<u8> {
        let mut buf = vec![0u8; 0x600];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x86..0x88].copy_from_slice(&1u16.to_le_bytes()); // NumberOfSections = 1
        buf[0x94..0x96].copy_from_slice(&0xF0u16.to_le_bytes()); // SizeOfOptionalHeader
        buf[0x98..0x9A].copy_from_slice(&0x20Bu16.to_le_bytes()); // PE32+ magic
                                                                  // DataDirectory[1] (Import Table) at opt_off (0x98) + 112 + 8 = 0x110
        let imp_rva: u32 = if imports.is_empty() { 0 } else { 0x1000 };
        buf[0x110..0x114].copy_from_slice(&imp_rva.to_le_bytes());
        // Section header #0 at 0x98 + 0xF0 = 0x188: VA=0x1000, VSize=0x400, RawSize=0x400, RawPtr=0x200
        buf[0x190..0x194].copy_from_slice(&0x400u32.to_le_bytes()); // VirtualSize
        buf[0x194..0x198].copy_from_slice(&0x1000u32.to_le_bytes()); // VirtualAddress
        buf[0x198..0x19C].copy_from_slice(&0x400u32.to_le_bytes()); // SizeOfRawData
        buf[0x19C..0x1A0].copy_from_slice(&0x200u32.to_le_bytes()); // PointerToRawData
        let mut str_off = 0x300usize;
        for (i, dll) in imports.iter().enumerate() {
            let desc = 0x200 + i * 20;
            let name_rva = 0x1000u32 + (str_off - 0x200) as u32;
            buf[desc + 12..desc + 16].copy_from_slice(&name_rva.to_le_bytes());
            buf[desc + 16..desc + 20].copy_from_slice(&0x1004u32.to_le_bytes());
            let bytes = dll.as_bytes();
            buf[str_off..str_off + bytes.len()].copy_from_slice(bytes);
            buf[str_off + bytes.len()] = 0;
            str_off += bytes.len() + 1;
        }
        buf
    }

    #[test]
    fn detect_pe_kind_finds_xinput_in_sibling_engine_dll() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-sibling-xinput-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let stub_exe = tmp.join("GameStub.exe");
        fs::write(&stub_exe, build_minimal_pe64_with_imports(&["KERNEL32.dll", "EngineCore.dll"])).unwrap();
        fs::write(tmp.join("AudioCore.dll"), build_minimal_pe64_with_imports(&["KERNEL32.dll"])).unwrap();
        assert_eq!(
            detect_pe_kind(&stub_exe),
            PeLaunchKind::PcNative { is_64bit: true, prefer_opengl: false, has_gamepad_api: false }
        );

        fs::write(tmp.join("EngineCore.dll"), build_minimal_pe64_with_imports(&["KERNEL32.dll", "XINPUT1_3.dll"]))
            .unwrap();
        assert_eq!(
            detect_pe_kind(&stub_exe),
            PeLaunchKind::PcNative { is_64bit: true, prefer_opengl: false, has_gamepad_api: true }
        );
        let _ = fs::remove_dir_all(&tmp);
    }

    fn build_minimal_pe32_with_imports_and_export(imports: &[&str], export: Option<&str>, pad_to: usize) -> Vec<u8> {
        let mut buf = vec![0u8; 0x800.max(pad_to)];
        buf[0..2].copy_from_slice(b"MZ");
        buf[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        buf[0x80..0x84].copy_from_slice(b"PE\0\0");
        buf[0x84..0x86].copy_from_slice(&0x014Cu16.to_le_bytes());
        buf[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        buf[0x94..0x96].copy_from_slice(&0xE0u16.to_le_bytes());
        buf[0x98..0x9A].copy_from_slice(&0x010Bu16.to_le_bytes()); // PE32
                                                                   // DataDirectory[0] (Export) at 0x98 + 96 = 0xF8; DataDirectory[1] (Import) at 0x100
        if export.is_some() {
            buf[0xF8..0xFC].copy_from_slice(&0x1200u32.to_le_bytes());
            buf[0xFC..0x100].copy_from_slice(&0x100u32.to_le_bytes());
        }
        if !imports.is_empty() {
            buf[0x100..0x104].copy_from_slice(&0x1000u32.to_le_bytes());
        }
        // Section header #0 at 0x98 + 0xE0 = 0x178: VA=0x1000, VSize=0x600, RawSize=0x600, RawPtr=0x200
        buf[0x180..0x184].copy_from_slice(&0x600u32.to_le_bytes());
        buf[0x184..0x188].copy_from_slice(&0x1000u32.to_le_bytes());
        buf[0x188..0x18C].copy_from_slice(&0x600u32.to_le_bytes());
        buf[0x18C..0x190].copy_from_slice(&0x200u32.to_le_bytes());
        let mut str_off = 0x300usize;
        for (i, dll) in imports.iter().enumerate() {
            let desc = 0x200 + i * 20;
            let name_rva = 0x1000u32 + (str_off - 0x200) as u32;
            buf[desc + 12..desc + 16].copy_from_slice(&name_rva.to_le_bytes());
            buf[desc + 16..desc + 20].copy_from_slice(&0x1004u32.to_le_bytes());
            let bytes = dll.as_bytes();
            buf[str_off..str_off + bytes.len()].copy_from_slice(bytes);
            buf[str_off + bytes.len()] = 0;
            str_off += bytes.len() + 1;
        }
        if let Some(exp) = export {
            // Export directory at file offset 0x400 (RVA 0x1200)
            buf[0x418..0x41C].copy_from_slice(&1u32.to_le_bytes()); // NumberOfNames = 1
            buf[0x420..0x424].copy_from_slice(&0x1240u32.to_le_bytes()); // AddressOfNames RVA
            buf[0x440..0x444].copy_from_slice(&0x1250u32.to_le_bytes()); // name[0] RVA
            let bytes = exp.as_bytes();
            buf[0x450..0x450 + bytes.len()].copy_from_slice(bytes);
            buf[0x450 + bytes.len()] = 0;
        }
        buf
    }

    #[test]
    fn ddraw_forwarder_pe32_is_valid_and_replaces_ddrawcompat_xdd_idempotently() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-xdd-fwd-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();

        let fwd_path = tmp.join("fwd_check.dll");
        let fwd_bytes = build_ddraw_forwarder_pe32("xdd.dll");
        fs::write(&fwd_path, &fwd_bytes).unwrap();
        let exports = read_pe_export_names(&fwd_path).unwrap();
        assert!(exports.contains(&"DirectDrawCreate".to_owned()));
        assert!(exports.contains(&"DirectDrawCreateEx".to_owned()));
        assert!(exports.windows(2).all(|w| w[0] < w[1]), "PE AddressOfNames must be sorted");
        assert!(fwd_bytes.windows(b"ddraw.DirectDrawCreate\0".len()).any(|w| w == b"ddraw.DirectDrawCreate\0"));

        // Create a PE32 game importing xdd.dll + a 70 KiB DDrawCompat-like xdd.dll
        let exe = tmp.join("ClassicGame.exe");
        let xdd = tmp.join("xdd.dll");
        fs::write(&exe, build_minimal_pe32_with_imports_and_export(&["KERNEL32.dll", "xdd.dll"], None, 0x800)).unwrap();
        let fake_ddrawcompat = build_minimal_pe32_with_imports_and_export(
            &["dwmapi.dll", "USER32.dll"],
            Some("DirectDrawCreate"),
            70 * 1024,
        );
        fs::write(&xdd, &fake_ddrawcompat).unwrap();

        assert!(replace_ddrawcompat_xdd_if_needed(&exe));
        assert!(tmp.join("xdd.dll.ddrawcompat.bak").is_file());
        assert_eq!(fs::read(tmp.join("xdd.dll.ddrawcompat.bak")).unwrap().len(), 70 * 1024);
        assert_eq!(fs::read(&xdd).unwrap(), fwd_bytes);
        // Second call must be a no-op (forwarder already installed)
        assert!(!replace_ddrawcompat_xdd_if_needed(&exe));

        // Must NOT replace a D3D9 wrapper named xdd.dll
        let _ = fs::remove_file(tmp.join("xdd.dll.ddrawcompat.bak"));
        let d3d9_wrap = build_minimal_pe32_with_imports_and_export(
            &["dwmapi.dll", "d3d9.dll"],
            Some("DirectDrawCreate"),
            70 * 1024,
        );
        fs::write(&xdd, &d3d9_wrap).unwrap();
        assert!(!replace_ddrawcompat_xdd_if_needed(&exe));
        assert!(!tmp.join("xdd.dll.ddrawcompat.bak").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn gamepad_import_classifier_detects_standard_windows_apis() {
        assert!(is_gamepad_import("xinput1_3.dll"));
        assert!(is_gamepad_import("xinput9_1_0.dll"));
        assert!(is_gamepad_import("sdl2.dll"));
        assert!(is_gamepad_import("unityplayer.dll"));
        assert!(!is_gamepad_import("dinput8.dll"));
        assert!(!is_gamepad_import("dinput.dll"));
        assert!(!is_gamepad_import("ddraw.dll"));
        assert!(!is_gamepad_import("opengl32.dll"));
    }

    #[test]
    fn installer_detects_installed_state_and_removes_files() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-inst-state-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let installers = tmp.join("installers");
        fs::create_dir_all(&installers).unwrap();
        let setup = installers.join("setup_demo.exe");
        let slice = installers.join("setup_demo-1.bin");
        fs::write(&setup, b"MZ").unwrap();
        fs::write(&slice, b"BIN").unwrap();
        let inst = GogInstaller {
            display_name: "Demo".into(),
            slug: "demo".into(),
            setup_exe: setup.clone(),
            slice_files: vec![slice.clone()],
            total_bytes: 5,
            install_parent: tmp.clone(),
        };
        assert!(!inst.is_installed());
        fs::create_dir_all(tmp.join("demo")).unwrap();
        fs::write(tmp.join("demo").join("MicrosoftGame.config"), "<Game/>").unwrap();
        assert!(inst.is_installed());
        assert_eq!(inst.remove_installer_files(), 2);
        assert!(!setup.exists() && !slice.exists());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finalize_gog_prefers_game_task_over_launcher_case_insensitively() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-w2-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("bin")).unwrap();
        fs::write(tmp.join("Launcher.exe"), b"MZ\x90\x00").unwrap();
        fs::write(tmp.join("bin").join("witcher2.exe"), b"MZ\x90\x00").unwrap();
        fs::write(
            tmp.join("goggame-1207658930.info"),
            r#"{"gameId":"1207658930","name":"The Witcher 2","playTasks":[{"category":"launcher","isPrimary":true,"path":"Launcher.exe","type":"FileTask"},{"category":"game","isHidden":true,"path":"Bin//witcher2.exe","type":"FileTask"}]}"#,
        )
        .unwrap();
        let res = finalize_gog_game(&tmp, "Fallback").unwrap();
        assert_eq!(res.executable_rel, "bin\\witcher2.exe");
        let xml = fs::read_to_string(tmp.join("MicrosoftGame.config")).unwrap();
        assert!(xml.contains("bin\\witcher2.exe"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn discovers_unpacked_pc_game_and_classic_app_layout() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-pc-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("app")).unwrap();
        fs::write(tmp.join("app").join("options.exe"), b"MZ\x90\x00small").unwrap();
        fs::write(tmp.join("app").join("GameMain.exe"), vec![b'M', b'Z', 0x90, 0x00, 1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let found = discover_unpacked_pc_game(&tmp, "classic-game", "/mnt/ext1/xboxseries").unwrap();
        assert_eq!(found.name, "classic game");
        assert!(found.executable.ends_with("GameMain.exe"));
        let fin = finalize_gog_game(&tmp, "Classic Game").unwrap();
        assert_eq!(fin.executable_rel, "app\\GameMain.exe");
        assert!(tmp.join("MicrosoftGame.config").is_file());
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn find_pc_executables_keeps_config_setup_tools_as_secondary_and_excludes_installers() {
        let tmp = std::env::temp_dir().join(format!("trinityxsx-setup-exes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("MainGame.exe"), vec![b'M', b'Z', 0x90, 0x00, 1, 2, 3, 4]).unwrap();
        fs::write(tmp.join("Game_Setup_DX8.exe"), vec![b'M', b'Z', 0x90, 0x00, 1, 2]).unwrap();
        fs::write(tmp.join("unins000.exe"), b"MZ\x90\x00").unwrap();
        fs::write(tmp.join("dxsetup.exe"), b"MZ\x90\x00").unwrap();
        fs::write(tmp.join("setup_offline_installer.exe"), b"MZ\x90\x00").unwrap();
        let exes = find_pc_executables(&tmp);
        assert_eq!(exes, vec!["MainGame.exe".to_owned(), "Game_Setup_DX8.exe".to_owned()]);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn parses_gog_script_registry_actions_and_skips_compat_shims() {
        let script = r#"{
            "actions": [
                {
                    "install": {
                        "action": "setRegistry",
                        "arguments": {
                            "root": "HKEY_LOCAL_MACHINE",
                            "subkey": "SOFTWARE\\Vendor\\ClassicGame",
                            "valueName": "InstallDir",
                            "valueData": "{app}",
                            "valueType": "string"
                        }
                    }
                },
                {
                    "install": {
                        "action": "setRegistry",
                        "arguments": {
                            "root": "HKEY_LOCAL_MACHINE",
                            "subkey": "SOFTWARE\\Vendor\\ClassicGame",
                            "valueName": "CompleteInstall",
                            "valueData": "00000001",
                            "valueType": "dword"
                        }
                    }
                },
                {
                    "install": {
                        "action": "setRegistry",
                        "arguments": {
                            "root": "HKEY_CURRENT_USER",
                            "subkey": "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\AppCompatFlags\\Layers",
                            "valueName": "{app}\\Setup.exe",
                            "valueData": "RUNASADMIN",
                            "valueType": "string"
                        }
                    }
                },
                {
                    "install": {
                        "action": "setRegistry",
                        "arguments": {
                            "deleteSubkeys": true,
                            "root": "HKEY_LOCAL_MACHINE",
                            "subkey": "SOFTWARE\\Vendor\\Old"
                        }
                    }
                }
            ]
        }"#;
        let entries = parse_gog_script_registry(script, "Z:\\mnt\\ext1\\xboxseries\\classic\\");
        assert_eq!(
            entries,
            vec![
                GogRegEntry {
                    root: GogRegRoot::LocalMachine,
                    subkey: "SOFTWARE\\Vendor\\ClassicGame".into(),
                    name: "InstallDir".into(),
                    value: GogRegValue::String("Z:\\mnt\\ext1\\xboxseries\\classic".into()),
                },
                GogRegEntry {
                    root: GogRegRoot::LocalMachine,
                    subkey: "SOFTWARE\\Vendor\\ClassicGame".into(),
                    name: "CompleteInstall".into(),
                    value: GogRegValue::Dword(1),
                },
            ]
        );
    }
}
