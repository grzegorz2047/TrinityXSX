//! Game folder discovery, multi-folder configuration, per-title settings, and saves summary for TrinityXSX.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::{config::Config, launcher, library_gog::PcInputMode, library_i18n::Lang};

pub const TRINITYXSX_VERSION: &str = "v0.2.19";

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
    pub root_dir: PathBuf,
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
        root_dir: folder,
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
            } else if depth == 1 && !name.eq_ignore_ascii_case("installers") {
                if let Some(pc_game) = crate::library_gog::discover_unpacked_pc_game(&path, &name, source_label) {
                    let key = pc_game.executable.to_string_lossy().to_lowercase();
                    if seen_execs.insert(key) {
                        games.push(pc_game);
                        added_here += 1;
                    }
                } else {
                    *skipped += 1;
                    eprintln!("TrinityXSX: skipped {} (missing MicrosoftGame.config)", name);
                }
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

/// The game compatibility database, `library/compat.toml`, built into the library. It gives each
/// known title a status and the `xcl.toml` keys it needs on top of the PS5 defaults;
/// `docs/COMPATIBILITY.md` is generated from the same file (`tools/compat.py`).
pub mod compat {
    use std::sync::OnceLock;

    use serde::Deserialize;

    use crate::config::Config;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
    #[serde(rename_all = "lowercase")]
    pub enum Status {
        Playable,
        Ingame,
        Menus,
        Boots,
        Nothing,
    }

    impl Status {
        pub fn label(self) -> &'static str {
            match self {
                Status::Playable => "Playable",
                Status::Ingame => "In-game (issues)",
                Status::Menus => "Menus",
                Status::Boots => "Boots / loading",
                Status::Nothing => "Does not start",
            }
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Entry {
        pub title_id: String,
        pub identity: String,
        pub name: String,
        pub versions: Vec<String>,
        pub status: Status,
        pub release: String,
        pub notes: String,
        pub reports: Vec<u32>,
        /// `xcl.toml` keys applied on top of the PS5 defaults
        #[serde(default)]
        pub xcl: toml::Table,
        /// "<name> — <status> (<release>)", made when the database is read
        #[serde(skip)]
        pub label: String,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Database {
        title: Vec<Entry>,
    }

    pub(super) const SOURCE: &str = include_str!("compat.toml");

    pub(super) fn parse(text: &str) -> Result<Vec<Entry>, String> {
        let mut titles = toml::from_str::<Database>(text).map_err(|e| e.to_string())?.title;
        for t in &mut titles {
            t.label = format!("{} \u{2014} {} ({})", t.name, t.status.label(), t.release);
        }
        Ok(titles)
    }

    /// Every known title. The built-in file is checked by the tests; a broken one gives no entries
    /// rather than a library that does not start.
    pub fn all() -> &'static [Entry] {
        static DB: OnceLock<Vec<Entry>> = OnceLock::new();
        DB.get_or_init(|| parse(SOURCE).unwrap_or_default())
    }

    /// The entry for a game, matched by TitleId when the entry has one, else by Identity Name.
    pub fn lookup(title_id: &str, identity: &str) -> Option<&'static Entry> {
        all().iter().find(|e| {
            (!e.title_id.is_empty() && e.title_id.eq_ignore_ascii_case(title_id))
                || e.identity.eq_ignore_ascii_case(identity)
        })
    }

    /// `config` with the entry's `xcl.toml` keys set.
    pub fn apply(entry: &Entry, config: &Config) -> Result<Config, String> {
        if entry.xcl.is_empty() {
            return Ok(config.clone());
        }
        let mut table = toml::Table::try_from(config).map_err(|e| e.to_string())?;
        for (key, value) in &entry.xcl {
            if !table.contains_key(key) {
                return Err(format!("{}: {key} is not an xcl.toml key", entry.name));
            }
            table.insert(key.clone(), value.clone());
        }
        table.try_into().map_err(|e: toml::de::Error| format!("{}: {e}", entry.name))
    }
}

/// Benchmark queue: `<storage root>/bench/request.toml`, written by `tools/ps5_bench.py`
/// (docs/PERFORMANCE.md), lists titles to measure. Each library start takes the next one, runs it
/// with XCL's benchmark mode (`XCL_BENCH_*`) and asks `launcher.c` to start the library again
/// afterwards, until the queue is empty. Results go to `bench/results/<run>/<identity>.json`;
/// `queue.json` there says which title runs and when the run is done.
pub mod bench {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Request {
        run: String,
        seconds: u64,
        warmup: u64,
        #[serde(default)]
        title: Vec<RequestTitle>,
    }

    #[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct RequestTitle {
        identity: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    }

    #[derive(Debug, Default, Deserialize, Serialize)]
    struct Queue {
        run: String,
        /// "running" or "done"
        state: String,
        current: Option<String>,
        remaining: Vec<String>,
    }

    /// One title to measure now: its Identity Name and the environment to start it with.
    #[derive(Debug, PartialEq)]
    pub struct Job {
        pub identity: String,
        pub env: Vec<(String, String)>,
        pub out: PathBuf,
    }

    fn valid_run(run: &str) -> bool {
        run.len() == 15 && run.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'-' } else { b.is_ascii_digit() })
    }

    fn valid_identity(identity: &str) -> bool {
        (2..=100).contains(&identity.len())
            && identity.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            && identity.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
    }

    fn valid_env(name: &str, value: &str) -> bool {
        (1..=64).contains(&name.len())
            && name.bytes().next().is_some_and(|b| b.is_ascii_uppercase())
            && name.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            && !name.starts_with("XCL_BENCH")
            && !value.contains(['\0', '\n', '\r'])
    }

    fn check(request: &Request) -> Result<(), String> {
        if !valid_run(&request.run) {
            return Err(format!("run {:?} is not YYYYMMDD-HHMMSS", request.run));
        }
        if !(5..=1800).contains(&request.seconds) || request.warmup > 600 {
            return Err("seconds must be 5..1800 and warmup at most 600".into());
        }
        for t in &request.title {
            if !valid_identity(&t.identity) {
                return Err(format!("invalid Identity Name {:?}", t.identity));
            }
            if let Some((k, _)) = t.env.iter().find(|(k, v)| !valid_env(k, v)) {
                return Err(format!("environment variable {k:?} is not allowed"));
            }
        }
        Ok(())
    }

    fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text)?;
        if fs::rename(&tmp, path).is_err() {
            // PS5: replacing a file another UID owns can fail; remove it first
            let _ = fs::remove_file(path);
            fs::rename(&tmp, path)?;
        }
        Ok(())
    }

    fn result_text(identity: &str, status: &str) -> String {
        format!("{{\n  \"version\": 1,\n  \"identity\": \"{identity}\",\n  \"status\": \"{status}\"\n}}\n")
    }

    /// Record a title of the current run that produced no measurement (`not-found`, `unsupported`, ...).
    pub fn record_failure(job: &Job, status: &str) {
        let _ = write_atomic(&job.out, &result_text(&job.identity, status));
    }

    /// The next title to measure from the queue in `root` (the storage root), or `None` when there is
    /// no queue or it just ended. The title is taken off `request.toml` before it runs, so a title
    /// that hangs or crashes the console is not started again; the next start records it as
    /// `crashed` when it left no result.
    pub fn next_job(root: &Path) -> Option<Job> {
        let dir = root.join("bench");
        let request_path = dir.join("request.toml");
        let text = fs::read_to_string(&request_path).ok()?;
        let mut request =
            match toml::from_str::<Request>(&text).map_err(|e| e.to_string()).and_then(|r| check(&r).map(|_| r)) {
                Ok(r) => r,
                Err(e) => {
                    crate::xlog_warn!("bench: ignoring {}: {e}", request_path.display());
                    let _ = fs::write(dir.join("request.error.txt"), format!("{e}\n"));
                    let _ = fs::rename(&request_path, dir.join("request.invalid.toml"));
                    return None;
                }
            };
        let results = dir.join("results").join(&request.run);
        fs::create_dir_all(&results).ok()?;
        let queue_path = results.join("queue.json");
        let previous: Queue =
            fs::read_to_string(&queue_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        if let Some(current) = previous.current.filter(|c| valid_identity(c)) {
            let out = results.join(format!("{current}.json"));
            if !out.exists() {
                let _ = write_atomic(&out, &result_text(&current, "crashed"));
            }
        }
        if request.title.is_empty() {
            let queue = Queue { run: request.run.clone(), state: "done".into(), current: None, remaining: Vec::new() };
            let _ = write_atomic(&queue_path, &serde_json::to_string_pretty(&queue).unwrap_or_default());
            let _ = fs::remove_file(&request_path);
            return None;
        }
        let title = request.title.remove(0);
        // persist the shorter queue first: whatever this title does, it is not started twice
        write_atomic(&request_path, &toml::to_string(&request).ok()?).ok()?;
        let queue = Queue {
            run: request.run.clone(),
            state: "running".into(),
            current: Some(title.identity.clone()),
            remaining: request.title.iter().map(|t| t.identity.clone()).collect(),
        };
        let _ = write_atomic(&queue_path, &serde_json::to_string_pretty(&queue).unwrap_or_default());
        let out = results.join(format!("{}.json", title.identity));
        let mut env = vec![
            ("XCL_BENCH_SECONDS".to_owned(), request.seconds.to_string()),
            ("XCL_BENCH_WARMUP".to_owned(), request.warmup.to_string()),
            ("XCL_BENCH_OUT".to_owned(), out.to_string_lossy().into_owned()),
            ("XCL_BENCH_ID".to_owned(), title.identity.clone()),
        ];
        env.extend(title.env);
        Some(Job { identity: title.identity, env, out })
    }
}

/// The library's label for a game the compatibility database knows ("Name — Status (release)").
pub fn lookup_tested_preset_label(title_id: &str, identity: &str) -> Option<&'static str> {
    compat::lookup(title_id, identity).map(|e| e.label.as_str())
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
    if let Some(entry) = compat::lookup(title_id, identity) {
        match compat::apply(entry, &config) {
            Ok(tested) => config = tested,
            Err(e) => crate::xlog_warn!("compat: {e}"),
        }
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

/// Whether the compatibility database has `xcl.toml` settings for this game.
pub fn has_tested_preset(title_id: &str, identity: &str) -> bool {
    compat::lookup(title_id, identity).is_some_and(|e| !e.xcl.is_empty())
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
    // Standard 32-bit and 64-bit Windows PC/GOG games hand off directly to prospero-win
    // (`wine-wow64` pe32/pe64) and do not require an Xbox 64-bit PE or `xcl.toml`.
    if matches!(crate::library_gog::detect_pe_kind(&game.executable), crate::library_gog::PeLaunchKind::PcNative { .. })
    {
        if !game.executable.is_file() {
            return Err(format!("executable missing: {}", game.executable.display()));
        }
        return Ok(());
    }
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
    pub pc_input_modes: BTreeMap<String, String>,
    pub pc_executables: BTreeMap<String, String>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pc_input_modes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pc_executables: BTreeMap<String, String>,
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
            pc_input_modes: BTreeMap::new(),
            pc_executables: BTreeMap::new(),
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

    pub fn pc_input_mode(&self, identity: &str) -> PcInputMode {
        self.pc_input_modes
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(identity))
            .map(|(_, v)| PcInputMode::parse(v))
            .unwrap_or(PcInputMode::Auto)
    }

    pub fn set_pc_input_mode(&mut self, identity: &str, mode: PcInputMode) {
        let existing: Vec<String> =
            self.pc_input_modes.keys().filter(|k| k.eq_ignore_ascii_case(identity)).cloned().collect();
        for key in existing {
            self.pc_input_modes.remove(&key);
        }
        if mode != PcInputMode::Auto {
            self.pc_input_modes.insert(identity.to_owned(), mode.code().to_owned());
        }
    }

    pub fn pc_executable(&self, identity: &str) -> Option<&str> {
        self.pc_executables.iter().find(|(k, _)| k.eq_ignore_ascii_case(identity)).map(|(_, v)| v.as_str())
    }

    pub fn set_pc_executable(&mut self, identity: &str, rel: Option<&str>) {
        let existing: Vec<String> =
            self.pc_executables.keys().filter(|k| k.eq_ignore_ascii_case(identity)).cloned().collect();
        for key in existing {
            self.pc_executables.remove(&key);
        }
        if let Some(rel) = rel.map(str::trim).filter(|s| !s.is_empty()) {
            self.pc_executables.insert(identity.to_owned(), rel.to_owned());
        }
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

/// vkd3d-proton's cache folder for titles run by this process (`VKD3D_SHADER_CACHE_PATH` from the
/// runtime profile).
fn vkd3d_cache_dir() -> Option<PathBuf> {
    let vk = std::env::var("VKD3D_SHADER_CACHE_PATH").ok()?;
    let trimmed = vk.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

/// The shader cache folders. All of them are shared by every title: XCL's translated shaders and
/// DXIL (`dxc::cache_dir`), vkd3d-proton's pipeline cache, and on the PS5 RADV's disk cache, which
/// the runner points at `<storage root>/cache/radv` (`MESA_SHADER_CACHE_DIR`). There is no per-title
/// part to count or clear. (`data_root/<title>/cache` holds XCL's TLS loader DLLs, not shaders.)
fn shader_cache_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push_unique = |p: PathBuf| {
        if !dirs.iter().any(|d| d == &p) {
            dirs.push(p);
        }
    };
    push_unique(crate::gpu::recomp::dxc::cache_dir());
    if let Some(vk) = vkd3d_cache_dir() {
        push_unique(vk);
    } else if Path::new(LIBRARY_VKD3D_CACHE).is_dir() {
        push_unique(PathBuf::from(LIBRARY_VKD3D_CACHE));
    }
    // PC / GOG games handed off to prospero-win keep their own vkd3d-proton cache
    if Path::new(PC_GAME_VKD3D_CACHE).is_dir() {
        push_unique(PathBuf::from(PC_GAME_VKD3D_CACHE));
    }
    let radv = Path::new(PS5_STORAGE_ROOT).join("cache").join("radv");
    if radv.is_dir() {
        push_unique(radv);
    }
    dirs
}

/// The library profile's vkd3d-proton cache folder (`configs/trinityxsx-library.profile`).
const LIBRARY_VKD3D_CACHE: &str = "C:\\XCL\\cache\\trinityxsx-library";
/// The vkd3d-proton cache folder of PC games handed off to prospero-win (`library_gog.rs`).
const PC_GAME_VKD3D_CACHE: &str = "C:\\XCL\\cache\\pc-game";

/// Create the vkd3d-proton cache folders. vkd3d-proton writes its cache into
/// `VKD3D_SHADER_CACHE_PATH` but never creates that folder: when it is missing, opening the cache
/// file fails ("Failed to open stream archive write file exclusively") and every launch compiles
/// all pipelines again. Called when the library starts, before any title runs in this process.
pub fn ensure_shader_cache_dirs() -> Vec<PathBuf> {
    let mut wanted: Vec<PathBuf> = vkd3d_cache_dir().into_iter().collect();
    // The PC-game handoff runs in another process with its own profile; create its folder too,
    // but only inside the PS5 runtime's C:\XCL (not on a development PC).
    if Path::new("C:\\XCL").is_dir() {
        wanted.push(PathBuf::from(PC_GAME_VKD3D_CACHE));
    }
    create_missing_dirs(&wanted)
}

/// Create each folder that does not exist yet; returns the ones created.
fn create_missing_dirs(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut created = Vec::new();
    for dir in dirs {
        if !dir.is_dir() && fs::create_dir_all(dir).is_ok() {
            created.push(dir.clone());
        }
    }
    created
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
            // file_type comes with the directory listing (no open per entry under Wine) and does not
            // follow links, so a link inside a cache folder is never walked into
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                stack.push(entry.path());
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
        // never recurse through a link: only what really is inside the cache folder is deleted
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
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

/// Total file count and size of the shader caches (all titles: see `shader_cache_dirs`).
pub fn shader_cache_summary() -> (usize, u64) {
    let mut files = 0usize;
    let mut bytes = 0u64;
    for dir in shader_cache_dirs() {
        let (f, b) = count_dir_files(&dir);
        files += f;
        bytes = bytes.saturating_add(b);
    }
    (files, bytes)
}

/// Remove the cached shaders of every title without touching saves, profiles, or `xcl.toml`.
pub fn clear_shader_cache() -> (usize, u64) {
    let mut removed = 0usize;
    let mut freed = 0u64;
    for dir in shader_cache_dirs() {
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
    read_profile_value(profile_path, "display", key)
}

/// Read a key from a section of a prospero-win `.profile` file.
fn read_profile_value(profile_path: &Path, section: &str, key: &str) -> Option<String> {
    let text = fs::read_to_string(profile_path).ok()?;
    let mut in_display = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_display = line[1..line.len() - 1].trim().eq_ignore_ascii_case(section);
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

/// Atomically update `key = value` inside `[display]` of a prospero-win `.profile` file.
fn save_display_profile_value(profile_path: &Path, key: &str, value: &str) -> io::Result<()> {
    save_profile_value(profile_path, "display", key, value)
}

/// Atomically update `key = value` inside a section of a prospero-win `.profile` file,
/// preserving all other sections, keys, and comments, and never emitting duplicate keys.
fn save_profile_value(profile_path: &Path, section: &str, key: &str, value: &str) -> io::Result<()> {
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
            in_display = trimmed[1..trimmed.len() - 1].trim().eq_ignore_ascii_case(section);
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
            out.push(format!("[{section}]"));
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

/// How a PC's keyboard and mouse reach the game (`[input] remote` in the runtime profile). The PS5 runner
/// reads it again every two seconds, so a change applies without a restart. A USB keyboard and mouse plugged
/// into the console always work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RemoteInput {
    /// No listening port: only what is plugged into the PS5.
    #[default]
    Off,
    /// `txin-sender.exe` on the PC sends UDP datagrams (port 9396).
    Sender,
    /// The PS5 serves a page on port 8361; the PIN is in the Touchpad menu.
    Browser,
}

impl RemoteInput {
    pub fn code(self) -> &'static str {
        match self {
            RemoteInput::Off => "off",
            RemoteInput::Sender => "sender",
            RemoteInput::Browser => "browser",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sender" => RemoteInput::Sender,
            "browser" => RemoteInput::Browser,
            _ => RemoteInput::Off,
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            RemoteInput::Off => "Off (USB on the PS5 only)",
            RemoteInput::Sender => "txin-sender program",
            RemoteInput::Browser => "Web browser",
        }
    }

    pub fn next(self) -> Self {
        match self {
            RemoteInput::Off => RemoteInput::Browser,
            RemoteInput::Browser => RemoteInput::Sender,
            RemoteInput::Sender => RemoteInput::Off,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            RemoteInput::Off => RemoteInput::Sender,
            RemoteInput::Sender => RemoteInput::Browser,
            RemoteInput::Browser => RemoteInput::Off,
        }
    }
}

/// Read `[input] remote` from a prospero-win `.profile` file (defaults to off).
pub fn read_remote_input_setting(profile_path: &Path) -> RemoteInput {
    read_profile_value(profile_path, "input", "remote").map(|v| RemoteInput::parse(&v)).unwrap_or_default()
}

/// Update `[input] remote = off | sender | browser` inside a prospero-win `.profile` file.
pub fn save_remote_input_setting(profile_path: &Path, mode: RemoteInput) -> io::Result<()> {
    save_profile_value(profile_path, "input", "remote", mode.code())
}

/// What the PS5 runner says it opened for the PC keyboard and mouse (`<storage>/netinput.txt`): the mode in
/// use and, for the browser, the page's address and PIN.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoteInputStatus {
    pub mode: String,
    pub url: String,
    pub pin: String,
    pub address: String,
}

pub fn parse_remote_input_status(text: &str) -> RemoteInputStatus {
    let mut status = RemoteInputStatus::default();
    for line in text.lines() {
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim().to_owned();
            match k.trim() {
                "mode" => status.mode = v,
                "url" => status.url = v,
                "pin" => status.pin = v,
                "address" => status.address = v,
                _ => {}
            }
        }
    }
    status
}

pub fn read_remote_input_status() -> Option<RemoteInputStatus> {
    let text = fs::read_to_string(Path::new(PS5_STORAGE_ROOT).join("netinput.txt")).ok()?;
    Some(parse_remote_input_status(&text))
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

/// Resolve the effective executable path for `game`, honoring any user-selected relative `.exe`
/// override in `ui` as long as it remains safely inside `game.root_dir` and exists on disk.
pub fn resolve_game_executable(game: &Game, ui: &UiSettings) -> PathBuf {
    if let Some(rel) = ui.pc_executable(&game.identity) {
        let clean = rel.replace('/', "\\");
        let safe = !clean.is_empty()
            && !Path::new(&clean).is_absolute()
            && Path::new(&clean).components().all(|c| matches!(c, Component::Normal(_)));
        if safe {
            let candidate = game.root_dir.join(&clean);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    game.executable.clone()
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
        pc_input_modes: parsed.pc_input_modes,
        pc_executables: parsed.pc_executables,
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
        pc_input_modes: settings.pc_input_modes.clone(),
        pc_executables: settings.pc_executables.clone(),
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
            root_dir: dir.clone(),
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

    // Copy of configs/trinityxsx-library.profile: a comment, four sections, and repeated `env` keys in [debug].
    const SAMPLE_PROFILE: &str = concat!(
        "; TrinityXSX catalog inside the existing PS5 Wine runtime.\n",
        "[application]\n",
        "id = trinityxsx-library\n",
        "name = TrinityXSX\n",
        "executable = C:\\XCL\\xcl-library.exe\n",
        "working_directory = C:\\XCL\n",
        "arguments = library C:\\XCL\\games\n",
        "dll_overrides = d3d12,d3d12core,dxgi,d3d11=n\n",
        "prefix = default\n",
        "runtime = wine-wow64\n",
        "architecture = pe64\n",
        "graphics = dxvk\n",
        "\n",
        "[display]\n",
        "desktop = 1920x1080\n",
        "\n",
        "[input]\n",
        "mode = xinput\n",
        "\n",
        "[debug]\n",
        "winedebug = err+all\n",
        "env = VKD3D_SHADER_CACHE_PATH=C:/XCL/cache/trinityxsx-library\n",
        "env = VKD3D_DEBUG=warn\n",
        "env = VKD3D_LOG_FILE=C:/XCL/vkd3d-trinityxsx-library.log\n",
        "env = DXVK_LOG_PATH=C:/XCL\n",
    );

    #[test]
    fn missing_shader_cache_folders_are_created_and_existing_ones_left_alone() {
        // vkd3d-proton does not create VKD3D_SHADER_CACHE_PATH itself; the library must.
        let root = std::env::temp_dir().join(format!("txsx_cache_dirs_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let existing = root.join("existing");
        fs::create_dir_all(&existing).unwrap();
        fs::write(existing.join("vkd3d-proton.cache"), b"VKS4").unwrap();
        let missing = root.join("cache").join("trinityxsx-library");
        let created = create_missing_dirs(&[existing.clone(), missing.clone()]);
        assert_eq!(created, vec![missing.clone()]);
        assert!(missing.is_dir());
        assert_eq!(fs::read(existing.join("vkd3d-proton.cache")).unwrap(), b"VKS4");
        assert!(create_missing_dirs(&[existing, missing]).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn show_fps_defaults_to_true_when_file_section_or_key_is_missing() {
        let dir = temp_dir("fps-default");
        assert!(read_show_fps_setting(&dir.join("absent.profile")));

        let profile = dir.join("t.profile");
        let cases = [
            "",
            "[application]\nid = trinityxsx-library\n",
            "[display]\ndesktop = 1920x1080\n",
            // Commented-out keys are not values.
            "[display]\n; show_fps = false\n# show_fps = off\n",
            // The same key in another section does not count.
            "[input]\nshow_fps = false\n[display]\ndesktop = 1920x1080\n",
        ];
        for body in cases {
            fs::write(&profile, body).unwrap();
            assert!(read_show_fps_setting(&profile), "{body:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn show_fps_reads_false_zero_off_case_insensitively_and_anything_else_as_true() {
        let dir = temp_dir("fps-values");
        let profile = dir.join("t.profile");
        for v in ["false", "FALSE", "False", "0", "off", "OFF", "Off"] {
            fs::write(&profile, format!("[display]\nshow_fps = {v}\n")).unwrap();
            assert!(!read_show_fps_setting(&profile), "{v:?}");
        }
        // Only false/0/off turn the HUD off: even "no" or an empty value keeps it on.
        for v in ["true", "TRUE", "1", "on", "yes", "no", "2", ""] {
            fs::write(&profile, format!("[display]\nshow_fps = {v}\n")).unwrap();
            assert!(read_show_fps_setting(&profile), "{v:?}");
        }
        // Section name, key name and surrounding whitespace are matched loosely.
        fs::write(&profile, "[ Display ]\n  SHOW_FPS   =   Off  \n").unwrap();
        assert!(!read_show_fps_setting(&profile));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_show_fps_round_trips_and_creates_missing_file_and_parent_dirs() {
        let dir = temp_dir("fps-roundtrip");
        let profile = dir.join("profiles").join("trinityxsx-library.profile");

        save_show_fps_setting(&profile, false).unwrap();
        assert!(!read_show_fps_setting(&profile));
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[display]\nshow_fps = false\n");

        save_show_fps_setting(&profile, true).unwrap();
        assert!(read_show_fps_setting(&profile));
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[display]\nshow_fps = true\n");

        save_show_fps_setting(&profile, false).unwrap();
        assert!(!read_show_fps_setting(&profile));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn framegen_defaults_to_off_and_round_trips_supported_factors() {
        let dir = temp_dir("framegen-roundtrip");
        let profile = dir.join("t.profile");
        assert_eq!(read_framegen_setting(&profile), 0);

        for factor in [2u8, 3, 4] {
            save_framegen_setting(&profile, factor).unwrap();
            assert_eq!(read_framegen_setting(&profile), factor);
            assert_eq!(fs::read_to_string(&profile).unwrap(), format!("[display]\nframegen = {factor}\n"));
        }
        // Any factor outside 2..=4 is stored as `off` and reads back as 0.
        for factor in [0u8, 1, 5, 255] {
            save_framegen_setting(&profile, 4).unwrap();
            save_framegen_setting(&profile, factor).unwrap();
            assert_eq!(read_framegen_setting(&profile), 0, "{factor}");
            assert_eq!(fs::read_to_string(&profile).unwrap(), "[display]\nframegen = off\n");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn framegen_reads_only_known_factors_from_the_display_section() {
        let dir = temp_dir("framegen-values");
        let profile = dir.join("t.profile");
        let cases: [(&str, u8); 9] = [
            ("[display]\nframegen = 3\n", 3),
            ("[DISPLAY]\nFrameGen=4\n", 4),
            ("[display]\nframegen = off\n", 0),
            ("[display]\nframegen = 1\n", 0),
            ("[display]\nframegen = 8\n", 0),
            ("[display]\nframegen = x2\n", 0),
            ("[display]\nframegen =\n", 0),
            ("[application]\nframegen = 2\n", 0),
            ("[display]\n; framegen = 2\n", 0),
        ];
        for (body, expected) in cases {
            fs::write(&profile, body).unwrap();
            assert_eq!(read_framegen_setting(&profile), expected, "{body:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_display_keys_preserves_the_rest_of_a_realistic_profile() {
        let dir = temp_dir("profile-preserve");
        let profile = dir.join("trinityxsx-library.profile");
        fs::write(&profile, SAMPLE_PROFILE).unwrap();

        save_framegen_setting(&profile, 3).unwrap();
        assert_eq!(
            fs::read_to_string(&profile).unwrap(),
            SAMPLE_PROFILE.replace("desktop = 1920x1080\n", "desktop = 1920x1080\nframegen = 3\n")
        );

        save_show_fps_setting(&profile, false).unwrap();
        save_framegen_setting(&profile, 4).unwrap();
        // New keys go after the last key of [display]; an existing key is rewritten in place.
        assert_eq!(
            fs::read_to_string(&profile).unwrap(),
            SAMPLE_PROFILE.replace("desktop = 1920x1080\n", "desktop = 1920x1080\nframegen = 4\nshow_fps = false\n")
        );
        assert_eq!(read_framegen_setting(&profile), 4);
        assert!(!read_show_fps_setting(&profile));
        assert_eq!(read_display_profile_value(&profile, "desktop").as_deref(), Some("1920x1080"));
        // The atomic write (tmp + rename) leaves no temporary file behind.
        assert!(!PathBuf::from(format!("{}.tmp", profile.display())).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_input_defaults_to_off_and_is_saved_in_the_input_section() {
        let dir = temp_dir("profile-remote");
        let profile = dir.join("trinityxsx-library.profile");
        fs::write(&profile, SAMPLE_PROFILE).unwrap();
        assert_eq!(read_remote_input_setting(&profile), RemoteInput::Off);
        assert_eq!(read_remote_input_setting(&dir.join("missing.profile")), RemoteInput::Off);

        save_remote_input_setting(&profile, RemoteInput::Browser).unwrap();
        assert_eq!(
            fs::read_to_string(&profile).unwrap(),
            SAMPLE_PROFILE.replace("mode = xinput\n", "mode = xinput\nremote = browser\n")
        );
        assert_eq!(read_remote_input_setting(&profile), RemoteInput::Browser);
        save_remote_input_setting(&profile, RemoteInput::Sender).unwrap();
        assert_eq!(read_remote_input_setting(&profile), RemoteInput::Sender);
        assert_eq!(fs::read_to_string(&profile).unwrap().matches("remote =").count(), 1);
        // [display] keys are untouched by [input] ones, and the other way round.
        save_framegen_setting(&profile, 2).unwrap();
        assert_eq!(read_remote_input_setting(&profile), RemoteInput::Sender);
        assert_eq!(read_framegen_setting(&profile), 2);

        // A profile without [input] gets one.
        let bare = dir.join("bare.profile");
        fs::write(&bare, "[display]\nframegen = off\n").unwrap();
        save_remote_input_setting(&bare, RemoteInput::Off).unwrap();
        assert_eq!(fs::read_to_string(&bare).unwrap(), "[display]\nframegen = off\n\n[input]\nremote = off\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_input_cycles_and_reads_the_runner_status() {
        let mut m = RemoteInput::Off;
        for _ in 0..3 {
            m = m.next();
        }
        assert_eq!(m, RemoteInput::Off);
        assert_eq!(RemoteInput::Off.next().prev(), RemoteInput::Off);
        assert_eq!(RemoteInput::parse(" Browser "), RemoteInput::Browser);
        assert_eq!(RemoteInput::parse("usb"), RemoteInput::Off);
        let s = parse_remote_input_status("mode=browser\nurl=http://192.168.1.170:8361\npin=4821\n");
        assert_eq!((s.mode.as_str(), s.url.as_str(), s.pin.as_str()), ("browser", "http://192.168.1.170:8361", "4821"));
        let s = parse_remote_input_status("mode=sender\naddress=192.168.1.170\nport=9396\n");
        assert_eq!((s.mode.as_str(), s.address.as_str()), ("sender", "192.168.1.170"));
    }

    #[test]
    fn missing_display_section_is_appended_after_a_blank_line() {
        let dir = temp_dir("profile-no-display");
        let profile = dir.join("t.profile");

        let body = "; comment\n[application]\nid = trinityxsx-library\n\n[input]\nmode = xinput\n";
        fs::write(&profile, body).unwrap();
        save_show_fps_setting(&profile, false).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), format!("{body}\n[display]\nshow_fps = false\n"));
        assert!(!read_show_fps_setting(&profile));

        // No second blank line when the file already ends with one.
        fs::write(&profile, "[input]\nmode = xinput\n\n").unwrap();
        save_framegen_setting(&profile, 2).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[input]\nmode = xinput\n\n[display]\nframegen = 2\n");
        assert_eq!(read_framegen_setting(&profile), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_named_key_in_another_section_is_left_untouched() {
        let dir = temp_dir("profile-other-section");
        let profile = dir.join("t.profile");
        fs::write(&profile, "[debug]\nshow_fps = off\n").unwrap();
        assert!(read_show_fps_setting(&profile));

        save_show_fps_setting(&profile, true).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[debug]\nshow_fps = off\n\n[display]\nshow_fps = true\n");
        assert!(read_show_fps_setting(&profile));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_display_key_goes_after_last_key_before_trailing_comments_and_blanks() {
        let dir = temp_dir("profile-insert-pos");
        let profile = dir.join("t.profile");
        let cases = [
            (
                "[display]\ndesktop = 1920x1080\n; note\n\n[input]\nmode = xinput\n",
                "[display]\ndesktop = 1920x1080\nframegen = 2\n; note\n\n[input]\nmode = xinput\n",
            ),
            // Empty [display] directly followed by another header.
            ("[display]\n[input]\nmode = xinput\n", "[display]\nframegen = 2\n[input]\nmode = xinput\n"),
            // [display] is the last section of the file.
            (
                "[application]\nid = a\n\n[display]\ndesktop = 1920x1080\n",
                "[application]\nid = a\n\n[display]\ndesktop = 1920x1080\nframegen = 2\n",
            ),
        ];
        for (before, after) in cases {
            fs::write(&profile, before).unwrap();
            save_framegen_setting(&profile, 2).unwrap();
            assert_eq!(fs::read_to_string(&profile).unwrap(), after, "{before:?}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_rewrites_key_case_insensitively_and_drops_duplicates() {
        let dir = temp_dir("profile-dedupe");
        let profile = dir.join("t.profile");
        fs::write(&profile, "[Display]\nFrameGen = 2\nframegen = 3\nshow_fps = off\nshow_fps = on\n").unwrap();
        // Reading takes the first occurrence.
        assert_eq!(read_framegen_setting(&profile), 2);
        assert!(!read_show_fps_setting(&profile));

        save_framegen_setting(&profile, 4).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[Display]\nframegen = 4\nshow_fps = off\nshow_fps = on\n");

        save_show_fps_setting(&profile, true).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), "[Display]\nframegen = 4\nshow_fps = true\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_keeps_crlf_line_endings() {
        let dir = temp_dir("profile-crlf");
        let profile = dir.join("t.profile");
        let body = "; c\r\n[application]\r\nid = a\r\n\r\n[display]\r\ndesktop = 1920x1080\r\n";
        fs::write(&profile, body).unwrap();

        save_framegen_setting(&profile, 3).unwrap();
        assert_eq!(fs::read_to_string(&profile).unwrap(), format!("{body}framegen = 3\r\n"));
        assert_eq!(read_framegen_setting(&profile), 3);
        // Values are read without a stray '\r'.
        assert_eq!(read_display_profile_value(&profile, "desktop").as_deref(), Some("1920x1080"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn format_storage_line_shows_free_gib_per_mount_joined_by_middle_dots() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(format_storage_line(&[]), "");

        let one = StorageSpace { label: "C:".into(), free_bytes: 5 * GIB + GIB / 2, total_bytes: 100 * GIB };
        assert_eq!(format_storage_line(&[one]), "C: 5.5 GB free");

        let items = vec![
            // 1 TB (decimal) is shown in binary units with one decimal place.
            StorageSpace { label: "/mnt/ext1".into(), free_bytes: 1_000_000_000_000, total_bytes: 2_000_000_000_000 },
            // Rounded, not truncated.
            StorageSpace { label: "/data".into(), free_bytes: 2 * GIB - 1, total_bytes: 4 * GIB },
            StorageSpace { label: "/mnt/usb0".into(), free_bytes: 0, total_bytes: GIB },
        ];
        assert_eq!(
            format_storage_line(&items),
            "/mnt/ext1 931.3 GB free  \u{b7}  /data 2.0 GB free  \u{b7}  /mnt/usb0 0.0 GB free"
        );
    }

    #[test]
    fn query_disk_space_reports_existing_directory_and_rejects_missing_or_file_paths() {
        let dir = temp_dir("disk-space");
        let (free, total) = query_disk_space(&dir).expect("temp dir is on a mounted volume");
        assert!(total > 0);
        assert!(free <= total);

        assert_eq!(query_disk_space(&dir.join("missing")), None);
        let file = dir.join("f.bin");
        fs::write(&file, b"x").unwrap();
        assert_eq!(query_disk_space(&file), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn storage_space_summary_only_reports_known_mounts_with_consistent_sizes() {
        let known = ["C:", "/mnt/ext1", "/data", "/mnt/usb0"];
        for s in storage_space_summary() {
            assert!(known.contains(&s.label.as_str()), "{}", s.label);
            assert!(s.total_bytes > 0);
            assert!(s.free_bytes <= s.total_bytes);
        }
    }

    #[test]
    fn compat_database_is_valid_and_its_presets_are_real_xcl_toml_keys() {
        let titles = compat::parse(compat::SOURCE).expect("library/compat.toml");
        assert!(!titles.is_empty());
        assert_eq!(compat::all().len(), titles.len());
        let defaults = Config::default();
        for t in &titles {
            compat::apply(t, &defaults).unwrap_or_else(|e| panic!("{e}"));
            assert!(
                t.title_id.is_empty() || (t.title_id.len() == 8 && t.title_id.bytes().all(|b| b.is_ascii_hexdigit()))
            );
        }
    }

    #[test]
    fn compat_tool_only_accepts_keys_that_xcl_toml_has() {
        // tools/compat.py takes these keys from issue reports and drops values equal to the PS5
        // default: each must be an xcl.toml key of that type, with that default
        let defaults = toml::Table::try_from(default_config_for_title("", "")).unwrap();
        let mut count = 0;
        for line in include_str!("compat_keys.txt").lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.splitn(3, ' ');
            let (key, kind, default) = (parts.next().unwrap(), parts.next().unwrap(), parts.next().unwrap());
            let value =
                defaults.get(key).unwrap_or_else(|| panic!("compat_keys.txt allows {key}, not an xcl.toml key"));
            let ok = match kind {
                "str" => value.is_str(),
                "u32" | "u64" => value.is_integer(),
                "bool" => value.is_bool(),
                other => panic!("compat_keys.txt: unknown type {other}"),
            };
            assert!(ok, "compat_keys.txt: {key} is not {kind} in xcl.toml");
            let listed: toml::Table = toml::from_str(&format!("v = {default}")).unwrap();
            assert_eq!(&listed["v"], value, "compat_keys.txt: the PS5 default of {key}");
            count += 1;
        }
        assert!(count >= 10, "{count} keys");
        // the Rust types behind u32 / u64: a preset outside them would not load
        let mut t = defaults.clone();
        t.insert("window_width".into(), toml::Value::Integer(i64::from(u32::MAX) + 1));
        assert!(toml::Value::Table(t).try_into::<Config>().is_err());
    }

    #[test]
    fn compat_presets_reach_new_game_configs_like_before() {
        // the Minecraft preset that used to be hard-coded
        let mc = default_config_for_title("639aa8dd", "");
        assert_eq!((mc.title_memory.as_str(), mc.pls_mb, mc.uma_mb), ("standard", 2048, 2048));
        assert_eq!((mc.device_type.as_str(), mc.title_memory_mb, mc.direct_dxil.as_str()), ("lockhart", 6144, "all"));
        assert_eq!(default_config("Microsoft.MinecraftBeta").pls_mb, 2048);
        assert!(has_tested_preset("", "microsoft.minecraftbeta"));
        assert!(lookup_tested_preset_label("639AA8DD", "").unwrap().starts_with("Minecraft Preview"));
        // known, but with nothing to set
        assert!(lookup_tested_preset_label("", "XCL.TestTitle").is_some());
        assert!(!has_tested_preset("", "XCL.TestTitle"));
        // unknown: the PS5 defaults only
        let other = default_config_for_title("0BADF00D", "Publisher.Unknown");
        assert_eq!(
            (other.pls_mb, other.title_memory.as_str()),
            (Config::default().pls_mb, Config::default().title_memory.as_str())
        );
        assert!(lookup_tested_preset_label("0BADF00D", "Publisher.Unknown").is_none());
    }

    #[test]
    fn compat_rejects_presets_with_unknown_keys_or_wrong_types() {
        let bad_key = "[[title]]\ntitle_id = \"\"\nidentity = \"A.B\"\nname = \"A\"\nversions = []\nstatus = \"menus\"\nrelease = \"v1.0.0\"\nnotes = \"\"\nreports = []\n[title.xcl]\nexe_typo = 1\n";
        let t = compat::parse(bad_key).unwrap();
        assert!(compat::apply(&t[0], &Config::default()).is_err());
        let bad_type = bad_key.replace("exe_typo = 1", "pls_mb = \"lots\"");
        let t = compat::parse(&bad_type).unwrap();
        assert!(compat::apply(&t[0], &Config::default()).is_err());
        assert!(compat::parse(&bad_key.replace("menus", "great")).is_err());
    }

    #[test]
    fn bench_queue_runs_each_title_once_and_records_crashes() {
        let root = std::env::temp_dir().join(format!("txsx_bench_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("bench");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("request.toml"),
            "run = \"20261126-120000\"\nseconds = 30\nwarmup = 5\n\n[[title]]\nidentity = \"A.Game\"\nenv = { AETHER_AUTO_DEMO = \"1\" }\n\n[[title]]\nidentity = \"B.Game\"\n",
        )
        .unwrap();
        let results = dir.join("results").join("20261126-120000");

        let a = bench::next_job(&root).unwrap();
        assert_eq!(a.identity, "A.Game");
        assert_eq!(a.out, results.join("A.Game.json"));
        let env: std::collections::HashMap<_, _> = a.env.iter().cloned().collect();
        assert_eq!(env["XCL_BENCH_SECONDS"], "30");
        assert_eq!(env["XCL_BENCH_WARMUP"], "5");
        assert_eq!(env["XCL_BENCH_ID"], "A.Game");
        assert_eq!(env["AETHER_AUTO_DEMO"], "1");
        // A is off the queue before it runs
        assert!(!fs::read_to_string(dir.join("request.toml")).unwrap().contains("A.Game"));
        assert!(fs::read_to_string(results.join("queue.json")).unwrap().contains("\"running\""));

        // A died without a result: recorded as crashed, B is next
        let b = bench::next_job(&root).unwrap();
        assert_eq!(b.identity, "B.Game");
        assert!(fs::read_to_string(results.join("A.Game.json")).unwrap().contains("\"crashed\""));

        // B wrote its result: the queue ends and the request is gone
        fs::write(&b.out, "{\"status\": \"ok\"}").unwrap();
        assert!(bench::next_job(&root).is_none());
        assert!(fs::read_to_string(results.join("queue.json")).unwrap().contains("\"done\""));
        assert_eq!(fs::read_to_string(&b.out).unwrap(), "{\"status\": \"ok\"}");
        assert!(!dir.join("request.toml").exists());
        assert!(bench::next_job(&root).is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn bench_queue_refuses_unsafe_requests() {
        let root = std::env::temp_dir().join(format!("txsx_bench_bad_{}", std::process::id()));
        let good = "run = \"20261126-120000\"\nseconds = 30\nwarmup = 5\n[[title]]\nidentity = \"A.Game\"\n";
        for bad in [
            good.replace("20261126-120000", "../../x"),
            good.replace("A.Game", "../evil"),
            good.replace("seconds = 30", "seconds = 99999"),
            format!("{good}env = {{ XCL_BENCH_SECONDS = \"1\" }}\n"),
            format!("{good}env = {{ lower = \"1\" }}\n"),
            format!("{good}unknown = 1\n"),
        ] {
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(root.join("bench")).unwrap();
            fs::write(root.join("bench/request.toml"), &bad).unwrap();
            assert!(bench::next_job(&root).is_none(), "{bad}");
            assert!(root.join("bench/request.invalid.toml").exists(), "{bad}");
            assert!(!root.join("bench/results").exists(), "{bad}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn prepare_with_profile_accepts_32bit_pc_executable_without_manifest() {
        let dir = std::env::temp_dir().join(format!("txsx_prepare_pe32_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("game32.exe");
        let mut pe = vec![0u8; 0x200];
        pe[0..2].copy_from_slice(b"MZ");
        pe[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        pe[0x80 + 24..0x80 + 26].copy_from_slice(&0x010Bu16.to_le_bytes());
        fs::write(&exe, &pe).unwrap();
        let game = Game {
            name: "Classic 32-bit PC Game".into(),
            identity: "PC.Classic32".into(),
            version: "1.0.0.0".into(),
            title_id: "12345678".into(),
            executable: exe,
            root_dir: dir.clone(),
            folder: "/mnt/ext1/xboxseries".into(),
            logo: None,
        };
        let profile = Profile { name: "PS5Player".into(), user_id: 1 };
        assert!(prepare_with_profile(&game, &profile).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn ui_settings_persists_pc_input_modes_and_clears_auto() {
        let dir = temp_dir("pc-input-modes");
        let file = dir.join("ui_settings.toml");
        let mut settings = UiSettings::default();
        assert_eq!(settings.pc_input_mode("GOG.1207658930"), PcInputMode::Auto);

        settings.set_pc_input_mode("GOG.1207658930", PcInputMode::MouseKeyboard);
        settings.set_pc_input_mode("PC.Strategy", PcInputMode::Gamepad);
        save_ui_settings(&file, &settings).unwrap();

        let reloaded = load_ui_settings(&file);
        assert_eq!(reloaded.pc_input_mode("gog.1207658930"), PcInputMode::MouseKeyboard);
        assert_eq!(reloaded.pc_input_mode("PC.Strategy"), PcInputMode::Gamepad);

        let mut cleared = reloaded;
        cleared.set_pc_input_mode("gog.1207658930", PcInputMode::Auto);
        assert_eq!(cleared.pc_input_mode("GOG.1207658930"), PcInputMode::Auto);
        assert!(!cleared.pc_input_modes.contains_key("GOG.1207658930"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_game_executable_honors_override_and_rejects_traversal() {
        let dir = temp_dir("pc-exe-override");
        fs::create_dir_all(dir.join("sub")).unwrap();
        let default_exe = dir.join("default.exe");
        let alt_exe = dir.join("sub").join("alt.exe");
        fs::write(&default_exe, b"MZ").unwrap();
        fs::write(&alt_exe, b"MZ").unwrap();

        let game = Game {
            name: "Multi-EXE".into(),
            identity: "PC.MultiExe".into(),
            version: "1.0".into(),
            title_id: "00000001".into(),
            executable: default_exe.clone(),
            root_dir: dir.clone(),
            folder: "/mnt/ext1".into(),
            logo: None,
        };
        let mut ui = UiSettings::default();
        assert_eq!(resolve_game_executable(&game, &ui), default_exe);

        ui.set_pc_executable("PC.MultiExe", Some("sub\\alt.exe"));
        assert_eq!(resolve_game_executable(&game, &ui), alt_exe);

        let file = dir.join("ui_settings.toml");
        save_ui_settings(&file, &ui).unwrap();
        let reloaded = load_ui_settings(&file);
        assert_eq!(resolve_game_executable(&game, &reloaded), alt_exe);

        // Traversal and missing paths safely fall back to default executable.
        let mut bad = reloaded;
        bad.set_pc_executable("PC.MultiExe", Some("..\\outside.exe"));
        assert_eq!(resolve_game_executable(&game, &bad), default_exe);
        bad.set_pc_executable("PC.MultiExe", Some("missing.exe"));
        assert_eq!(resolve_game_executable(&game, &bad), default_exe);
        let _ = fs::remove_dir_all(&dir);
    }
}
