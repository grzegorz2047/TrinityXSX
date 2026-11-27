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
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use windows_sys::Win32::Foundation::FreeLibrary;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

use crate::library;

const PS5_STORAGE_ROOT: &str = "Z:\\data\\homebrew\\PPSA83601";
const PC_PROFILE_ID: &str = "trinityxsx-pc-game";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeLaunchKind {
    /// Imports console-only DLLs (`d3d12_xs.dll`, `xgameplatform.dll`, `xcrdapi.dll`, etc.)
    XboxSeries,
    /// Standard Windows PC executable (e.g. GOG DRM-free game) — best launched via prospero-win
    PcNative { is_64bit: bool, prefer_opengl: bool },
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

/// Inspect a PE executable's architecture (PE32 vs PE32+) and import table to determine
/// whether it is a native Xbox Series X|S binary or a standard Windows PC/GOG game.
pub fn detect_pe_kind(exe_path: &Path) -> PeLaunchKind {
    let Ok(mut f) = File::open(exe_path) else {
        return PeLaunchKind::XboxSeries;
    };
    // Read up to 4 MiB (headers + import directory live near the front in virtually all PEs;
    // if the import table sits further out we fall back to reading the exact slice).
    let mut header = vec![0u8; 64 * 1024];
    let n = f.read(&mut header).unwrap_or(0);
    header.truncate(n);
    if header.len() < 0x100 || &header[0..2] != b"MZ" {
        return PeLaunchKind::XboxSeries;
    }
    let Some(pe_off) = read_u32_le(&header, 0x3C).map(|v| v as usize) else {
        return PeLaunchKind::XboxSeries;
    };
    if header.get(pe_off..pe_off + 4) != Some(b"PE\0\0") {
        return PeLaunchKind::XboxSeries;
    }
    let num_sections = read_u16_le(&header, pe_off + 6).unwrap_or(0) as usize;
    let opt_size = read_u16_le(&header, pe_off + 20).unwrap_or(0) as usize;
    let opt_off = pe_off + 24;
    let magic = read_u16_le(&header, opt_off).unwrap_or(0);
    let (is_64bit, data_dir_off) = match magic {
        0x20B => (true, opt_off + 112), // PE32+
        0x10B => (false, opt_off + 96), // PE32
        _ => return PeLaunchKind::XboxSeries,
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
            if let Ok(full) = fs::read(exe_path) {
                let mut idx = 0usize;
                while idx < 256 {
                    let desc = imp_file_off + idx * 20;
                    let Some(name_rva) = read_u32_le(&full, desc + 12) else { break };
                    let ft_rva = read_u32_le(&full, desc + 16).unwrap_or(0);
                    if name_rva == 0 && ft_rva == 0 {
                        break;
                    }
                    if let Some(name_off) = rva_to_off(name_rva) {
                        if let Some(end) = full.get(name_off..).and_then(|s| s.iter().position(|&b| b == 0)) {
                            if let Ok(s) = std::str::from_utf8(&full[name_off..name_off + end.min(128)]) {
                                imports.push(s.to_ascii_lowercase());
                            }
                        }
                    }
                    idx += 1;
                }
            }
        }
    }

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

    PeLaunchKind::PcNative { is_64bit, prefer_opengl }
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
            if low.contains("unins") || low.contains("setup") || low.contains("redist") || low.contains("crashhandler")
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
    Ok(FinalizedGog { name, identity, executable_rel: exe_rel_clean, has_cover, pe_kind })
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

/// Write a dynamic `prospero-win` `.profile` (`trinityxsx-pc-game.profile`) + `pc_launch.flag`
/// so `pw_runner.bin` launches a standard 32-bit or 64-bit PC/GOG game directly under Wine +
/// DXVK / vkd3d-proton / Zink with zero XCL UMA overhead and Touchpad Return-to-Library support.
pub fn write_pc_handoff(game_name: &str, exe_path: &Path, is_64bit: bool, prefer_opengl: bool) -> Result<(), String> {
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

    let profile_text = format_pc_handoff_profile(safe_name, &clean_exe, &work_dir, arch, gfx, fg_val, fps_val);

    let profile_file = format!("{PC_PROFILE_ID}.profile");
    fs::write(profiles_dir.join(&profile_file), profile_text)
        .map_err(|e| format!("Cannot write PC game profile: {e}"))?;
    ensure_profile_listed(&profiles_dir, &profile_file).map_err(|e| format!("Cannot update profiles.lst: {e}"))?;
    fs::write(root.join("pc_launch.flag"), format!("{PC_PROFILE_ID}\n"))
        .map_err(|e| format!("Cannot write pc_launch.flag: {e}"))?;
    Ok(())
}

/// Format the dynamic `prospero-win` `.profile` for PC/GOG titles:
/// - `windows.gaming.input=d` disables Wine's unimplemented WGI semi-stub so Unity 6 and
///   modern Input System games immediately fall back to DualSense XInput.
/// - `thread_scheduling = true` enables `SCHED_RR` so Wine/DXVK worker threads do not starve.
/// - `menu = true` enables `DXVK_HUD=fps,menu` so the Touchpad Quick Menu renders live over DXVK.
fn format_pc_handoff_profile(
    safe_name: &str,
    clean_exe: &str,
    work_dir: &str,
    arch: &str,
    gfx: &str,
    fg_val: &str,
    fps_val: &str,
) -> String {
    format!(
        "; Generated dynamically by TrinityXSX for PC/GOG game handoff\n\
         [application]\n\
         id = {PC_PROFILE_ID}\n\
         name = {safe_name}\n\
         executable = {clean_exe}\n\
         working_directory = {work_dir}\n\
         dll_overrides = d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9=n,b;windows.gaming.input=d\n\
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
         framegen = {fg_val}\n\
         show_fps = {fps_val}\n\
         menu = true\n\
         \n\
         [input]\n\
         mode = xinput\n\
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
        );
        assert!(
            profile.contains("dll_overrides = d3d12,d3d12core,dxgi,d3d11,d3d10core,d3d9=n,b;windows.gaming.input=d")
        );
        assert!(profile.contains("[runtime]\nthread_scheduling = true"));
        assert!(profile.contains("show_fps = true\nmenu = true"));
        assert!(profile.contains("[input]\nmode = xinput"));
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
}
