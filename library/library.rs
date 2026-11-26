//! Game folder discovery, multi-folder configuration, per-title settings, and saves summary for TrinityXSX.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::{config::Config, launcher, library_i18n::Lang};

pub const TRINITYXSX_VERSION: &str = "v0.2.7";

const PS5_STORAGE_ROOT: &str = "Z:\\data\\homebrew\\PPSA83601";
const MAX_STORED_PATH_LEN: usize = 1024;
const MAX_SAVES_WALK_FILES: usize = 4096;

#[derive(Debug, Clone)]
pub struct Game {
    pub name: String,
    pub identity: String,
    pub version: String,
    pub title_id: String,
    pub executable: PathBuf,
    pub folder: String,
    pub logo: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderStatus {
    pub path: String,
    pub available: bool,
    pub game_count: usize,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub games: Vec<Game>,
    pub skipped: usize,
    pub folders: Vec<FolderStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserEntry {
    pub name: String,
    pub stored_path: String,
    pub is_game: bool,
}

struct Metadata {
    name: String,
    identity: String,
    version: String,
    title_id: String,
    executable: String,
    logo_candidates: Vec<String>,
}

fn attribute(xml: &str, tag: &str, key: &str) -> Option<String> {
    launcher::xml_attrs(xml, tag, key).into_iter().next()
}

fn element_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)?;
    let value = xml[start..start + end].trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn is_safe_relative_asset(rel: &str) -> bool {
    if rel.is_empty() || rel.contains([':', '\0']) {
        return false;
    }
    let path = Path::new(rel);
    path.components().all(|c| matches!(c, Component::Normal(_)))
        && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

fn metadata(raw: &[u8]) -> Result<Metadata, &'static str> {
    let xml = launcher::decode_game_config(raw).ok_or("unsupported manifest encoding")?;
    let identity = attribute(&xml, "Identity", "Name").ok_or("missing Identity Name")?;
    if identity.is_empty()
        || identity.len() > 50
        || !identity.as_bytes()[0].is_ascii_alphanumeric()
        || !identity.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        || identity.ends_with('.')
    {
        return Err("invalid package identity");
    }
    let mut choices = Vec::new();
    for (start, _) in xml.match_indices("<Executable") {
        let Some(end) = xml[start..].find('>') else { continue };
        let tag = &xml[start..start + end + 1];
        if !attribute(tag, "Executable", "TargetDeviceFamily").is_some_and(|s| s.eq_ignore_ascii_case("Scarlett"))
            || attribute(tag, "Executable", "IsDevOnly").is_some_and(|s| s.eq_ignore_ascii_case("true") || s == "1")
        {
            continue;
        }
        if let Some(name) = attribute(tag, "Executable", "Name") {
            let default = attribute(tag, "Executable", "Id").is_some_and(|s| s.eq_ignore_ascii_case("Game"));
            choices.push((default, name));
        }
    }
    if choices.iter().any(|(default, _)| *default) {
        choices.retain(|(default, _)| *default);
    }
    let [(_, executable)] = choices.as_slice() else { return Err("expected one Scarlett game executable") };
    let path = Path::new(executable);
    if executable.contains([':', '\0'])
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
        || !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return Err("invalid executable path");
    }
    let name = attribute(&xml, "ShellVisuals", "DefaultDisplayName")
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty() && !name.starts_with("ms-resource:"))
        .unwrap_or_else(|| identity.clone());

    let mut logo_candidates = Vec::new();
    for key in ["Square150x150Logo", "StoreLogo", "Square480x480Logo", "SplashScreenImage", "Square44x44Logo"] {
        if let Some(val) = attribute(&xml, "ShellVisuals", key) {
            let trimmed = val.trim();
            if is_safe_relative_asset(trimmed) {
                logo_candidates.push(trimmed.to_owned());
            }
        }
    }

    Ok(Metadata {
        name,
        identity,
        version: attribute(&xml, "Identity", "Version").unwrap_or_default(),
        title_id: element_text(&xml, "TitleId").unwrap_or_default(),
        executable: executable.clone(),
        logo_candidates,
    })
}

fn find_logo_in_folder(folder: &Path, candidates: &[String]) -> Option<PathBuf> {
    // Prefer custom cover.png if placed by the user, then manifest PNGs
    for rel in
        std::iter::once("cover.png").chain(candidates.iter().map(String::as_str)).chain(["Logo.png", "StoreLogo.png"])
    {
        if !is_safe_relative_asset(rel) {
            continue;
        }
        let joined = folder.join(rel);
        if let Ok(canon) = joined.canonicalize() {
            if canon.starts_with(folder) && canon.is_file() {
                return Some(canon);
            }
        }
    }
    None
}

fn has_manifest(folder: &Path) -> bool {
    fs::read_dir(folder)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| entry.file_name().eq_ignore_ascii_case("MicrosoftGame.config") && entry.path().is_file())
}

fn from_folder(folder: &Path, source: &str) -> Result<Game, String> {
    let folder = folder.canonicalize().map_err(|e| e.to_string())?;
    let manifest = fs::read_dir(&folder)
        .map_err(|e| e.to_string())?
        .flatten()
        .find(|entry| entry.file_name().eq_ignore_ascii_case("MicrosoftGame.config"))
        .ok_or("missing MicrosoftGame.config")?;
    if !manifest.path().is_file() {
        return Err("manifest must be a regular file".into());
    }
    let mut bytes = Vec::new();
    const LIMIT: usize = 256 * 1024;
    File::open(manifest.path())
        .and_then(|file| file.take((LIMIT + 1) as u64).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > LIMIT {
        return Err("manifest exceeds 256 KiB".into());
    }
    let info = metadata(&bytes)?;
    let executable = folder.join(&info.executable).canonicalize().map_err(|e| e.to_string())?;
    if !executable.starts_with(&folder) || !executable.is_file() {
        return Err("executable is outside the game folder or missing".into());
    }
    let logo = find_logo_in_folder(&folder, &info.logo_candidates);
    Ok(Game {
        name: info.name,
        identity: info.identity,
        version: info.version,
        title_id: info.title_id,
        executable,
        folder: source.to_owned(),
        logo,
    })
}

/// True when running inside the PS5 Wine runtime where `Z:\` maps to the PS5 root filesystem.
pub fn ps5_environment() -> bool {
    Path::new("Z:\\mnt").is_dir() || Path::new("Z:\\data").is_dir()
}

/// Persistent file storing configured game folder paths (one per line), matching TrinityX360.
pub fn paths_file() -> PathBuf {
    let ps5_root = Path::new(PS5_STORAGE_ROOT);
    if ps5_root.is_dir() {
        return ps5_root.join("game_paths.txt");
    }
    launcher::data_root().join("game_paths.txt")
}

/// Normalize a user or persisted folder path.
pub fn normalize_path(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_STORED_PATH_LEN || trimmed.contains(['\0', '\r', '\n']) {
        return None;
    }
    let trimmed = trimmed.strip_prefix("\\\\?\\").unwrap_or(trimmed);

    let unix_input =
        if trimmed.len() >= 2 && trimmed.as_bytes()[0].eq_ignore_ascii_case(&b'z') && trimmed.as_bytes()[1] == b':' {
            let rest = &trimmed[2..];
            if !rest.is_empty() && !rest.starts_with('\\') && !rest.starts_with('/') {
                return None;
            }
            Some(rest.replace('\\', "/"))
        } else if trimmed.starts_with('/') {
            if trimmed.contains('\\') {
                return None;
            }
            Some(trimmed.to_owned())
        } else {
            None
        };

    if let Some(unix) = unix_input {
        let mut segments: Vec<&str> = Vec::new();
        for part in unix.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    segments.pop();
                }
                seg => segments.push(seg),
            }
        }
        if segments.is_empty() {
            return None;
        }
        return Some(format!("/{}", segments.join("/")));
    }

    let bytes = trimmed.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/')
    {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let mut segments: Vec<&str> = Vec::new();
        for part in trimmed[3..].split(['\\', '/']) {
            match part {
                "" | "." => {}
                ".." => {
                    segments.pop();
                }
                seg => segments.push(seg),
            }
        }
        if segments.is_empty() {
            return None;
        }
        return Some(format!("{drive}:\\{}", segments.join("\\")));
    }

    None
}

/// Convert a stored or browser path into a filesystem path readable by Windows/Wine APIs.
pub fn to_fs_path(stored: &str) -> PathBuf {
    if stored == "/" {
        return PathBuf::from("Z:\\");
    }
    if let Some(rest) = stored.strip_prefix('/') {
        return PathBuf::from(format!("Z:\\{}", rest.replace('/', "\\")));
    }
    PathBuf::from(stored)
}

/// Format a path for human display in the UI (converts `Z:\...` to `/...`).
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let stripped = text.strip_prefix("\\\\?\\").unwrap_or(&text);
    if let Some(normalized) = normalize_path(stripped) {
        return normalized;
    }
    if stripped.eq_ignore_ascii_case("Z:\\") || stripped.eq_ignore_ascii_case("Z:/") {
        return "/".into();
    }
    stripped.to_owned()
}

fn path_dedup_key(stored: &str) -> String {
    if stored.starts_with('/') {
        stored.to_owned()
    } else {
        stored.to_lowercase()
    }
}

pub fn push_unique_path(paths: &mut Vec<String>, candidate: &str) -> Option<String> {
    let normalized = normalize_path(candidate)?;
    let key = path_dedup_key(&normalized);
    if !paths.iter().any(|existing| path_dedup_key(existing) == key) {
        paths.push(normalized.clone());
    }
    Some(normalized)
}

/// Default game locations, modeled after TrinityX360's `DefaultGamePaths()`.
pub fn default_paths(primary_root: Option<&Path>) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(root) = primary_root {
        let candidate = if root.is_absolute() {
            root.to_string_lossy().into_owned()
        } else {
            fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf()).to_string_lossy().into_owned()
        };
        let _ = push_unique_path(&mut paths, &candidate);
    }
    if ps5_environment() {
        for path in [
            "/data/homebrew/PPSA83601/games",
            "/data/xboxseries",
            "/mnt/usb0/xboxseries",
            "/mnt/usb1/xboxseries",
            "/mnt/ext0/xboxseries",
            "/mnt/ext1/xboxseries",
        ] {
            let _ = push_unique_path(&mut paths, path);
        }
    }
    paths
}

pub fn read_paths(file: &Path) -> Option<Vec<String>> {
    let text = fs::read_to_string(file).ok()?;
    let mut loaded = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(normalized) = normalize_path(trimmed) {
            if seen.insert(path_dedup_key(&normalized)) {
                loaded.push(normalized);
            }
        }
    }
    Some(loaded)
}

pub fn write_paths(file: &Path, paths: &[String]) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut content = String::new();
    for path in paths {
        content.push_str(path);
        content.push('\n');
    }
    let tmp = PathBuf::from(format!("{}.tmp", file.display()));
    {
        let mut out = File::create(&tmp)?;
        out.write_all(content.as_bytes())?;
        out.sync_all()?;
    }
    if let Err(err) = fs::rename(&tmp, file) {
        if file.exists() {
            fs::remove_file(file)?;
            if let Err(retry_err) = fs::rename(&tmp, file) {
                let _ = fs::remove_file(&tmp);
                return Err(retry_err);
            }
        } else {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    }
    Ok(())
}

pub fn load_or_init_paths(file: &Path, primary_root: Option<&Path>) -> Vec<String> {
    if let Some(paths) = read_paths(file) {
        return paths;
    }
    let defaults = default_paths(primary_root);
    let _ = write_paths(file, &defaults);
    defaults
}

fn should_skip_dir_name(name: &str) -> bool {
    name.starts_with('.') || name.starts_with('$') || name.eq_ignore_ascii_case("System Volume Information")
}

fn scan_directory_into(
    root: &Path,
    source_label: &str,
    depth: u8,
    seen_execs: &mut HashSet<String>,
    games: &mut Vec<Game>,
    skipped: &mut usize,
) -> usize {
    let mut added_here = 0;

    if has_manifest(root) {
        match from_folder(root, source_label) {
            Ok(game) => {
                let key = game.executable.to_string_lossy().to_lowercase();
                if seen_execs.insert(key) {
                    games.push(game);
                    added_here += 1;
                }
            }
            Err(error) => {
                *skipped += 1;
                eprintln!("TrinityXSX: skipped {} ({error})", root.display());
            }
        }
        return added_here;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if should_skip_dir_name(&name) {
            continue;
        }
        if has_manifest(&path) {
            match from_folder(&path, source_label) {
                Ok(game) => {
                    let key = game.executable.to_string_lossy().to_lowercase();
                    if seen_execs.insert(key) {
                        games.push(game);
                        added_here += 1;
                    }
                }
                Err(error) => {
                    *skipped += 1;
                    eprintln!("TrinityXSX: skipped {} ({error})", name);
                }
            }
        } else if depth > 0 {
            let nested = scan_directory_into(&path, source_label, depth - 1, seen_execs, games, skipped);
            if nested > 0 {
                added_here += nested;
            } else if depth == 1 {
                *skipped += 1;
                eprintln!("TrinityXSX: skipped {} (missing MicrosoftGame.config)", name);
            }
        } else {
            *skipped += 1;
            eprintln!("TrinityXSX: skipped {} (missing MicrosoftGame.config)", name);
        }
    }
    added_here
}

pub fn scan(root: &Path) -> io::Result<(Vec<Game>, usize)> {
    let root = root.canonicalize()?;
    let label = display_path(&root);
    let mut games = Vec::new();
    let mut seen_execs = HashSet::new();
    let mut skipped = 0;
    scan_directory_into(&root, &label, 1, &mut seen_execs, &mut games, &mut skipped);
    games.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.executable.cmp(&b.executable))
    });
    Ok((games, skipped))
}

pub fn scan_paths(paths: &[String]) -> Catalog {
    let mut games = Vec::new();
    let mut seen_execs = HashSet::new();
    let mut skipped = 0;
    let mut folders = Vec::with_capacity(paths.len());

    for stored in paths {
        let fs_path = to_fs_path(stored);
        if !fs_path.is_dir() {
            folders.push(FolderStatus { path: stored.clone(), available: false, game_count: 0 });
            continue;
        }
        let count = scan_directory_into(&fs_path, stored, 1, &mut seen_execs, &mut games, &mut skipped);
        folders.push(FolderStatus { path: stored.clone(), available: true, game_count: count });
    }

    games.sort_by(|a, b| {
        a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.executable.cmp(&b.executable))
    });
    Catalog { games, skipped, folders }
}

pub fn browser_initial_path(selected: Option<&str>) -> String {
    if let Some(path) = selected {
        if to_fs_path(path).is_dir() {
            return path.to_owned();
        }
    }
    if ps5_environment() {
        if Path::new("Z:\\mnt").is_dir() {
            return "/mnt".into();
        }
        return "/".into();
    }
    if let Some(path) = selected {
        if let Some(parent) = browser_parent(path) {
            if to_fs_path(&parent).is_dir() {
                return parent;
            }
        }
    }
    "C:\\".into()
}

pub fn browser_parent(current: &str) -> Option<String> {
    if current == "/" {
        return None;
    }
    if let Some(rest) = current.strip_prefix('/') {
        let trimmed = rest.trim_end_matches('/');
        if trimmed.is_empty() {
            return None;
        }
        return match trimmed.rfind('/') {
            Some(idx) => Some(format!("/{}", &trimmed[..idx])),
            None => Some("/".into()),
        };
    }
    let bytes = current.as_bytes();
    if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        let drive = (bytes[0] as char).to_ascii_uppercase();
        let trimmed = current[3..].trim_end_matches(['\\', '/']);
        if trimmed.is_empty() {
            return None;
        }
        return match trimmed.rfind(['\\', '/']) {
            Some(idx) => Some(format!("{drive}:\\{}", &trimmed[..idx])),
            None => Some(format!("{drive}:\\")),
        };
    }
    None
}

pub fn list_subfolders(current: &str) -> Result<Vec<BrowserEntry>, String> {
    let dir = to_fs_path(current);
    let read = fs::read_dir(&dir).map_err(|_| "This folder is not accessible.".to_owned())?;
    let mut entries = Vec::new();
    for item in read.flatten() {
        let path = item.path();
        if !path.is_dir() {
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let stored_path = if current == "/" {
            format!("/{name}")
        } else if current.starts_with('/') {
            format!("{}/{name}", current.trim_end_matches('/'))
        } else if current.ends_with('\\') {
            format!("{current}{name}")
        } else {
            format!("{current}\\{name}")
        };
        let is_game = has_manifest(&path);
        entries.push(BrowserEntry { name, stored_path, is_game });
    }
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

#[derive(Debug, Clone, Copy)]
pub struct CompatPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub summary: &'static str,
}

pub const NAMED_PRESETS: &[CompatPreset] = &[
    CompatPreset {
        id: "recommended",
        label: "Recommended (Auto / TitleId)",
        summary: "Applies tested TitleId preset if known, otherwise PS5 Safe Offline (Series S, 6144 MiB).",
    },
    CompatPreset {
        id: "safe_low_ram",
        label: "PS5 Low-RAM Safe (4096 MiB)",
        summary: "Xbox Series S with 4096 MiB title RAM and offline mode for memory-constrained titles.",
    },
    CompatPreset {
        id: "dxc_shaders",
        label: "PS5 DXC Shader Fallback",
        summary: "Disables direct_dxil (uses DXC) to work around shader translation artifacts.",
    },
    CompatPreset {
        id: "series_x",
        label: "Xbox Series X (Anaconda 6144 MiB)",
        summary: "Reports Series X (anaconda) with 1536 MiB coherent UMA window for titles requiring Series X.",
    },
    CompatPreset {
        id: "debug_trace",
        label: "Bug Report Diagnostics",
        summary: "Enables debug log level, activity counters, network tracing, and immediate log flushing.",
    },
];

pub fn lookup_tested_preset_label(title_id: &str, identity: &str) -> Option<&'static str> {
    if title_id.eq_ignore_ascii_case("639AA8DD") || identity.eq_ignore_ascii_case("Microsoft.MinecraftBeta") {
        return Some("Minecraft Preview (Verified 60 FPS)");
    }
    if identity.eq_ignore_ascii_case("XCL.TestTitle") {
        return Some("XCL Test Title (Verified 51/51)");
    }
    None
}

/// Recommended PS5 defaults (plus known tested title presets matched by TitleId or Identity).
pub fn default_config_for_title(title_id: &str, identity: &str) -> Config {
    let mut config = Config {
        device_type: "lockhart".into(),
        console_type_raw: 5,
        gamertag: "PS5Player".into(),
        user_id: 2533274790395905,
        title_memory_mb: 6144,
        coherent_mb: 1024,
        raytracing: "off".into(),
        keyboard_mouse: false,
        direct_dxil: "all".into(),
        network: false,
        xbox_live_tokens: false,
        window_width: 1920,
        window_height: 1080,
        window_pos: "0,0".into(),
        ..Config::default()
    };
    if title_id.eq_ignore_ascii_case("639AA8DD") || identity.eq_ignore_ascii_case("Microsoft.MinecraftBeta") {
        config.title_memory = "standard".into();
        config.pls_mb = 2048;
        config.uma_mb = 2048;
    }
    config
}

/// Recommended PS5 defaults (plus known tested title presets).
pub fn default_config(identity: &str) -> Config {
    default_config_for_title("", identity)
}

/// Apply one of `NAMED_PRESETS` onto an existing config while preserving the active profile.
pub fn apply_named_preset(game: &Game, current: &Config, preset_id: &str) -> Config {
    let mut cfg = default_config_for_title(&game.title_id, &game.identity);
    if !current.gamertag.is_empty() {
        cfg.gamertag = current.gamertag.clone();
    }
    if current.user_id != 0 {
        cfg.user_id = current.user_id;
    }
    match preset_id {
        "safe_low_ram" => {
            cfg.device_type = "lockhart".into();
            cfg.console_type_raw = 5;
            cfg.title_memory_mb = 4096;
            cfg.coherent_mb = 1024;
            cfg.direct_dxil = "all".into();
            cfg.raytracing = "off".into();
            cfg.network = false;
            cfg.xbox_live_tokens = false;
        }
        "dxc_shaders" => {
            cfg.direct_dxil = String::new();
            cfg.raytracing = "off".into();
            cfg.network = false;
            cfg.xbox_live_tokens = false;
        }
        "series_x" => {
            cfg.device_type = "anaconda".into();
            cfg.console_type_raw = 6;
            cfg.title_memory_mb = 6144;
            cfg.coherent_mb = 1536;
            cfg.direct_dxil = "all".into();
            cfg.raytracing = "off".into();
            cfg.network = false;
            cfg.xbox_live_tokens = false;
        }
        "debug_trace" => {
            cfg = current.clone();
            cfg.log_level = "debug".into();
            cfg.activity = true;
            cfg.trace_net = true;
            cfg.live_flush = true;
        }
        _ => {}
    }
    cfg
}

pub fn has_tested_preset(identity: &str) -> bool {
    lookup_tested_preset_label("", identity).is_some()
}

pub fn config_path_for_title(title: &str) -> PathBuf {
    launcher::data_root().join(title).join("xcl.toml")
}

pub fn saves_dir_for_title(title: &str) -> PathBuf {
    launcher::data_root().join(title).join("pls")
}

/// Load a title's `xcl.toml` if present, otherwise return its recommended preset/default.
/// Returns `(config, exists_on_disk)`.
pub fn load_game_config(game: &Game) -> (Config, bool) {
    let path = config_path_for_title(&game.identity);
    if let Ok(text) = fs::read_to_string(&path) {
        if let Ok(cfg) = toml::from_str::<Config>(&text) {
            return (cfg, true);
        }
    }
    (default_config_for_title(&game.title_id, &game.identity), false)
}

/// Save a title's `xcl.toml` atomically.
pub fn save_game_config(game: &Game, config: &Config) -> io::Result<()> {
    let path = config_path_for_title(&game.identity);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = toml::to_string_pretty(config).map_err(io::Error::other)?;
    let tmp = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(err) = fs::rename(&tmp, &path) {
        if path.exists() {
            fs::remove_file(&path)?;
            if let Err(retry) = fs::rename(&tmp, &path) {
                let _ = fs::remove_file(&tmp);
                return Err(retry);
            }
        } else {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    }
    Ok(())
}

/// Reset a title's `xcl.toml` back to its recommended PS5 preset.
pub fn reset_game_config(game: &Game) -> io::Result<Config> {
    let preset = default_config(&game.identity);
    save_game_config(game, &preset)?;
    Ok(preset)
}

/// Count files and bytes under the title's `pls/` saves folder (bounded so UI stays snappy).
pub fn saves_summary(game: &Game) -> (usize, u64) {
    let root = saves_dir_for_title(&game.identity);
    if !root.is_dir() {
        return (0, 0);
    }
    let mut stack = vec![root];
    let mut files = 0usize;
    let mut bytes = 0u64;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata() {
                files += 1;
                bytes = bytes.saturating_add(meta.len());
                if files >= MAX_SAVES_WALK_FILES {
                    return (files, bytes);
                }
            }
        }
    }
    (files, bytes)
}

fn write_initial_config(path: &Path, config: &Config) -> io::Result<()> {
    let body = toml::to_string_pretty(config).map_err(io::Error::other)?;
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => return Err(error),
    };
    if let Err(error) = file.write_all(body.as_bytes()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

pub fn prepare(game: &Game) -> Result<(), String> {
    let settings = load_ui_settings(&ui_settings_file());
    prepare_with_profile(game, settings.current_profile())
}

pub fn prepare_with_profile(game: &Game, profile: &Profile) -> Result<(), String> {
    let (_, _, title) = launcher::resolve_target(&game.executable)?;
    if crate::inspect::console_imports(&game.executable).is_none() {
        return Err("the selected executable is not a valid 64-bit Windows PE program".into());
    }
    let dir = launcher::data_root().join(title);
    let cfg_path = dir.join("xcl.toml");
    fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if cfg_path.is_file() {
        if let Ok(text) = fs::read_to_string(&cfg_path) {
            if let Ok(mut cfg) = toml::from_str::<Config>(&text) {
                if cfg.gamertag != profile.name || cfg.user_id != profile.user_id {
                    cfg.gamertag = profile.name.clone();
                    cfg.user_id = profile.user_id;
                    save_game_config(game, &cfg).map_err(|e| format!("{}: {e}", cfg_path.display()))?;
                }
            }
        }
        return Ok(());
    }
    let mut initial = default_config_for_title(&game.title_id, &game.identity);
    initial.gamertag = profile.name.clone();
    initial.user_id = profile.user_id;
    write_initial_config(&cfg_path, &initial).map_err(|e| format!("{}: {e}", dir.display()))
}

pub fn inspect_game_folder(folder: &Path) -> Result<Game, String> {
    let label = display_path(folder);
    from_folder(folder, &label)
}

/// Fast FNV-1a signature over configured folders and their direct children for auto-refresh.
pub fn folders_signature(paths: &[String]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |bytes: &[u8]| {
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for stored in paths {
        mix(stored.as_bytes());
        mix(b"|");
        let fs_path = to_fs_path(stored);
        if !fs_path.is_dir() {
            mix(b"0;");
            continue;
        }
        mix(b"1:");
        if let Ok(rd) = fs::read_dir(&fs_path) {
            let mut names: Vec<String> = rd
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| !should_skip_dir_name(n))
                .collect();
            names.sort();
            for name in names {
                mix(name.as_bytes());
                mix(b",");
            }
        }
        mix(b";");
    }
    h
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    #[default]
    Alphabetical,
    Recent,
    Favorites,
}

impl SortMode {
    pub fn code(self) -> &'static str {
        match self {
            SortMode::Alphabetical => "alpha",
            SortMode::Recent => "recent",
            SortMode::Favorites => "favorites",
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            SortMode::Alphabetical => "Sort: A-Z",
            SortMode::Recent => "Sort: Recent",
            SortMode::Favorites => "Sort: Favorites",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "recent" => SortMode::Recent,
            "favorites" | "fav" => SortMode::Favorites,
            _ => SortMode::Alphabetical,
        }
    }

    pub fn next(self) -> Self {
        match self {
            SortMode::Alphabetical => SortMode::Recent,
            SortMode::Recent => SortMode::Favorites,
            SortMode::Favorites => SortMode::Alphabetical,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            SortMode::Alphabetical => SortMode::Favorites,
            SortMode::Recent => SortMode::Alphabetical,
            SortMode::Favorites => SortMode::Recent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Profile {
    pub name: String,
    pub user_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiSettings {
    pub language: Lang,
    pub active_profile: usize,
    pub profiles: Vec<Profile>,
    pub sort_mode: SortMode,
    pub favorites: Vec<String>,
    pub recent: Vec<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct UiSettingsFile {
    language: String,
    active_profile: usize,
    profiles: Vec<Profile>,
    #[serde(default)]
    sort_mode: String,
    #[serde(default)]
    favorites: Vec<String>,
    #[serde(default)]
    recent: Vec<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            language: Lang::En,
            active_profile: 0,
            profiles: vec![
                Profile { name: "PS5Player".into(), user_id: 2533274790395905 },
                Profile { name: "Player2".into(), user_id: 2533274790395906 },
                Profile { name: "Player3".into(), user_id: 2533274790395907 },
                Profile { name: "Player4".into(), user_id: 2533274790395908 },
            ],
            sort_mode: SortMode::Alphabetical,
            favorites: Vec::new(),
            recent: Vec::new(),
        }
    }
}

impl UiSettings {
    pub fn current_profile(&self) -> &Profile {
        self.profiles.get(self.active_profile).or_else(|| self.profiles.first()).expect("profiles is never empty")
    }

    pub fn is_favorite(&self, identity: &str) -> bool {
        self.favorites.iter().any(|id| id.eq_ignore_ascii_case(identity))
    }

    pub fn toggle_favorite(&mut self, identity: &str) -> bool {
        if let Some(pos) = self.favorites.iter().position(|id| id.eq_ignore_ascii_case(identity)) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.push(identity.to_owned());
            true
        }
    }

    pub fn record_recent(&mut self, identity: &str) {
        self.recent.retain(|id| !id.eq_ignore_ascii_case(identity));
        self.recent.insert(0, identity.to_owned());
        self.recent.truncate(32);
    }

    pub fn sort_games(&self, games: &mut [Game]) {
        match self.sort_mode {
            SortMode::Alphabetical => {
                games.sort_by(|a, b| {
                    a.name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase())
                        .then_with(|| a.identity.cmp(&b.identity))
                });
            }
            SortMode::Recent => {
                games.sort_by(|a, b| {
                    let ra =
                        self.recent.iter().position(|id| id.eq_ignore_ascii_case(&a.identity)).unwrap_or(usize::MAX);
                    let rb =
                        self.recent.iter().position(|id| id.eq_ignore_ascii_case(&b.identity)).unwrap_or(usize::MAX);
                    ra.cmp(&rb).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
                });
            }
            SortMode::Favorites => {
                games.sort_by(|a, b| {
                    let fa = !self.is_favorite(&a.identity);
                    let fb = !self.is_favorite(&b.identity);
                    fa.cmp(&fb).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
                });
            }
        }
    }

    pub fn add_profile(&mut self, raw_name: &str) -> Result<usize, &'static str> {
        let name = raw_name.trim();
        if name.is_empty()
            || name.len() > 15
            || !name.as_bytes()[0].is_ascii_alphabetic()
            || !name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'_' | b'-'))
        {
            return Err("Gamertag must be 1-15 ASCII letters/digits starting with a letter.");
        }
        if let Some(idx) = self.profiles.iter().position(|p| p.name.eq_ignore_ascii_case(name)) {
            self.active_profile = idx;
            return Ok(idx);
        }
        let mut hash = 0x811c_9dc5u32;
        for &b in name.to_ascii_lowercase().as_bytes() {
            hash ^= b as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        let mut xuid = 0x0009_0000_0000_0000u64 | (hash as u64).max(100);
        while self.profiles.iter().any(|p| p.user_id == xuid) {
            xuid += 1;
        }
        self.profiles.push(Profile { name: name.to_owned(), user_id: xuid });
        let idx = self.profiles.len() - 1;
        self.active_profile = idx;
        Ok(idx)
    }
}

/// Writable root directory for TrinityXSX metadata files (`/data/homebrew/PPSA83601` on PS5).
pub fn storage_root_dir() -> PathBuf {
    let ps5_root = Path::new(PS5_STORAGE_ROOT);
    if ps5_root.is_dir() {
        return ps5_root.to_path_buf();
    }
    launcher::data_root()
}

/// Request a clean TrinityXSX relaunch from `launcher.c` when `xcl-library.exe` exits.
pub fn request_app_restart() -> io::Result<()> {
    let root = storage_root_dir();
    fs::create_dir_all(&root)?;
    fs::write(root.join("restart.flag"), b"1\n")
}

/// Record the executable path of the game being launched so the Touchpad menu's
/// `RESTART GAME` action can relaunch it directly, and clear any stale restart flag.
pub fn save_last_launched_game(exe: &Path) -> io::Result<()> {
    let root = storage_root_dir();
    fs::create_dir_all(&root)?;
    let _ = fs::remove_file(root.join("restart_game.flag"));
    fs::write(root.join("last_game.txt"), format!("{}\n", exe.to_string_lossy()))
}

/// Read the last launched game's executable path (for pre-selecting in the library).
pub fn read_last_launched_game() -> Option<PathBuf> {
    let text = fs::read_to_string(storage_root_dir().join("last_game.txt")).ok()?;
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(PathBuf::from(line))
    }
}

/// Consume `restart_game.flag` (one-shot so a crash never boot-loops) and return the
/// last launched game's executable if it still exists on disk.
pub fn take_restart_game_target() -> Option<PathBuf> {
    let root = storage_root_dir();
    let flag = root.join("restart_game.flag");
    if !flag.is_file() {
        return None;
    }
    let _ = fs::remove_file(&flag);
    let exe = read_last_launched_game()?;
    exe.is_file().then_some(exe)
}

fn shader_cache_dirs(title: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if !dirs.iter().any(|d| d == &p) {
            dirs.push(p);
        }
    };
    push_unique(crate::gpu::recomp::dxc::cache_dir());
    if let Ok(vk) = std::env::var("VKD3D_SHADER_CACHE_PATH") {
        let trimmed = vk.trim();
        if !trimmed.is_empty() {
            push_unique(PathBuf::from(trimmed));
        }
    } else if Path::new("C:\\XCL\\cache\\trinityxsx-library").is_dir() {
        push_unique(PathBuf::from("C:\\XCL\\cache\\trinityxsx-library"));
    }
    if let Some(t) = title {
        if !t.is_empty() {
            push_unique(launcher::data_root().join(t).join("cache"));
        }
    }
    dirs
}

fn count_dir_files(root: &Path) -> (usize, u64) {
    if !root.is_dir() {
        return (0, 0);
    }
    let mut files = 0usize;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(meta) = entry.metadata() {
                files += 1;
                bytes = bytes.saturating_add(meta.len());
            }
        }
    }
    (files, bytes)
}

fn clear_dir_contents(root: &Path) -> (usize, u64) {
    if !root.is_dir() {
        return (0, 0);
    }
    let mut removed = 0usize;
    let mut freed = 0u64;
    let Ok(rd) = fs::read_dir(root) else {
        return (0, 0);
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let (f, b) = clear_dir_contents(&p);
            removed += f;
            freed = freed.saturating_add(b);
            let _ = fs::remove_dir(&p);
        } else if let Ok(meta) = entry.metadata() {
            let len = meta.len();
            if fs::remove_file(&p).is_ok() {
                removed += 1;
                freed = freed.saturating_add(len);
            }
        }
    }
    (removed, freed)
}

/// Total file count and size across XCL DXIL, vkd3d-proton, and optional per-title shader caches.
pub fn shader_cache_summary(title: Option<&str>) -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    for dir in shader_cache_dirs(title) {
        let (f, b) = count_dir_files(&dir);
        files += f;
        bytes = bytes.saturating_add(b);
    }
    (files, bytes)
}

/// Remove cached shader binaries without touching saves, profiles, or `xcl.toml`.
pub fn clear_shader_cache(title: Option<&str>) -> (usize, u64) {
    let mut removed = 0usize;
    let mut freed = 0u64;
    for dir in shader_cache_dirs(title) {
        let (f, b) = clear_dir_contents(&dir);
        removed += f;
        freed = freed.saturating_add(b);
    }
    (removed, freed)
}

pub fn ui_settings_file() -> PathBuf {
    storage_root_dir().join("ui_settings.toml")
}

/// Path to the prospero-win runtime profile (`trinityxsx-library.profile`) that controls
/// display options such as MihawkGen `framegen = off | 2 | 3 | 4`.
pub fn runtime_profile_file() -> PathBuf {
    let ps5_root = Path::new(PS5_STORAGE_ROOT);
    if ps5_root.is_dir() {
        return ps5_root.join("profiles").join("trinityxsx-library.profile");
    }
    launcher::data_root().join("trinityxsx-library.profile")
}

/// Read a key from `[display]` inside a prospero-win `.profile` file.
fn read_display_profile_value(profile_path: &Path, key: &str) -> Option<String> {
    let text = fs::read_to_string(profile_path).ok()?;
    let mut in_display = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_display = line[1..line.len() - 1].trim().eq_ignore_ascii_case("display");
            continue;
        }
        if in_display {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_owned());
                }
            }
        }
    }
    None
}

/// Atomically update `key = value` inside `[display]` of a prospero-win `.profile` file,
/// preserving all other sections, keys, and comments, and never emitting duplicate keys.
fn save_display_profile_value(profile_path: &Path, key: &str, value: &str) -> io::Result<()> {
    let new_line = format!("{key} = {value}");
    let existing = fs::read_to_string(profile_path).unwrap_or_default();
    let eol = if existing.contains("\r\n") { "\r\n" } else { "\n" };

    let mut out: Vec<String> = Vec::new();
    let mut in_display = false;
    let mut saw_display = false;
    let mut replaced = false;
    let mut display_insert_at: Option<usize> = None;

    for raw in existing.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if in_display && !replaced {
                let pos = display_insert_at.unwrap_or(out.len());
                out.insert(pos, new_line.clone());
                replaced = true;
            }
            in_display = trimmed[1..trimmed.len() - 1].trim().eq_ignore_ascii_case("display");
            if in_display {
                saw_display = true;
                display_insert_at = Some(out.len() + 1);
            }
            out.push(raw.to_owned());
            continue;
        }
        if in_display {
            if let Some((k, _)) = trimmed.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    if !replaced {
                        out.push(new_line.clone());
                        replaced = true;
                    }
                    continue;
                }
            }
            out.push(raw.to_owned());
            if !trimmed.is_empty() && !trimmed.starts_with(';') && !trimmed.starts_with('#') {
                display_insert_at = Some(out.len());
            }
            continue;
        }
        out.push(raw.to_owned());
    }

    if !replaced {
        if saw_display {
            let pos = display_insert_at.unwrap_or(out.len());
            out.insert(pos, new_line);
        } else {
            if !out.is_empty() && !out.last().is_some_and(|l| l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push("[display]".into());
            out.push(new_line);
        }
    }

    let mut body = out.join(eol);
    body.push_str(eol);

    if let Some(parent) = profile_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let tmp = PathBuf::from(format!("{}.tmp", profile_path.display()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(err) = fs::rename(&tmp, profile_path) {
        if profile_path.exists() {
            fs::remove_file(profile_path)?;
            if let Err(retry) = fs::rename(&tmp, profile_path) {
                let _ = fs::remove_file(&tmp);
                return Err(retry);
            }
        } else {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    }
    Ok(())
}

/// Read `[display] framegen` from a prospero-win `.profile` file (`0` = off, `2..=4` = factor).
pub fn read_framegen_setting(profile_path: &Path) -> u8 {
    match read_display_profile_value(profile_path, "framegen").as_deref().map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("2") => 2,
        Some("3") => 3,
        Some("4") => 4,
        _ => 0,
    }
}

/// Update `[display] framegen = off | 2 | 3 | 4` inside a prospero-win `.profile` file.
pub fn save_framegen_setting(profile_path: &Path, factor: u8) -> io::Result<()> {
    let value = match factor {
        2 => "2",
        3 => "3",
        4 => "4",
        _ => "off",
    };
    save_display_profile_value(profile_path, "framegen", value)
}

/// Read `[display] show_fps` from a prospero-win `.profile` file (defaults to `true`).
pub fn read_show_fps_setting(profile_path: &Path) -> bool {
    !matches!(
        read_display_profile_value(profile_path, "show_fps").as_deref().map(|s| s.to_ascii_lowercase()).as_deref(),
        Some("false" | "0" | "off")
    )
}

/// Update `[display] show_fps = true | false` inside a prospero-win `.profile` file.
pub fn save_show_fps_setting(profile_path: &Path, enabled: bool) -> io::Result<()> {
    save_display_profile_value(profile_path, "show_fps", if enabled { "true" } else { "false" })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageSpace {
    pub label: String,
    pub free_bytes: u64,
    pub total_bytes: u64,
}

fn query_disk_space(fs_path: &Path) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    if !fs_path.is_dir() {
        return None;
    }
    let wide = crate::util::wide(&fs_path.to_string_lossy());
    let mut avail = 0u64;
    let mut total = 0u64;
    let mut free = 0u64;
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut avail, &mut total, &mut free) };
    if ok != 0 && total > 0 {
        Some((avail, total))
    } else {
        None
    }
}

/// Free and total space for the main PS5 storage mounts (`/mnt/ext1`, `/data`, `/mnt/usb0`)
/// or `C:\` when running locally on Windows.
pub fn storage_space_summary() -> Vec<StorageSpace> {
    let candidates: &[(&str, &str)] = if ps5_environment() {
        &[("/mnt/ext1", "Z:\\mnt\\ext1"), ("/data", "Z:\\data"), ("/mnt/usb0", "Z:\\mnt\\usb0")]
    } else {
        &[("C:", "C:\\")]
    };
    let mut out = Vec::new();
    for &(label, path) in candidates {
        if let Some((free_bytes, total_bytes)) = query_disk_space(Path::new(path)) {
            out.push(StorageSpace { label: label.to_owned(), free_bytes, total_bytes });
        }
    }
    out
}

pub fn format_storage_line(items: &[StorageSpace]) -> String {
    if items.is_empty() {
        return String::new();
    }
    let gib = 1024.0 * 1024.0 * 1024.0;
    items
        .iter()
        .map(|s| format!("{} {:.1} GB free", s.label, (s.free_bytes as f64) / gib))
        .collect::<Vec<_>>()
        .join("  ·  ")
}

pub fn load_ui_settings(file: &Path) -> UiSettings {
    let Ok(text) = fs::read_to_string(file) else {
        return UiSettings::default();
    };
    let Ok(parsed) = toml::from_str::<UiSettingsFile>(&text) else {
        return UiSettings::default();
    };
    let profiles = if parsed.profiles.is_empty() { UiSettings::default().profiles } else { parsed.profiles };
    let active_profile = parsed.active_profile.min(profiles.len() - 1);
    UiSettings {
        language: Lang::parse(&parsed.language).unwrap_or(Lang::En),
        active_profile,
        profiles,
        sort_mode: SortMode::parse(&parsed.sort_mode),
        favorites: parsed.favorites,
        recent: parsed.recent,
    }
}

pub fn save_ui_settings(file: &Path, settings: &UiSettings) -> io::Result<()> {
    if let Some(parent) = file.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let body = toml::to_string_pretty(&UiSettingsFile {
        language: settings.language.code().into(),
        active_profile: settings.active_profile,
        profiles: settings.profiles.clone(),
        sort_mode: settings.sort_mode.code().into(),
        favorites: settings.favorites.clone(),
        recent: settings.recent.clone(),
    })
    .map_err(io::Error::other)?;
    let tmp = PathBuf::from(format!("{}.tmp", file.display()));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all()?;
    }
    if let Err(err) = fs::rename(&tmp, file) {
        if file.exists() {
            fs::remove_file(file)?;
            if let Err(retry) = fs::rename(&tmp, file) {
                let _ = fs::remove_file(&tmp);
                return Err(retry);
            }
        } else {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("xcl-library-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fake_game(dir: &Path, identity: &str, display_name: &str) {
        fs::create_dir_all(dir).unwrap();
        let manifest = format!(
            r#"<Game><Identity Name="{identity}" Version="1.0.0.0"/><TitleId>1234ABCD</TitleId>
               <ShellVisuals DefaultDisplayName="{display_name}" Square150x150Logo="Logo.png"/>
               <Executable Name="game.exe" Id="Game" TargetDeviceFamily="Scarlett"/></Game>"#
        );
        fs::write(dir.join("MicrosoftGame.config"), manifest).unwrap();
        fs::write(dir.join("game.exe"), b"MZ").unwrap();
        fs::write(dir.join("Logo.png"), b"\x89PNG\r\n\x1a\n").unwrap();
    }

    #[test]
    fn ignores_other_platforms_and_developer_entries() {
        let raw = br#"<Game><Identity Name="Example.Game" Version="1.0.0.0"/><TitleId>639AA8DD</TitleId>
            <ShellVisuals DefaultDisplayName="Example &amp; Friends" Square150x150Logo="Logo.png"/>
            <Executable Name="desktop.exe" Id="Game" TargetDeviceFamily="Desktop"/>
            <Executable Name="tools.exe" TargetDeviceFamily="Scarlett" IsDevOnly="true"/>
            <Executable Name="game.exe" Id="Game" TargetDeviceFamily="Scarlett"/>
            </Game>"#;
        let selected = metadata(raw).unwrap();
        assert_eq!(selected.executable, "game.exe");
        assert_eq!(selected.name, "Example & Friends");
        assert_eq!(selected.version, "1.0.0.0");
        assert_eq!(selected.title_id, "639AA8DD");
        assert_eq!(selected.logo_candidates, vec!["Logo.png"]);
    }

    #[test]
    fn refuses_paths_outside_the_game() {
        let raw = br#"<Game><Identity Name="Example.Game"/>
            <Executable Name="..\outside.exe" TargetDeviceFamily="Scarlett"/></Game>"#;
        assert!(metadata(raw).is_err());
    }

    #[test]
    fn existing_configuration_is_preserved() {
        let path = std::env::temp_dir().join(format!("xcl-library-{}.toml", std::process::id()));
        fs::write(&path, "network = true\n").unwrap();
        write_initial_config(&path, &default_config("Microsoft.MinecraftBeta")).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "network = true\n");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn normalizes_unix_wine_and_windows_paths() {
        assert_eq!(normalize_path("  /mnt/ext1/xboxseries/  ").as_deref(), Some("/mnt/ext1/xboxseries"));
        assert_eq!(normalize_path("Z:\\mnt\\usb0\\xboxseries\\").as_deref(), Some("/mnt/usb0/xboxseries"));
        assert_eq!(normalize_path("z:/data/games/../xboxseries").as_deref(), Some("/data/xboxseries"));
        assert_eq!(normalize_path("c:/XCL/games/").as_deref(), Some("C:\\XCL\\games"));
        assert_eq!(normalize_path("/"), None);
        assert_eq!(normalize_path("Z:\\"), None);
        assert_eq!(normalize_path("C:\\"), None);
        assert_eq!(normalize_path("relative/path"), None);
    }

    #[test]
    fn converts_stored_paths_to_wine_fs_and_navigates_parents() {
        assert_eq!(to_fs_path("/mnt/ext1/xboxseries"), PathBuf::from("Z:\\mnt\\ext1\\xboxseries"));
        assert_eq!(to_fs_path("/"), PathBuf::from("Z:\\"));
        assert_eq!(to_fs_path("C:\\XCL\\games"), PathBuf::from("C:\\XCL\\games"));

        assert_eq!(browser_parent("/mnt/ext1/xboxseries").as_deref(), Some("/mnt/ext1"));
        assert_eq!(browser_parent("/mnt").as_deref(), Some("/"));
        assert_eq!(browser_parent("/"), None);
        assert_eq!(browser_parent("C:\\XCL\\games").as_deref(), Some("C:\\XCL"));
        assert_eq!(browser_parent("C:\\XCL").as_deref(), Some("C:\\"));
        assert_eq!(browser_parent("C:\\"), None);
    }

    #[test]
    fn reads_writes_and_deduplicates_game_paths() {
        let dir = temp_dir("paths");
        let file = dir.join("game_paths.txt");
        assert!(read_paths(&file).is_none());

        let paths = vec!["/mnt/ext1/xboxseries".to_owned(), "C:\\XCL\\games".to_owned()];
        write_paths(&file, &paths).unwrap();
        assert_eq!(read_paths(&file).unwrap(), paths);

        fs::write(
            &file,
            "# comment\nZ:\\mnt\\ext1\\xboxseries\\\n/mnt/ext1/xboxseries\nc:/xcl/games\nC:\\XCL\\games\n",
        )
        .unwrap();
        assert_eq!(read_paths(&file).unwrap(), vec!["/mnt/ext1/xboxseries", "C:\\xcl\\games"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scans_multiple_folders_extracts_logo_and_tolerates_missing() {
        let dir = temp_dir("multi");
        let root_a = dir.join("a");
        let root_b = dir.join("b");
        let missing = dir.join("disconnected_usb");
        write_fake_game(&root_a.join("Alpha"), "Example.Alpha", "Alpha Game");
        write_fake_game(&root_b.join("Nested").join("Beta"), "Example.Beta", "Beta Game");

        let paths = vec![
            normalize_path(&root_a.to_string_lossy()).unwrap(),
            normalize_path(&root_a.join("Alpha").to_string_lossy()).unwrap(),
            normalize_path(&missing.to_string_lossy()).unwrap(),
            normalize_path(&root_b.to_string_lossy()).unwrap(),
        ];

        let catalog = scan_paths(&paths);
        assert_eq!(catalog.games.len(), 2);
        assert_eq!(catalog.games[0].name, "Alpha Game");
        assert_eq!(catalog.games[0].title_id, "1234ABCD");
        assert!(catalog.games[0].logo.is_some());
        assert_eq!(catalog.games[1].name, "Beta Game");
        assert!(!catalog.folders[2].available);

        let subs = list_subfolders(&normalize_path(&root_a.to_string_lossy()).unwrap()).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "Alpha");
        assert!(subs[0].is_game);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn persists_ui_settings_profiles_and_detects_folder_changes() {
        let dir = temp_dir("ui");
        let file = dir.join("ui_settings.toml");
        let mut ui = load_ui_settings(&file);
        assert_eq!(ui.language, Lang::En);
        assert_eq!(ui.current_profile().name, "PS5Player");

        assert!(ui.add_profile("123bad").is_err());
        let idx = ui.add_profile("Grzegorz").unwrap();
        assert_eq!(ui.active_profile, idx);
        ui.language = Lang::Pl;
        save_ui_settings(&file, &ui).unwrap();

        let reloaded = load_ui_settings(&file);
        assert_eq!(reloaded.language, Lang::Pl);
        assert_eq!(reloaded.current_profile().name, "Grzegorz");

        let root = dir.join("games");
        fs::create_dir_all(&root).unwrap();
        let stored = vec![normalize_path(&root.to_string_lossy()).unwrap()];
        let sig1 = folders_signature(&stored);
        fs::create_dir_all(root.join("NewGame")).unwrap();
        let sig2 = folders_signature(&stored);
        assert_ne!(sig1, sig2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_and_updates_framegen_in_runtime_profile() {
        let dir = temp_dir("framegen");
        let profile = dir.join("trinityxsx-library.profile");
        let initial = "; comment\n[application]\nid = trinityxsx-library\n\n[display]\ndesktop = 1920x1080\n\n[input]\nmode = xinput\n";
        fs::write(&profile, initial).unwrap();
        assert_eq!(read_framegen_setting(&profile), 0);

        save_framegen_setting(&profile, 2).unwrap();
        assert_eq!(read_framegen_setting(&profile), 2);
        let text = fs::read_to_string(&profile).unwrap();
        assert!(text.contains("desktop = 1920x1080\nframegen = 2\n"));
        assert!(text.contains("[input]\nmode = xinput\n"));

        save_framegen_setting(&profile, 4).unwrap();
        assert_eq!(read_framegen_setting(&profile), 4);
        assert_eq!(fs::read_to_string(&profile).unwrap().matches("framegen =").count(), 1);

        save_framegen_setting(&profile, 0).unwrap();
        assert_eq!(read_framegen_setting(&profile), 0);
        assert!(fs::read_to_string(&profile).unwrap().contains("framegen = off\n"));

        assert!(read_show_fps_setting(&profile));
        save_show_fps_setting(&profile, false).unwrap();
        assert!(!read_show_fps_setting(&profile));
        let text_fps = fs::read_to_string(&profile).unwrap();
        assert!(text_fps.contains("show_fps = false\n"));
        assert_eq!(text_fps.matches("show_fps =").count(), 1);
        save_show_fps_setting(&profile, true).unwrap();
        assert!(read_show_fps_setting(&profile));
        assert_eq!(fs::read_to_string(&profile).unwrap().matches("show_fps =").count(), 1);

        let spaces = storage_space_summary();
        assert!(!spaces.is_empty());
        assert!(!format_storage_line(&spaces).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn sorts_by_favorites_and_recent_and_clears_cache_dir() {
        let dir = temp_dir("sort-cache");
        let ui_file = dir.join("ui_settings.toml");
        let mut ui = load_ui_settings(&ui_file);
        assert!(ui.toggle_favorite("Example.Beta"));
        ui.record_recent("Example.Gamma");
        ui.sort_mode = SortMode::Favorites;
        save_ui_settings(&ui_file, &ui).unwrap();

        let reloaded = load_ui_settings(&ui_file);
        assert_eq!(reloaded.sort_mode, SortMode::Favorites);
        assert!(reloaded.is_favorite("Example.Beta"));
        assert_eq!(reloaded.recent.first().map(String::as_str), Some("Example.Gamma"));

        let mk = |name: &str, id: &str| Game {
            name: name.into(),
            identity: id.into(),
            version: "1.0".into(),
            title_id: String::new(),
            executable: dir.join("game.exe"),
            folder: "/mnt/ext1".into(),
            logo: None,
        };
        let mut games = vec![mk("Alpha", "Example.Alpha"), mk("Beta", "Example.Beta"), mk("Gamma", "Example.Gamma")];
        reloaded.sort_games(&mut games);
        assert_eq!(games[0].identity, "Example.Beta");

        let mut recent_ui = reloaded.clone();
        recent_ui.sort_mode = SortMode::Recent;
        recent_ui.sort_games(&mut games);
        assert_eq!(games[0].identity, "Example.Gamma");

        let cache = dir.join("cache");
        fs::create_dir_all(cache.join("sub")).unwrap();
        fs::write(cache.join("a.bin"), b"12345").unwrap();
        fs::write(cache.join("sub/b.bin"), b"67890").unwrap();
        assert_eq!(count_dir_files(&cache), (2, 10));
        assert_eq!(clear_dir_contents(&cache), (2, 10));
        assert_eq!(count_dir_files(&cache), (0, 0));

        let _ = fs::remove_dir_all(&dir);
    }
}
