//! TrinityXSX's controller-friendly library, cover preview, per-game xcl.toml editor,
//! folder manager, and Web Companion QR window. Everything closes cleanly before in-process launch.

use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use xcl::config::Config;
use xcl::library::{self, BrowserEntry, Catalog, UiSettings};
use xcl::library_art::{self, DibImage, QrCode};
use xcl::library_gog::{self, GogInstaller, PeLaunchKind};
use xcl::library_i18n::tr;
use xcl::library_web::WebCompanion;

const GAME_ROWS: usize = 6;
const PATH_ROWS: usize = 6;
const BROWSER_ROWS: usize = 7;
const SETTINGS_ROWS: usize = 8;
const SETTINGS_COUNT: usize = 13;

const UP: u16 = 0x0001;
const DOWN: u16 = 0x0002;
const LEFT: u16 = 0x0004;
const RIGHT: u16 = 0x0008;
const START: u16 = 0x0010; // OPTIONS
const L1: u16 = 0x0100; // XINPUT_GAMEPAD_LEFT_SHOULDER
const R1: u16 = 0x0200; // XINPUT_GAMEPAD_RIGHT_SHOULDER
const L2: u16 = 0x0400; // Virtual trigger edge (unused bit in XInput)
const R2: u16 = 0x0800; // Virtual trigger edge (unused bit in XInput)
const CROSS: u16 = 0x1000; // XINPUT_GAMEPAD_A  (DualSense Cross)
const CIRCLE: u16 = 0x2000; // XINPUT_GAMEPAD_B  (DualSense Circle)
const SQUARE: u16 = 0x4000; // XINPUT_GAMEPAD_X  (DualSense Square)
const TRIANGLE: u16 = 0x8000; // XINPUT_GAMEPAD_Y  (DualSense Triangle)

const STICK_THRESHOLD: i16 = 18000;
const REPEAT_DELAY_TICKS: u8 = 14; // ~350 ms at 25 ms timer
const REPEAT_INTERVAL_TICKS: u8 = 4; // ~100 ms
const AUTO_WATCH_TICKS: u16 = 200; // ~5 s at 25 ms timer
const CARD_BG_RGB: (u8, u8, u8) = (24, 37, 44);

type ReadPad = unsafe extern "system" fn(u32, *mut PadState) -> u32;

#[repr(C)]
#[derive(Default)]
struct PadState {
    packet: u32,
    buttons: u16,
    triggers: [u8; 2],
    sticks: [i16; 4],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Library,
    GameSettings,
    AppSettings,
    Installers,
    Paths,
    Browser,
    WebQr,
}

struct InstallJob {
    game_name: String,
    done: Arc<AtomicU64>,
    total: Arc<AtomicU64>,
    started: Instant,
    result: Arc<Mutex<Option<Result<String, String>>>>,
}

fn format_install_eta(done: u64, total: u64, elapsed_secs: f64) -> String {
    if elapsed_secs < 2.0 || done < 4 * 1024 * 1024 || total <= done {
        return "estimating time...".to_owned();
    }
    let bytes_per_sec = done as f64 / elapsed_secs;
    if bytes_per_sec <= 1.0 {
        return "estimating time...".to_owned();
    }
    let mib_s = bytes_per_sec / (1024.0 * 1024.0);
    let rem_secs = ((total - done) as f64 / bytes_per_sec).ceil() as u64;
    let eta = if rem_secs >= 3600 {
        format!("~{}h {:02}m left", rem_secs / 3600, (rem_secs % 3600) / 60)
    } else if rem_secs >= 60 {
        format!("~{}m {:02}s left", rem_secs / 60, rem_secs % 60)
    } else {
        format!("~{rem_secs}s left")
    };
    format!("{mib_s:.1} MiB/s   ·   {eta}")
}

struct Menu {
    paths_file: PathBuf,
    ui_file: PathBuf,
    profile_file: PathBuf,
    ui: UiSettings,
    framegen: u8,
    active_framegen: u8,
    show_fps: bool,
    active_show_fps: bool,
    storage_line: String,
    app_row: usize,
    paths: Vec<String>,
    catalog: Catalog,
    folder_sig: u64,
    watch_ticks: u16,
    screen: Screen,
    selected: usize,
    // Cached cover art thumbnails & preview for currently selected game
    cover_cache: library_art::CoverCache,
    preview_key: Option<PathBuf>,
    preview_pe_kind: PeLaunchKind,
    preview_has_custom_cfg: bool,
    preview_saves: (usize, u64),
    /// Shader cache size, counted once per session: the caches are shared by all titles, and RADV's
    /// can hold many files.
    preview_shader_cache: Option<(usize, u64)>,
    // GOG offline installers state
    installers: Vec<GogInstaller>,
    installer_row: usize,
    delete_installer_after: bool,
    install_job: Option<InstallJob>,
    // Per-game xcl.toml editor state
    settings_cfg: Config,
    settings_row: usize,
    settings_preset: usize,
    // Folder manager & browser state
    path_row: usize,
    browser_path: String,
    browser_entries: Vec<BrowserEntry>,
    browser_row: usize,
    browser_error: String,
    // Web Companion & QR code
    web: Option<WebCompanion>,
    qr: Option<QrCode>,
    // Input & lifecycle
    previous: Option<u16>,
    hold_dir: u16,
    hold_ticks: u8,
    chosen: Option<(String, PathBuf)>,
    status: String,
    read_pad: Option<ReadPad>,
}

fn wrap_up(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else if index == 0 {
        len - 1
    } else {
        index - 1
    }
}

fn wrap_down(index: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (index + 1) % len
    }
}

impl Menu {
    fn scan(&mut self) {
        // the cache may have been cleared from the Web Companion: count it again when shown
        self.preview_shader_cache = None;
        let before = self
            .catalog
            .games
            .get(self.selected)
            .map(|game| game.executable.clone())
            .or_else(library::read_last_launched_game);
        self.paths = library::read_paths(&self.paths_file).unwrap_or_else(|| self.paths.clone());
        self.ui = library::load_ui_settings(&self.ui_file);
        self.framegen = library::read_framegen_setting(&self.profile_file);
        self.show_fps = library::read_show_fps_setting(&self.profile_file);
        self.storage_line = library::format_storage_line(&library::storage_space_summary());
        self.catalog = library::scan_paths(&self.paths);
        self.ui.sort_games(&mut self.catalog.games);
        self.installers = library_gog::scan_gog_installers(&self.paths);
        if !self.installers.is_empty() && self.installer_row >= self.installers.len() {
            self.installer_row = self.installers.len() - 1;
        }
        self.folder_sig = library::folders_signature(&self.paths);
        self.selected =
            before.and_then(|path| self.catalog.games.iter().position(|game| game.executable == path)).unwrap_or(0);
        if !self.paths.is_empty() && self.path_row >= self.paths.len() {
            self.path_row = self.paths.len() - 1;
        }
        self.preview_key = None;
        self.refresh_preview();
        let active = self.catalog.folders.iter().filter(|f| f.available).count();
        println!(
            "TrinityXSX: {} game(s) across {}/{} folder(s)",
            self.catalog.games.len(),
            active,
            self.catalog.folders.len()
        );
    }

    fn resort_keeping_selection(&mut self) {
        let current = self.catalog.games.get(self.selected).map(|g| g.executable.clone());
        self.ui.sort_games(&mut self.catalog.games);
        self.selected =
            current.and_then(|path| self.catalog.games.iter().position(|g| g.executable == path)).unwrap_or(0);
        self.preview_key = None;
        self.refresh_preview();
    }

    fn cache_visible_covers(&mut self) {
        let first = scroll_window(self.selected, self.catalog.games.len(), GAME_ROWS);
        let paths: Vec<PathBuf> =
            self.catalog.games.iter().skip(first).take(GAME_ROWS).filter_map(|g| g.logo.clone()).collect();
        for path in paths {
            self.cover_cache.load(&path, CARD_BG_RGB);
        }
    }

    fn refresh_preview(&mut self) {
        self.cache_visible_covers();
        let Some(game) = self.catalog.games.get(self.selected) else {
            self.preview_key = None;
            self.preview_has_custom_cfg = false;
            self.preview_saves = (0, 0);
            return;
        };
        if self.preview_key.as_ref() == Some(&game.executable) {
            return;
        }
        self.preview_key = Some(game.executable.clone());
        if let Some(path) = &game.logo {
            self.cover_cache.load(path, CARD_BG_RGB);
        }
        let (_, has_cfg) = library::load_game_config(game);
        self.preview_pe_kind = library_gog::detect_pe_kind(&game.executable);
        self.preview_has_custom_cfg = has_cfg;
        self.preview_saves = library::saves_summary(game);
    }

    fn open_game_settings(&mut self) {
        let Some(game) = self.catalog.games.get(self.selected) else { return };
        let (cfg, _) = library::load_game_config(game);
        self.settings_cfg = cfg;
        self.settings_row = 0;
        self.settings_preset = 0;
        if self.preview_shader_cache.is_none() {
            self.preview_shader_cache = Some(library::shader_cache_summary());
        }
        self.status.clear();
        self.screen = Screen::GameSettings;
    }

    fn cycle_setting(&mut self, forward: bool) {
        let step = |idx: usize, len: usize| -> usize {
            if forward {
                (idx + 1) % len
            } else {
                (idx + len - 1) % len
            }
        };
        match self.settings_row {
            0 => {
                // Compatibility preset
                let presets = library::NAMED_PRESETS;
                self.settings_preset = step(self.settings_preset, presets.len());
                if let Some(game) = self.catalog.games.get(self.selected) {
                    let preset = presets[self.settings_preset];
                    self.settings_cfg = library::apply_named_preset(game, &self.settings_cfg, preset.id);
                }
            }
            1 => {
                // Console model
                if self.settings_cfg.device_type.eq_ignore_ascii_case("lockhart") {
                    self.settings_cfg.device_type = "anaconda".into();
                    self.settings_cfg.console_type_raw = 6;
                } else {
                    self.settings_cfg.device_type = "lockhart".into();
                    self.settings_cfg.console_type_raw = 5;
                }
            }
            2 => {
                // Title memory MiB
                const OPTS: [u64; 4] = [4096, 6144, 8192, 0];
                let cur = OPTS.iter().position(|&v| v == self.settings_cfg.title_memory_mb).unwrap_or(1);
                self.settings_cfg.title_memory_mb = OPTS[step(cur, OPTS.len())];
            }
            3 => {
                // Coherent UMA MiB
                const OPTS: [u64; 4] = [512, 1024, 1536, 2048];
                let cur = OPTS.iter().position(|&v| v == self.settings_cfg.coherent_mb).unwrap_or(1);
                self.settings_cfg.coherent_mb = OPTS[step(cur, OPTS.len())];
            }
            4 => {
                // Direct DXIL
                const OPTS: [&str; 3] = ["all", "1", ""];
                let cur = OPTS
                    .iter()
                    .position(|&v| v.eq_ignore_ascii_case(self.settings_cfg.direct_dxil.trim()))
                    .unwrap_or(0);
                self.settings_cfg.direct_dxil = OPTS[step(cur, OPTS.len())].into();
            }
            5 => {
                // Raytracing
                const OPTS: [&str; 4] = ["off", "tier1_0", "tier1_1", "auto"];
                let cur =
                    OPTS.iter().position(|&v| v.eq_ignore_ascii_case(self.settings_cfg.raytracing.trim())).unwrap_or(0);
                self.settings_cfg.raytracing = OPTS[step(cur, OPTS.len())].into();
            }
            6 => {
                // Network
                self.settings_cfg.network = !self.settings_cfg.network;
            }
            7 => {
                // Xbox Live tokens
                self.settings_cfg.xbox_live_tokens = !self.settings_cfg.xbox_live_tokens;
            }
            8 => {
                // Log level
                const OPTS: [&str; 3] = ["info", "debug", "warn"];
                let cur =
                    OPTS.iter().position(|&v| v.eq_ignore_ascii_case(self.settings_cfg.log_level.trim())).unwrap_or(0);
                self.settings_cfg.log_level = OPTS[step(cur, OPTS.len())].into();
            }
            9 => {
                // Diagnostic tracing bundle
                let on = !(self.settings_cfg.activity || self.settings_cfg.live_flush);
                self.settings_cfg.activity = on;
                self.settings_cfg.trace_net = on;
                self.settings_cfg.live_flush = on;
            }
            10 => {
                // Stub nvngx_dlss.dll
                if self.settings_cfg.aliases.get("nvngx_dlss.dll").is_some_and(|v| v == "@stub") {
                    self.settings_cfg.aliases.remove("nvngx_dlss.dll");
                } else {
                    self.settings_cfg.aliases.insert("nvngx_dlss.dll".into(), "@stub".into());
                }
            }
            11 => {
                // vkd3d-proton D3D12 feature level override
                const OPTS: [&str; 3] = ["", "12_0", "12_1"];
                let cur = OPTS
                    .iter()
                    .position(|&v| v.eq_ignore_ascii_case(self.settings_cfg.vkd3d_feature_level.trim()))
                    .unwrap_or(0);
                self.settings_cfg.vkd3d_feature_level = OPTS[step(cur, OPTS.len())].into();
            }
            12 => {
                // Action row: Clear shader cache (all titles share it; xcl.toml is not touched)
                let (files, bytes) = library::clear_shader_cache();
                self.preview_shader_cache = Some(library::shader_cache_summary());
                self.status = format!(
                    "Cleared {files} shader cache file(s) of all games ({:.1} MiB freed)",
                    (bytes as f64) / (1024.0 * 1024.0)
                );
                return;
            }
            _ => {}
        }

        if let Some(game) = self.catalog.games.get(self.selected) {
            match library::save_game_config(game, &self.settings_cfg) {
                Ok(()) => {
                    self.preview_has_custom_cfg = true;
                    self.status = format!("Saved xcl.toml for {}", game.identity);
                }
                Err(e) => self.status = format!("Cannot save xcl.toml: {e}"),
            }
        }
    }

    fn setting_row_info(&self, row: usize) -> (&'static str, String, &'static str) {
        match row {
            0 => {
                let preset = library::NAMED_PRESETS
                    .get(self.settings_preset)
                    .copied()
                    .unwrap_or(library::NAMED_PRESETS[0]);
                let label = if self.settings_preset == 0 {
                    if let Some(game) = self.catalog.games.get(self.selected) {
                        if let Some(known) = library::lookup_tested_preset_label(&game.title_id, &game.identity) {
                            known.to_string()
                        } else {
                            preset.label.into()
                        }
                    } else {
                        preset.label.into()
                    }
                } else {
                    preset.label.into()
                };
                ("Compatibility preset", label, preset.summary)
            }
            1 => (
                "Console model",
                if self.settings_cfg.device_type.eq_ignore_ascii_case("lockhart") {
                    "Xbox Series S (lockhart)".into()
                } else {
                    "Xbox Series X (anaconda)".into()
                },
                "Series S uses less RAM and safer LOD on PS5. Switch to Series X only if a game refuses Series S.",
            ),
            2 => (
                "Title memory budget",
                if self.settings_cfg.title_memory_mb == 0 {
                    "Auto (console default)".into()
                } else {
                    format!("{} MiB", self.settings_cfg.title_memory_mb)
                },
                "6144 MiB is recommended on PS5. Lower to 4096 MiB if a game crashes with out-of-memory errors.",
            ),
            3 => (
                "Coherent UMA window",
                format!("{} MiB", self.settings_cfg.coherent_mb.max(512)),
                "CPU/GPU mapped texture window. Raise to 1536 or 2048 MiB if xcl.log reports coherent window exhaustion.",
            ),
            4 => (
                "Direct DXIL translator",
                match self.settings_cfg.direct_dxil.trim() {
                    "all" => "all (fastest on PS5)".into(),
                    "1" => "1 (partial)".into(),
                    _ => "off (use DXC)".into(),
                },
                "\"all\" skips heavy multi-threaded DXC shader compilation on PS5. Switch to \"off\" if shaders look broken.",
            ),
            5 => (
                "Raytracing (DXR)",
                if self.settings_cfg.raytracing.is_empty() { "off".into() } else { self.settings_cfg.raytracing.clone() },
                "Keep \"off\" while testing a new game; enable tier1_0/tier1_1 only after it runs stably.",
            ),
            6 => (
                "Network access",
                if self.settings_cfg.network { "Online (true)".into() } else { "Offline (false)".into() },
                "Offline blocks external sockets so games boot cleanly without hanging on Xbox Live.",
            ),
            7 => (
                "Xbox Live placeholder tokens",
                if self.settings_cfg.xbox_live_tokens { "Enabled (true)".into() } else { "Disabled (false)".into() },
                "Keep Disabled for offline play — placeholder tokens can cause infinite sign-in loops.",
            ),
            8 => (
                "Log verbosity",
                self.settings_cfg.log_level.clone(),
                "Use \"info\" for normal play. Switch to \"debug\" when capturing logs for a GitHub bug report.",
            ),
            9 => (
                "Diagnostic tracing",
                if self.settings_cfg.activity || self.settings_cfg.live_flush {
                    "On (activity + net + live flush)".into()
                } else {
                    "Off (normal play)".into()
                },
                "Flushes xcl.log immediately and records memory/activity every 2s. Helpful when diagnosing hangs.",
            ),
            10 => (
                "Stub nvngx_dlss.dll",
                if self.settings_cfg.aliases.get("nvngx_dlss.dll").is_some_and(|v| v == "@stub") {
                    "On (@stub)".into()
                } else {
                    "Off".into()
                },
                "Replaces NVIDIA DLSS DLL imports with a harmless stub on PS5's AMD RDNA2 GPU.",
            ),
            11 => (
                "D3D12 Feature Level (vkd3d)",
                match self.settings_cfg.vkd3d_feature_level.trim() {
                    "12_0" => "12_0 (force FL 12_0)".into(),
                    "12_1" => "12_1 (force FL 12_1)".into(),
                    _ => "Auto (GPU default)".into(),
                },
                "Sets VKD3D_FEATURE_LEVEL for vkd3d-proton. Try 12_0 if a game fails D3D12CreateDevice on PS5 RADV.",
            ),
            _ => {
                let (files, bytes) = self.preview_shader_cache.unwrap_or((0, 0));
                let value = if files == 0 {
                    tr(self.ui.language, "Empty").into()
                } else {
                    format!("Clear ({files} files · {:.1} MiB)", (bytes as f64) / (1024.0 * 1024.0))
                };
                (
                    tr(self.ui.language, "Clear Shader Cache"),
                    value,
                    "Deletes the compiled shaders of ALL games (XCL, vkd3d-proton, RADV); they rebuild on next launch. Saves and xcl.toml stay. Press X.",
                )
            }
        }
    }

    fn open_browser(&mut self, start_at: Option<&str>) {
        self.browser_path = library::browser_initial_path(start_at);
        self.refresh_browser();
        self.screen = Screen::Browser;
    }

    fn refresh_browser(&mut self) {
        self.browser_row = 0;
        self.browser_error.clear();
        match library::list_subfolders(&self.browser_path) {
            Ok(entries) => self.browser_entries = entries,
            Err(error) => {
                self.browser_entries.clear();
                self.browser_error = error;
            }
        }
    }

    fn input(&mut self, buttons: u16) -> bool {
        match self.screen {
            Screen::Library => self.input_library(buttons),
            Screen::GameSettings => self.input_game_settings(buttons),
            Screen::AppSettings => self.input_app_settings(buttons),
            Screen::Installers => self.input_installers(buttons),
            Screen::Paths => self.input_paths(buttons),
            Screen::Browser => self.input_browser(buttons),
            Screen::WebQr => self.input_web_qr(buttons),
        }
    }

    fn start_selected_install(&mut self) {
        if self.install_job.is_some() {
            return;
        }
        let Some(inst) = self.installers.get(self.installer_row).cloned() else {
            return;
        };
        let dest_dir = inst.install_parent.join(&inst.slug);
        let done = Arc::new(AtomicU64::new(0));
        let total = Arc::new(AtomicU64::new(inst.total_bytes.max(1)));
        let result = Arc::new(Mutex::new(None));
        let delete_after = self.delete_installer_after;

        let done_bg = Arc::clone(&done);
        let total_bg = Arc::clone(&total);
        let res_bg = Arc::clone(&result);
        let fallback_name = inst.display_name.clone();
        std::thread::spawn(move || {
            let outcome = library_gog::extract_gog_installer_in_process(&inst.setup_exe, &dest_dir, done_bg, total_bg)
                .and_then(|()| library_gog::finalize_gog_game(&dest_dir, &fallback_name))
                .map(|fin| {
                    if delete_after {
                        let _ = std::fs::remove_file(&inst.setup_exe);
                        for bin in &inst.slice_files {
                            let _ = std::fs::remove_file(bin);
                        }
                    }
                    format!(
                        "Installed '{}' [{}] ({})",
                        fin.name,
                        fin.pe_kind.badge(),
                        if fin.has_cover { "cover OK" } else { "no cover" }
                    )
                });
            if let Ok(mut guard) = res_bg.lock() {
                *guard = Some(outcome);
            }
        });

        self.status = format!("Installing {}...", inst.display_name);
        self.install_job =
            Some(InstallJob { game_name: inst.display_name, done, total, started: Instant::now(), result });
    }

    fn poll_install_job(&mut self) -> bool {
        let finished = if let Some(job) = &self.install_job {
            job.result.lock().ok().and_then(|mut g| g.take())
        } else {
            return false;
        };
        if let Some(outcome) = finished {
            self.install_job = None;
            match outcome {
                Ok(msg) => {
                    self.scan();
                    self.status = format!("{msg}  ·  Press Circle to play");
                }
                Err(err) => {
                    self.status = format!("Install failed: {err}");
                }
            }
            return true;
        }
        true // repaint live progress bar while running
    }

    fn input_installers(&mut self, buttons: u16) -> bool {
        if self.install_job.is_some() {
            return false; // ignore input while extraction thread is running
        }
        if buttons & CIRCLE != 0 {
            self.screen = Screen::Library;
            return false;
        }
        if buttons & TRIANGLE != 0 {
            self.installers = library_gog::scan_gog_installers(&self.paths);
            self.status = format!("Found {} GOG installer(s)", self.installers.len());
            return false;
        }
        if buttons & SQUARE != 0 {
            self.delete_installer_after = !self.delete_installer_after;
            self.status =
                format!("Delete installer after install: {}", if self.delete_installer_after { "On" } else { "Off" });
            return false;
        }
        if buttons & UP != 0 {
            self.installer_row = wrap_up(self.installer_row, self.installers.len());
        }
        if buttons & DOWN != 0 {
            self.installer_row = wrap_down(self.installer_row, self.installers.len());
        }
        if buttons & CROSS != 0 && !self.installers.is_empty() {
            self.start_selected_install();
        }
        false
    }

    fn input_app_settings(&mut self, buttons: u16) -> bool {
        if buttons & (CIRCLE | START) != 0 {
            self.screen = Screen::Library;
            return false;
        }
        if buttons & TRIANGLE != 0 {
            match library::request_app_restart() {
                Ok(()) => {
                    self.chosen = None;
                    return true;
                }
                Err(e) => {
                    self.status = format!("Cannot request restart: {e}");
                    return false;
                }
            }
        }
        if buttons & UP != 0 {
            self.app_row = wrap_up(self.app_row, 6);
        }
        if buttons & DOWN != 0 {
            self.app_row = wrap_down(self.app_row, 6);
        }
        if buttons & (CROSS | RIGHT | LEFT) != 0 {
            let forward = (buttons & LEFT) == 0;
            match self.app_row {
                0 => {
                    if buttons & CROSS != 0 {
                        self.installers = library_gog::scan_gog_installers(&self.paths);
                        self.status.clear();
                        self.screen = Screen::Installers;
                    }
                }
                1 => {
                    self.ui.language = if forward { self.ui.language.next() } else { self.ui.language.prev() };
                    match library::save_ui_settings(&self.ui_file, &self.ui) {
                        Ok(()) => {
                            self.status =
                                format!("{}  ·  {}", self.ui.language.display_name(), self.ui.current_profile().name);
                        }
                        Err(e) => self.status = format!("Cannot save ui_settings.toml: {e}"),
                    }
                }
                2 => {
                    let len = self.ui.profiles.len();
                    if len > 0 {
                        self.ui.active_profile = if forward {
                            (self.ui.active_profile + 1) % len
                        } else {
                            (self.ui.active_profile + len - 1) % len
                        };
                    }
                    match library::save_ui_settings(&self.ui_file, &self.ui) {
                        Ok(()) => {
                            self.status =
                                format!("{}  ·  {}", self.ui.language.display_name(), self.ui.current_profile().name);
                        }
                        Err(e) => self.status = format!("Cannot save ui_settings.toml: {e}"),
                    }
                }
                3 => {
                    const OPTS: [u8; 4] = [0, 2, 3, 4];
                    let cur = OPTS.iter().position(|&v| v == self.framegen).unwrap_or(0);
                    let next = if forward { (cur + 1) % OPTS.len() } else { (cur + OPTS.len() - 1) % OPTS.len() };
                    self.framegen = OPTS[next];
                    match library::save_framegen_setting(&self.profile_file, self.framegen) {
                        Ok(()) => {
                            let label = match self.framegen {
                                2 => "2x (30 -> 60 FPS)",
                                3 => "3x (20 -> 60 FPS)",
                                4 => "4x (15 -> 60 FPS)",
                                _ => tr(self.ui.language, "Off"),
                            };
                            self.status = if self.framegen != self.active_framegen {
                                format!(
                                    "{}: {label}  ·  {}",
                                    tr(self.ui.language, "Frame Generation"),
                                    tr(self.ui.language, "Restart TrinityXSX to apply")
                                )
                            } else {
                                format!("{}: {label}", tr(self.ui.language, "Frame Generation"))
                            };
                        }
                        Err(e) => self.status = format!("Cannot save profile: {e}"),
                    }
                }
                4 => {
                    self.show_fps = !self.show_fps;
                    match library::save_show_fps_setting(&self.profile_file, self.show_fps) {
                        Ok(()) => {
                            let label = tr(self.ui.language, if self.show_fps { "On" } else { "Off" });
                            self.status = if self.show_fps != self.active_show_fps {
                                format!(
                                    "{}: {label}  ·  {}",
                                    tr(self.ui.language, "FPS Counter"),
                                    tr(self.ui.language, "Restart TrinityXSX to apply")
                                )
                            } else {
                                format!("{}: {label}", tr(self.ui.language, "FPS Counter"))
                            };
                        }
                        Err(e) => self.status = format!("Cannot save profile: {e}"),
                    }
                }
                _ => {
                    if buttons & CROSS != 0 {
                        match library::request_app_restart() {
                            Ok(()) => {
                                self.chosen = None;
                                return true;
                            }
                            Err(e) => self.status = format!("Cannot request restart: {e}"),
                        }
                    }
                }
            }
        }
        false
    }

    fn input_library(&mut self, buttons: u16) -> bool {
        if buttons & CIRCLE != 0 {
            return true;
        }
        if buttons & SQUARE != 0 {
            self.status.clear();
            if !self.paths.is_empty() && self.path_row >= self.paths.len() {
                self.path_row = self.paths.len() - 1;
            }
            self.screen = Screen::Paths;
            return false;
        }
        if buttons & L1 != 0 {
            self.screen = Screen::WebQr;
            return false;
        }
        if buttons & START != 0 {
            self.status.clear();
            self.screen = Screen::AppSettings;
            return false;
        }
        if buttons & R1 != 0 && !self.catalog.games.is_empty() {
            self.open_game_settings();
            return false;
        }
        if buttons & TRIANGLE != 0 {
            self.scan();
            self.status = if self.catalog.skipped == 0 {
                format!("Rescanned {} folder(s)", self.paths.len())
            } else {
                format!("Rescanned  ·  skipped folders: {}", self.catalog.skipped)
            };
        }
        if buttons & L2 != 0 {
            if let Some(game) = self.catalog.games.get(self.selected) {
                let id = game.identity.clone();
                let name = game.name.clone();
                let fav = self.ui.toggle_favorite(&id);
                let _ = library::save_ui_settings(&self.ui_file, &self.ui);
                self.resort_keeping_selection();
                let verb = tr(self.ui.language, if fav { "Added to favorites" } else { "Removed from favorites" });
                self.status = format!("{verb}: {name}");
            }
        }
        if buttons & (R2 | RIGHT | LEFT) != 0 {
            self.ui.sort_mode = if (buttons & LEFT) != 0 { self.ui.sort_mode.prev() } else { self.ui.sort_mode.next() };
            let _ = library::save_ui_settings(&self.ui_file, &self.ui);
            self.resort_keeping_selection();
            self.status = tr(self.ui.language, self.ui.sort_mode.label_key()).into();
        }
        if buttons & UP != 0 {
            self.selected = wrap_up(self.selected, self.catalog.games.len());
            self.refresh_preview();
        }
        if buttons & DOWN != 0 {
            self.selected = wrap_down(self.selected, self.catalog.games.len());
            self.refresh_preview();
        }
        if buttons & CROSS != 0 {
            if let Some(game) = self.catalog.games.get(self.selected) {
                let exe = game.executable.clone();
                let name = game.name.clone();
                let id = game.identity.clone();
                match library::prepare_with_profile(game, self.ui.current_profile()) {
                    Ok(()) => {
                        self.ui.record_recent(&id);
                        let _ = library::save_ui_settings(&self.ui_file, &self.ui);
                        self.chosen = Some((name, exe));
                        return true;
                    }
                    Err(error) => self.status = format!("Cannot start game: {error}"),
                }
            } else {
                self.open_browser(None);
            }
        }
        false
    }

    fn input_game_settings(&mut self, buttons: u16) -> bool {
        if buttons & CIRCLE != 0 {
            self.screen = Screen::Library;
            self.preview_key = None;
            self.refresh_preview();
            return false;
        }
        if buttons & UP != 0 {
            self.settings_row = wrap_up(self.settings_row, SETTINGS_COUNT);
        }
        if buttons & DOWN != 0 {
            self.settings_row = wrap_down(self.settings_row, SETTINGS_COUNT);
        }
        if buttons & (CROSS | RIGHT) != 0 {
            self.cycle_setting(true);
        }
        if buttons & LEFT != 0 {
            self.cycle_setting(false);
        }
        if buttons & TRIANGLE != 0 {
            if let Some(game) = self.catalog.games.get(self.selected) {
                match library::reset_game_config(game) {
                    Ok(preset) => {
                        self.settings_cfg = preset;
                        self.preview_has_custom_cfg = true;
                        self.status = "Restored recommended PS5 preset".into();
                    }
                    Err(e) => self.status = format!("Cannot reset xcl.toml: {e}"),
                }
            }
        }
        false
    }

    fn input_web_qr(&mut self, buttons: u16) -> bool {
        if buttons & (CIRCLE | L1 | SQUARE) != 0 {
            self.screen = Screen::Library;
        }
        false
    }

    fn input_paths(&mut self, buttons: u16) -> bool {
        if buttons & CIRCLE != 0 {
            self.screen = Screen::Library;
            return false;
        }
        if buttons & UP != 0 {
            self.path_row = wrap_up(self.path_row, self.paths.len());
        }
        if buttons & DOWN != 0 {
            self.path_row = wrap_down(self.path_row, self.paths.len());
        }
        if buttons & CROSS != 0 {
            let selected = self.paths.get(self.path_row).cloned();
            self.open_browser(selected.as_deref());
            return false;
        }
        if buttons & SQUARE != 0 {
            self.open_browser(None);
            return false;
        }
        if buttons & TRIANGLE != 0 && self.path_row < self.paths.len() {
            let mut updated = self.paths.clone();
            let removed = updated.remove(self.path_row);
            match library::write_paths(&self.paths_file, &updated) {
                Ok(()) => {
                    self.paths = updated;
                    if self.paths.is_empty() {
                        self.path_row = 0;
                    } else if self.path_row >= self.paths.len() {
                        self.path_row = self.paths.len() - 1;
                    }
                    self.scan();
                    self.status = format!("Removed {removed} (files on disk untouched)");
                }
                Err(error) => self.status = format!("Cannot save folders: {error}"),
            }
        }
        false
    }

    fn input_browser(&mut self, buttons: u16) -> bool {
        if buttons & SQUARE != 0 {
            self.browser_error.clear();
            self.screen = Screen::Paths;
            return false;
        }
        if buttons & UP != 0 {
            self.browser_row = wrap_up(self.browser_row, self.browser_entries.len());
        }
        if buttons & DOWN != 0 {
            self.browser_row = wrap_down(self.browser_row, self.browser_entries.len());
        }
        if buttons & CROSS != 0 {
            if let Some(entry) = self.browser_entries.get(self.browser_row) {
                self.browser_path = entry.stored_path.clone();
                self.refresh_browser();
            }
            return false;
        }
        if buttons & CIRCLE != 0 {
            if let Some(parent) = library::browser_parent(&self.browser_path) {
                self.browser_path = parent;
                self.refresh_browser();
            } else {
                self.browser_error.clear();
                self.screen = Screen::Paths;
            }
            return false;
        }
        if buttons & TRIANGLE != 0 {
            let Some(normalized) = library::normalize_path(&self.browser_path) else {
                self.browser_error = "Select a specific game folder (not the filesystem root).".into();
                return false;
            };
            let mut updated = self.paths.clone();
            let before_len = updated.len();
            let _ = library::push_unique_path(&mut updated, &normalized);
            match library::write_paths(&self.paths_file, &updated) {
                Ok(()) => {
                    self.paths = updated;
                    if let Some(idx) = self.paths.iter().position(|p| p == &normalized) {
                        self.path_row = idx;
                    }
                    self.browser_error.clear();
                    self.screen = Screen::Paths;
                    self.scan();
                    self.status = if self.paths.len() > before_len {
                        format!("Added folder: {normalized}")
                    } else {
                        format!("Folder already configured: {normalized}")
                    };
                }
                Err(error) => self.browser_error = format!("Cannot save folders: {error}"),
            }
        }
        false
    }

    unsafe fn pad_buttons(&mut self) -> u16 {
        let mut state = PadState::default();
        if !self.read_pad.is_some_and(|read| read(0, &mut state) == 0) {
            return 0;
        }
        let mut buttons = state.buttons & !(L2 | R2);
        if state.triggers[0] > 128 {
            buttons |= L2;
        }
        if state.triggers[1] > 128 {
            buttons |= R2;
        }
        if state.sticks[1] > STICK_THRESHOLD {
            buttons |= UP;
        } else if state.sticks[1] < -STICK_THRESHOLD {
            buttons |= DOWN;
        }
        if state.sticks[0] > STICK_THRESHOLD {
            buttons |= RIGHT;
        } else if state.sticks[0] < -STICK_THRESHOLD {
            buttons |= LEFT;
        }

        let mut pressed = self.previous.map_or(0, |previous| buttons & !previous);
        self.previous = Some(buttons);

        let dir = buttons & (UP | DOWN);
        if dir == UP || dir == DOWN {
            if dir == self.hold_dir {
                self.hold_ticks = self.hold_ticks.saturating_add(1);
                if self.hold_ticks >= REPEAT_DELAY_TICKS
                    && (self.hold_ticks - REPEAT_DELAY_TICKS).is_multiple_of(REPEAT_INTERVAL_TICKS)
                {
                    pressed |= dir;
                }
            } else {
                self.hold_dir = dir;
                self.hold_ticks = 0;
            }
        } else {
            self.hold_dir = 0;
            self.hold_ticks = 0;
        }
        pressed
    }
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    r | (g << 8) | (b << 16)
}

unsafe fn fill(dc: HDC, rect: RECT, color: u32) {
    let brush = CreateSolidBrush(color);
    FillRect(dc, &rect, brush);
    DeleteObject(brush);
}

unsafe fn text_aligned(dc: HDC, value: &str, mut rect: RECT, size: i32, weight: i32, color: u32, flags: u32) {
    if value.is_empty() {
        return;
    }
    let font = CreateFontW(-size, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, super::wide("Segoe UI").as_ptr());
    let old = SelectObject(dc, font);
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, color);
    let text = super::wide(value);
    DrawTextW(dc, text.as_ptr(), (text.len() - 1) as i32, &mut rect, flags | DT_NOPREFIX);
    SelectObject(dc, old);
    DeleteObject(font);
}

unsafe fn text(dc: HDC, value: &str, rect: RECT, size: i32, weight: i32, color: u32) {
    text_aligned(dc, value, rect, size, weight, color, DT_SINGLELINE | DT_END_ELLIPSIS);
}

#[derive(Clone, Copy)]
enum BtnIcon {
    Cross,
    Circle,
    Square,
    Triangle,
    Pill(&'static str),
}

unsafe fn measure_text_w(dc: HDC, value: &str, size: i32, weight: i32) -> i32 {
    if value.is_empty() {
        return 0;
    }
    let font = CreateFontW(-size, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, super::wide("Segoe UI").as_ptr());
    let old = SelectObject(dc, font);
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    let wide = super::wide(value);
    DrawTextW(dc, wide.as_ptr(), (wide.len() - 1) as i32, &mut r, DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX);
    SelectObject(dc, old);
    DeleteObject(font);
    r.right - r.left
}

unsafe fn draw_btn_icon(dc: HDC, icon: BtnIcon, x: i32, y: i32, h: i32) -> i32 {
    let bg = rgb(30, 44, 54);
    let rim = rgb(104, 132, 148);
    let stroke = (h / 10).max(2);

    match icon {
        BtnIcon::Cross | BtnIcon::Circle | BtnIcon::Square | BtnIcon::Triangle => {
            let fill_br = CreateSolidBrush(bg);
            let rim_pen = CreatePen(PS_SOLID as i32, 1, rim);
            let old_br = SelectObject(dc, fill_br);
            let old_pen = SelectObject(dc, rim_pen);
            Ellipse(dc, x, y, x + h, y + h);
            SelectObject(dc, old_br);
            SelectObject(dc, old_pen);
            DeleteObject(fill_br);
            DeleteObject(rim_pen);

            let pad = (h * 27) / 100;
            let (color, kind) = match icon {
                BtnIcon::Cross => (rgb(112, 174, 255), 0u8),
                BtnIcon::Circle => (rgb(255, 110, 118), 1u8),
                BtnIcon::Square => (rgb(232, 130, 218), 2u8),
                _ => (rgb(82, 224, 164), 3u8),
            };
            let sym_pen = CreatePen(PS_SOLID as i32, stroke, color);
            let old_pen = SelectObject(dc, sym_pen);
            let old_br = SelectObject(dc, GetStockObject(NULL_BRUSH as i32));
            let l = x + pad;
            let t = y + pad;
            let r = x + h - pad;
            let b = y + h - pad;
            match kind {
                0 => {
                    MoveToEx(dc, l, t, null_mut());
                    LineTo(dc, r, b);
                    MoveToEx(dc, r, t, null_mut());
                    LineTo(dc, l, b);
                }
                1 => {
                    Ellipse(dc, l, t, r, b);
                }
                2 => {
                    Rectangle(dc, l, t, r, b);
                }
                _ => {
                    let cx = x + h / 2;
                    MoveToEx(dc, cx, t, null_mut());
                    LineTo(dc, r, b);
                    LineTo(dc, l, b);
                    LineTo(dc, cx, t);
                }
            }
            SelectObject(dc, old_br);
            SelectObject(dc, old_pen);
            DeleteObject(sym_pen);
            h
        }
        BtnIcon::Pill(lbl) => {
            let font_sz = (h * 56) / 100;
            let tw = measure_text_w(dc, lbl, font_sz, 700);
            let pad_x = (h * 34) / 100;
            let w = (tw + pad_x * 2).max(h + (h / 3));
            let fill_br = CreateSolidBrush(bg);
            let rim_pen = CreatePen(PS_SOLID as i32, 1, rim);
            let old_br = SelectObject(dc, fill_br);
            let old_pen = SelectObject(dc, rim_pen);
            let rad = (h * 45) / 100;
            RoundRect(dc, x, y, x + w, y + h, rad, rad);
            SelectObject(dc, old_br);
            SelectObject(dc, old_pen);
            DeleteObject(fill_br);
            DeleteObject(rim_pen);

            text_aligned(
                dc,
                lbl,
                RECT { left: x, top: y, right: x + w, bottom: y + h },
                font_sz,
                700,
                rgb(234, 244, 248),
                DT_SINGLELINE | DT_CENTER | DT_VCENTER,
            );
            w
        }
    }
}

unsafe fn draw_button_hints(dc: HDC, left: i32, right: i32, y: i32, text_size: i32, items: &[(BtnIcon, &str)]) {
    let icon_h = (text_size + (text_size * 30) / 100).max(20);
    let inner_gap = (text_size * 34) / 100;
    let item_gap = (text_size * 105) / 100;
    let mut x = left;
    let snow = rgb(238, 245, 248);

    for &(icon, label) in items {
        if x >= right {
            break;
        }
        let iw = draw_btn_icon(dc, icon, x, y, icon_h);
        x += iw + inner_gap;
        let lw = measure_text_w(dc, label, text_size, 600);
        if x < right {
            let end_x = (x + lw + 4).min(right);
            text_aligned(
                dc,
                label,
                RECT { left: x, top: y, right: end_x, bottom: y + icon_h },
                text_size,
                600,
                snow,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
            );
        }
        x += lw + item_gap;
    }
}

unsafe fn draw_dib(dc: HDC, img: &DibImage, box_x: i32, box_y: i32, box_w: i32, box_h: i32) {
    if img.width == 0 || img.height == 0 || box_w <= 0 || box_h <= 0 {
        return;
    }
    // Preserve aspect ratio, centered inside (box_x, box_y, box_w, box_h)
    let sx = box_w as f64 / img.width as f64;
    let sy = box_h as f64 / img.height as f64;
    let factor = sx.min(sy);
    let dst_w = ((img.width as f64 * factor).round() as i32).max(1);
    let dst_h = ((img.height as f64 * factor).round() as i32).max(1);
    let dst_x = box_x + (box_w - dst_w) / 2;
    let dst_y = box_y + (box_h - dst_h) / 2;

    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = img.width as i32;
    bmi.bmiHeader.biHeight = img.height as i32; // positive = bottom-up DIB
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;

    SetStretchBltMode(dc, COLORONCOLOR);
    StretchDIBits(
        dc,
        dst_x,
        dst_y,
        dst_w,
        dst_h,
        0,
        0,
        img.width as i32,
        img.height as i32,
        img.bgra_bottom_up.as_ptr() as *const _,
        &bmi,
        DIB_RGB_COLORS,
        SRCCOPY,
    );
}

unsafe fn draw_qr(dc: HDC, qr: &QrCode, x: i32, y: i32, card_px: i32) {
    let white = rgb(255, 255, 255);
    let black = rgb(12, 16, 20);
    fill(dc, RECT { left: x, top: y, right: x + card_px, bottom: y + card_px }, white);

    let quiet = 3i32;
    let total = qr.size as i32 + quiet * 2;
    let module = (card_px / total).max(1);
    let grid_px = module * qr.size as i32;
    let ox = x + (card_px - grid_px) / 2;
    let oy = y + (card_px - grid_px) / 2;

    let brush = CreateSolidBrush(black);
    for my in 0..qr.size {
        let mut mx = 0usize;
        while mx < qr.size {
            if !qr.at(mx, my) {
                mx += 1;
                continue;
            }
            let start = mx;
            while mx < qr.size && qr.at(mx, my) {
                mx += 1;
            }
            let rect = RECT {
                left: ox + (start as i32) * module,
                top: oy + (my as i32) * module,
                right: ox + (mx as i32) * module,
                bottom: oy + ((my + 1) as i32) * module,
            };
            FillRect(dc, &rect, brush);
        }
    }
    DeleteObject(brush);
}

fn scroll_window(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible {
        0
    } else if selected + 1 > visible {
        (selected + 1 - visible).min(total - visible)
    } else {
        0
    }
}

unsafe fn paint(window: HWND, menu: &Menu) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let screen_dc = BeginPaint(window, &mut ps);
    let mut client: RECT = std::mem::zeroed();
    GetClientRect(window, &mut client);
    let width = (client.right - client.left).max(1);
    let height = (client.bottom - client.top).max(1);
    // Double-buffer into an offscreen memory DC and present atomically with one BitBlt
    // so Wine/PS5 VideoOut scanout never flashes an incomplete clear or partial redraw.
    let mem_dc = CreateCompatibleDC(screen_dc);
    let mem_bmp = if !mem_dc.is_null() { CreateCompatibleBitmap(screen_dc, width, height) } else { null_mut() };
    let (dc, old_bmp) = if !mem_dc.is_null() && !mem_bmp.is_null() {
        (mem_dc, SelectObject(mem_dc, mem_bmp))
    } else {
        (screen_dc, null_mut())
    };
    fill(dc, client, rgb(14, 23, 29));
    let scale = |vertical: i32| (vertical * client.bottom / 1080).max(1);
    let left = client.right / 14;
    let right = client.right - left;
    let region = |x, y, r, h| RECT { left: x, top: scale(y), right: r, bottom: scale(y + h) };
    let snow = rgb(238, 247, 248);
    let muted = rgb(178, 199, 203);
    let mint = rgb(133, 230, 189);
    let amber = rgb(247, 198, 114);
    let card_bg = rgb(28, 42, 49);
    let panel_bg = rgb(CARD_BG_RGB.0 as u32, CARD_BG_RGB.1 as u32, CARD_BG_RGB.2 as u32);
    let card_sel = rgb(39, 78, 72);

    // Top-right profile + active Frame Generation + build version badge on every screen
    let lang = menu.ui.language;
    let t = |key: &'static str| tr(lang, key);
    let fg_badge = if menu.active_framegen >= 2 {
        if menu.framegen != menu.active_framegen {
            format!("  ·  FG {}x*", menu.active_framegen)
        } else {
            format!("  ·  FG {}x", menu.active_framegen)
        }
    } else if menu.framegen >= 2 {
        format!("  ·  FG {}x*", menu.framegen)
    } else {
        String::new()
    };
    let badge_color = if menu.framegen != menu.active_framegen {
        amber
    } else if menu.active_framegen >= 2 {
        mint
    } else {
        muted
    };
    text_aligned(
        dc,
        &format!(
            "{}{fg_badge}  ·  TrinityXSX {}  ·  PPSA83601",
            menu.ui.current_profile().name,
            library::TRINITYXSX_VERSION
        ),
        region(right - scale(760), 68, right, 36),
        scale(23),
        600,
        badge_color,
        DT_SINGLELINE | DT_RIGHT,
    );

    match menu.screen {
        Screen::Library => {
            let active = menu.catalog.folders.iter().filter(|f| f.available).count();
            let sort_label = t(menu.ui.sort_mode.label_key());
            text(dc, "TrinityXSX", region(left, 56, right - scale(740), 80), scale(62), 700, snow);
            let installer_note = if !menu.installers.is_empty() {
                format!("  ·  {} {} (OPTIONS)", menu.installers.len(), t("installer(s) ready"))
            } else {
                String::new()
            };
            let header_sub = if menu.catalog.games.len() > GAME_ROWS {
                format!(
                    "{}  ·  {} {}  ·  {}/{} {}  ·  {sort_label}  ·  {} / {}{installer_note}",
                    t("Game library"),
                    menu.catalog.games.len(),
                    t("games"),
                    active,
                    menu.catalog.folders.len(),
                    t("folders"),
                    menu.selected + 1,
                    menu.catalog.games.len()
                )
            } else {
                format!(
                    "{}  ·  {} {}  ·  {}/{} {}  ·  {sort_label}{installer_note}",
                    t("Game library"),
                    menu.catalog.games.len(),
                    t("games"),
                    active,
                    menu.catalog.folders.len(),
                    t("folders")
                )
            };
            text(dc, &header_sub, region(left, 142, right, 44), scale(27), 400, mint);

            if menu.catalog.games.is_empty() {
                text(dc, t("No Xbox Series games found"), region(left, 290, right, 60), scale(40), 600, snow);
                text(
                    dc,
                    t("Press Square to add game folders (e.g. /mnt/ext1/xboxseries or USB), or copy games into:"),
                    region(left, 364, right, 42),
                    scale(24),
                    400,
                    muted,
                );
                for (idx, folder) in menu.catalog.folders.iter().take(4).enumerate() {
                    let y = 426 + (idx as i32) * 44;
                    let badge = if folder.available { "Available" } else { "Not found" };
                    let color = if folder.available { mint } else { muted };
                    text(
                        dc,
                        &format!("{}   [{}]", folder.path, badge),
                        region(left + scale(18), y, right, 38),
                        scale(24),
                        400,
                        color,
                    );
                }
                text(
                    dc,
                    t("Press X or Square to open Game folders  ·  Press L1 for Phone/PC Web Companion"),
                    region(left, 630, right, 44),
                    scale(24),
                    600,
                    snow,
                );
            } else {
                // Split layout: left game list (62%), right preview card with cover art (35%)
                let list_right = left + ((right - left) * 62) / 100;
                let preview_left = list_right + scale(34);

                let first = scroll_window(menu.selected, menu.catalog.games.len(), GAME_ROWS);
                for (i, game) in menu.catalog.games.iter().enumerate().skip(first).take(GAME_ROWS) {
                    let top = 210 + ((i - first) as i32) * 104;
                    let selected = i == menu.selected;
                    fill(dc, region(left, top, list_right, 92), if selected { card_sel } else { card_bg });
                    if selected {
                        fill(dc, region(left, top, left + scale(6), 92), mint);
                    }
                    let thumb_x = left + scale(18);
                    let thumb_y = scale(top + 10);
                    let thumb_w = scale(116);
                    let thumb_h = scale(72);
                    let thumb_img = game.logo.as_ref().and_then(|p| menu.cover_cache.get(p));
                    if let Some(img) = thumb_img {
                        draw_dib(dc, img, thumb_x, thumb_y, thumb_w, thumb_h);
                    } else {
                        fill(
                            dc,
                            RECT { left: thumb_x, top: thumb_y, right: thumb_x + thumb_w, bottom: thumb_y + thumb_h },
                            panel_bg,
                        );
                        text_aligned(
                            dc,
                            "XSX",
                            RECT {
                                left: thumb_x,
                                top: thumb_y + thumb_h / 2 - scale(14),
                                right: thumb_x + thumb_w,
                                bottom: thumb_y + thumb_h / 2 + scale(16),
                            },
                            scale(22),
                            700,
                            mint,
                            DT_SINGLELINE | DT_CENTER,
                        );
                    }
                    let inset = thumb_x + thumb_w + scale(20);
                    let display_title = if menu.ui.is_favorite(&game.identity) {
                        format!("★ {}", game.name)
                    } else {
                        game.name.clone()
                    };
                    text(dc, &display_title, region(inset, top + 8, list_right - scale(16), 44), scale(33), 600, snow);
                    text(
                        dc,
                        &format!("{}  ·  {}  ·  {}", game.identity, game.version, game.folder),
                        region(inset, top + 52, list_right - scale(16), 34),
                        scale(21),
                        400,
                        muted,
                    );
                }

                // Right-side preview card for selected game
                if let Some(game) = menu.catalog.games.get(menu.selected) {
                    let card_top = 210;
                    let card_h = 612;
                    fill(dc, region(preview_left, card_top, right, card_h), panel_bg);
                    fill(dc, region(preview_left, card_top, right, 4), mint);

                    let pad = scale(24);
                    let art_x = preview_left + pad;
                    let art_y = scale(card_top + 24);
                    let art_w = (right - preview_left) - pad * 2;
                    let art_h = scale(290);
                    let preview_cover = game.logo.as_ref().and_then(|p| menu.cover_cache.get(p));
                    if let Some(cover) = preview_cover {
                        draw_dib(dc, cover, art_x, art_y, art_w, art_h);
                    } else {
                        fill(
                            dc,
                            RECT { left: art_x, top: art_y, right: art_x + art_w, bottom: art_y + art_h },
                            card_bg,
                        );
                        text_aligned(
                            dc,
                            "XSX",
                            RECT {
                                left: art_x,
                                top: art_y + art_h / 2 - scale(36),
                                right: art_x + art_w,
                                bottom: art_y + art_h / 2 + scale(36),
                            },
                            scale(56),
                            700,
                            mint,
                            DT_SINGLELINE | DT_CENTER,
                        );
                    }

                    let info_left = preview_left + pad;
                    let info_right = right - pad;
                    text(dc, &game.name, region(info_left, card_top + 332, info_right, 40), scale(30), 700, snow);
                    let badge = menu.preview_pe_kind.badge();
                    let id_line = if game.title_id.is_empty() {
                        format!("[{badge}]  {}  ·  v{}", game.identity, game.version)
                    } else {
                        format!("[{badge}]  ID {}  ·  v{}", game.title_id, game.version)
                    };
                    text(dc, &id_line, region(info_left, card_top + 378, info_right, 32), scale(21), 400, muted);

                    let cfg_label = if menu.preview_has_custom_cfg {
                        t("Config: Custom xcl.toml")
                    } else if library::has_tested_preset(&game.title_id, &game.identity) {
                        t("Config: Tested PS5 preset")
                    } else {
                        t("Config: Auto PS5 defaults")
                    };
                    text(dc, cfg_label, region(info_left, card_top + 428, info_right, 32), scale(22), 600, mint);

                    let (sfiles, sbytes) = menu.preview_saves;
                    let saves_line = if sfiles == 0 {
                        t("Saves: None yet").into()
                    } else {
                        format!("Saves: {sfiles} files ({:.1} MiB)", (sbytes as f64) / (1024.0 * 1024.0))
                    };
                    text(dc, &saves_line, region(info_left, card_top + 468, info_right, 32), scale(21), 400, muted);
                    text(
                        dc,
                        &format!("Folder: {}", game.folder),
                        region(info_left, card_top + 506, info_right, 32),
                        scale(20),
                        400,
                        muted,
                    );
                    text(
                        dc,
                        t("Press R1 to edit xcl.toml settings"),
                        region(info_left, card_top + 558, info_right, 32),
                        scale(21),
                        600,
                        snow,
                    );
                }
            }

            if let Some(web) = &menu.web {
                let web_right = if menu.storage_line.is_empty() { right } else { left + ((right - left) * 56) / 100 };
                text(
                    dc,
                    &format!("Web & Logs:  {}   ·   PIN {}   (L1 -> QR)", web.url, web.pin),
                    region(left, 852, web_right, 34),
                    scale(21),
                    400,
                    muted,
                );
            }
            if !menu.storage_line.is_empty() {
                text_aligned(
                    dc,
                    &menu.storage_line,
                    region(left + ((right - left) * 56) / 100, 852, right, 34),
                    scale(20),
                    400,
                    muted,
                    DT_SINGLELINE | DT_RIGHT | DT_END_ELLIPSIS,
                );
            }
            text(dc, &menu.status, region(left, 896, right, 38), scale(23), 400, mint);
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(21),
                &[
                    (BtnIcon::Cross, t("Play")),
                    (BtnIcon::Pill("R1"), t("Game")),
                    (BtnIcon::Square, t("Folders")),
                    (BtnIcon::Pill("L1"), "Web/QR"),
                    (BtnIcon::Pill("OPTIONS"), t("Settings")),
                    (BtnIcon::Pill("L2"), t("Fav")),
                    (BtnIcon::Pill("R2"), t("Sort")),
                    (BtnIcon::Circle, t("Exit")),
                ],
            );
        }

        Screen::AppSettings => {
            text(dc, t("Settings"), region(left, 56, right - scale(660), 76), scale(52), 700, snow);
            text(
                dc,
                t("Install games, adjust display options, player profile and language."),
                region(left, 140, right, 44),
                scale(25),
                400,
                muted,
            );

            let profile = menu.ui.current_profile();
            let fg_base = match menu.framegen {
                2 => "2x (30 -> 60 FPS)".to_owned(),
                3 => "3x (20 -> 60 FPS)".to_owned(),
                4 => "4x (15 -> 60 FPS)".to_owned(),
                _ => t("Off").to_owned(),
            };
            let fg_pending = menu.framegen != menu.active_framegen;
            let fps_pending = menu.show_fps != menu.active_show_fps;
            let any_pending = fg_pending || fps_pending;
            let fg_val = if fg_pending { format!("{fg_base}  [restart]") } else { fg_base };
            let fps_base = t(if menu.show_fps { "On" } else { "Off" }).to_owned();
            let fps_val = if fps_pending { format!("{fps_base}  [restart]") } else { fps_base };
            let restart_val = if any_pending { t("Apply changes").to_owned() } else { t("Press X").to_owned() };
            let gog_val = if menu.installers.is_empty() {
                t("None found").to_owned()
            } else {
                format!("{} {}  [X]", menu.installers.len(), t("ready"))
            };
            let rows = [
                (t("Install GOG Games"), gog_val),
                (t("Interface language"), menu.ui.language.display_name().to_owned()),
                (t("Active offline profile"), format!("{}  (XUID {})", profile.name, profile.user_id)),
                (t("Frame Generation"), fg_val),
                (t("FPS Counter"), fps_val),
                (t("Restart TrinityXSX now"), restart_val),
            ];
            for (idx, (label, val)) in rows.iter().enumerate() {
                let top = 200 + (idx as i32) * 78;
                let selected = idx == menu.app_row;
                fill(dc, region(left, top, right, 68), if selected { card_sel } else { card_bg });
                if selected {
                    fill(
                        dc,
                        region(left, top, left + scale(6), 68),
                        if idx == 5 && any_pending { amber } else { mint },
                    );
                }
                text(dc, label, region(left + scale(26), top + 16, right - scale(500), 36), scale(26), 600, snow);
                let val_color = if idx == 5 && any_pending {
                    amber
                } else if (idx == 0 && !menu.installers.is_empty()) || selected {
                    mint
                } else {
                    muted
                };
                text_aligned(
                    dc,
                    val,
                    region(right - scale(520), top + 16, right - scale(26), 36),
                    scale(26),
                    600,
                    val_color,
                    DT_SINGLELINE | DT_RIGHT | DT_END_ELLIPSIS,
                );
            }

            text(
                dc,
                t("Frame Generation creates smoother motion (requires restarting TrinityXSX)."),
                region(left, 682, right, 42),
                scale(23),
                400,
                snow,
            );
            text(
                dc,
                t("Custom gamertags can also be added from the Phone/PC Web Companion (L1)."),
                region(left, 730, right, 42),
                scale(23),
                400,
                muted,
            );
            text(dc, &menu.status, region(left, 892, right, 38), scale(23), 400, mint);
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(23),
                &[
                    (BtnIcon::Cross, t("Select")),
                    (BtnIcon::Pill("<-/->"), t("Change")),
                    (BtnIcon::Triangle, t("Restart")),
                    (BtnIcon::Circle, t("Back")),
                ],
            );
        }

        Screen::Installers => {
            text(dc, t("Install GOG Games"), region(left, 56, right - scale(360), 76), scale(44), 700, snow);
            text(
                dc,
                t("Place GOG installer files (setup_*.exe + .bin) in /mnt/ext1/xboxseries/installers/"),
                region(left, 138, right, 40),
                scale(23),
                400,
                muted,
            );
            if let Some(job) = &menu.install_job {
                let done = job.done.load(Ordering::Relaxed);
                let total = job.total.load(Ordering::Relaxed).max(1);
                let frac = (done as f64 / total as f64).clamp(0.0, 1.0);
                let pct = (frac * 100.0) as u32;
                fill(dc, region(left, 250, right, 280), panel_bg);
                fill(dc, region(left, 250, right, 4), mint);
                text(
                    dc,
                    &format!("{} {}...", t("Installing"), job.game_name),
                    region(left + scale(32), 290, right - scale(32), 48),
                    scale(34),
                    700,
                    snow,
                );
                let bar_l = left + scale(32);
                let bar_r = right - scale(32);
                let bar_w = (bar_r - bar_l).max(1);
                fill(dc, region(bar_l, 370, bar_r, 38), card_bg);
                let fill_w = ((bar_w as f64) * frac) as i32;
                if fill_w > 0 {
                    fill(dc, region(bar_l, 370, bar_l + fill_w, 38), mint);
                }
                let eta_line = format_install_eta(done, total, job.started.elapsed().as_secs_f64());
                text(
                    dc,
                    &format!(
                        "{pct}%   ·   {:.1} / {:.1} MiB   ·   {eta_line}",
                        (done as f64) / (1024.0 * 1024.0),
                        (total as f64) / (1024.0 * 1024.0)
                    ),
                    region(bar_l, 430, bar_r, 38),
                    scale(24),
                    600,
                    mint,
                );
                text(
                    dc,
                    t("Keep your PS5 awake until installation finishes"),
                    region(bar_l, 476, bar_r, 30),
                    scale(20),
                    400,
                    muted,
                );
            } else if menu.installers.is_empty() {
                fill(dc, region(left, 230, right, 240), panel_bg);
                text(
                    dc,
                    t("No GOG installers found."),
                    region(left + scale(28), 270, right - scale(28), 44),
                    scale(32),
                    600,
                    snow,
                );
                text(
                    dc,
                    t("Copy setup_<game>.exe (+ .bin files) to /mnt/ext1/xboxseries/installers/ and press Triangle to rescan."),
                    region(left + scale(28), 336, right - scale(28), 72),
                    scale(24),
                    400,
                    muted,
                );
            } else {
                let first = scroll_window(menu.installer_row, menu.installers.len(), PATH_ROWS);
                for (row, i) in (first..(first + PATH_ROWS).min(menu.installers.len())).enumerate() {
                    let top = 212 + (row as i32) * 96;
                    let inst = &menu.installers[i];
                    let selected = i == menu.installer_row;
                    fill(dc, region(left, top, right, 82), if selected { card_sel } else { card_bg });
                    if selected {
                        fill(dc, region(left, top, left + scale(6), 82), mint);
                    }
                    text(
                        dc,
                        &inst.display_name,
                        region(left + scale(24), top + 10, right - scale(280), 38),
                        scale(30),
                        600,
                        snow,
                    );
                    let detail = format!(
                        "{}  ·  {} file(s)  ->  {}",
                        library::display_path(&inst.setup_exe),
                        1 + inst.slice_files.len(),
                        inst.slug
                    );
                    text(
                        dc,
                        &detail,
                        region(left + scale(24), top + 48, right - scale(280), 28),
                        scale(20),
                        400,
                        muted,
                    );
                    let size_lbl = format!("{:.2} GiB", (inst.total_bytes as f64) / (1024.0 * 1024.0 * 1024.0));
                    text_aligned(
                        dc,
                        &size_lbl,
                        region(right - scale(260), top + 22, right - scale(24), 38),
                        scale(26),
                        700,
                        mint,
                        DT_SINGLELINE | DT_RIGHT,
                    );
                }
            }
            let del_lbl = t(if menu.delete_installer_after { "On" } else { "Off" });
            let del_text = format!("{} {del_lbl}", t("Delete files after:"));
            text(dc, &menu.status, region(left, 892, right, 38), scale(23), 400, mint);
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(22),
                &[
                    (BtnIcon::Cross, t("Install")),
                    (BtnIcon::Triangle, t("Rescan")),
                    (BtnIcon::Square, &del_text),
                    (BtnIcon::Circle, t("Back")),
                ],
            );
        }

        Screen::GameSettings => {
            let game_name = menu.catalog.games.get(menu.selected).map(|g| g.name.as_str()).unwrap_or("Game");
            let game_id = menu.catalog.games.get(menu.selected).map(|g| g.identity.as_str()).unwrap_or("");
            text(
                dc,
                &format!("{}  ·  {game_name}", t("Game settings")),
                region(left, 56, right - scale(660), 76),
                scale(48),
                700,
                snow,
            );
            let cfg_display = library::display_path(&library::config_path_for_title(game_id));
            text(
                dc,
                &format!("{}   ({} / {})", cfg_display, menu.settings_row + 1, SETTINGS_COUNT),
                region(left, 138, right, 40),
                scale(23),
                400,
                muted,
            );

            let first = scroll_window(menu.settings_row, SETTINGS_COUNT, SETTINGS_ROWS);
            for idx in first..(first + SETTINGS_ROWS).min(SETTINGS_COUNT) {
                let top = 198 + ((idx - first) as i32) * 76;
                let selected = idx == menu.settings_row;
                let (label, value, _) = menu.setting_row_info(idx);
                fill(dc, region(left, top, right, 66), if selected { card_sel } else { card_bg });
                if selected {
                    fill(dc, region(left, top, left + scale(6), 66), mint);
                }
                text(dc, label, region(left + scale(24), top + 16, right - scale(440), 36), scale(26), 600, snow);
                text_aligned(
                    dc,
                    &value,
                    region(right - scale(460), top + 16, right - scale(24), 36),
                    scale(26),
                    600,
                    if selected { mint } else { muted },
                    DT_SINGLELINE | DT_RIGHT | DT_END_ELLIPSIS,
                );
            }

            let (_, _, about) = menu.setting_row_info(menu.settings_row);
            text(dc, about, region(left, 826, right, 44), scale(22), 400, snow);
            text(dc, &menu.status, region(left, 892, right, 38), scale(23), 400, mint);
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(23),
                &[
                    (BtnIcon::Pill("D-PAD"), t("Navigate")),
                    (BtnIcon::Cross, t("Change & Save")),
                    (BtnIcon::Triangle, t("Reset preset")),
                    (BtnIcon::Circle, t("Back")),
                ],
            );
        }

        Screen::Paths => {
            text(dc, t("Game folders"), region(left, 56, right - scale(660), 80), scale(54), 700, snow);
            let sub = if menu.paths.len() > PATH_ROWS {
                format!(
                    "{}   ({} / {})",
                    t("Add internal, /mnt/ext1 or USB folders. Disconnected folders stay saved."),
                    menu.path_row + 1,
                    menu.paths.len()
                )
            } else {
                t("Add internal, /mnt/ext1 or USB folders. Disconnected folders stay saved.").into()
            };
            text(dc, &sub, region(left, 140, right, 44), scale(24), 400, muted);

            if menu.paths.is_empty() {
                text(dc, t("No game folders configured."), region(left, 320, right, 52), scale(34), 600, snow);
                text(
                    dc,
                    t("Press Square to browse and add a folder."),
                    region(left, 382, right, 44),
                    scale(24),
                    400,
                    muted,
                );
            } else {
                let first = scroll_window(menu.path_row, menu.paths.len(), PATH_ROWS);
                for (i, folder) in menu.catalog.folders.iter().enumerate().skip(first).take(PATH_ROWS) {
                    let top = 210 + ((i - first) as i32) * 102;
                    let selected = i == menu.path_row;
                    fill(dc, region(left, top, right, 90), if selected { card_sel } else { card_bg });
                    if selected {
                        fill(dc, region(left, top, left + scale(6), 90), mint);
                    }
                    let inset = left + scale(26);
                    text(dc, &folder.path, region(inset, top + 8, right - scale(20), 42), scale(30), 600, snow);
                    let (state_text, state_color) = if folder.available {
                        let label = if folder.game_count == 1 {
                            "1 game".into()
                        } else {
                            format!("{} {}", folder.game_count, t("games"))
                        };
                        (format!("Available  ·  {label}"), mint)
                    } else {
                        (t("Disconnected or unavailable").into(), muted)
                    };
                    text(dc, &state_text, region(inset, top + 50, right - scale(20), 34), scale(22), 400, state_color);
                }
            }

            let info = format!(
                "Removing a folder never deletes games on disk.   Config: {}",
                library::display_path(&menu.paths_file)
            );
            text(dc, &info, region(left, 846, right, 36), scale(21), 400, muted);
            text(dc, &menu.status, region(left, 892, right, 42), scale(24), 400, mint);
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(24),
                &[
                    (BtnIcon::Cross, t("Browse")),
                    (BtnIcon::Square, t("Add folder")),
                    (BtnIcon::Triangle, t("Remove")),
                    (BtnIcon::Circle, t("Back")),
                ],
            );
        }

        Screen::Browser => {
            text(dc, t("Choose folder"), region(left, 56, right - scale(660), 76), scale(54), 700, snow);
            let current_header = if menu.browser_entries.len() > BROWSER_ROWS {
                format!("{}   ({} / {})", menu.browser_path, menu.browser_row + 1, menu.browser_entries.len())
            } else {
                menu.browser_path.clone()
            };
            text(dc, &current_header, region(left, 140, right, 46), scale(28), 600, mint);

            if menu.browser_entries.is_empty() && menu.browser_error.is_empty() {
                text(
                    dc,
                    t("No subfolders here. Press Triangle to add the current folder."),
                    region(left, 260, right, 48),
                    scale(28),
                    400,
                    muted,
                );
            } else {
                let first = scroll_window(menu.browser_row, menu.browser_entries.len(), BROWSER_ROWS);
                for (i, entry) in menu.browser_entries.iter().enumerate().skip(first).take(BROWSER_ROWS) {
                    let top = 206 + ((i - first) as i32) * 86;
                    let selected = i == menu.browser_row;
                    fill(dc, region(left, top, right, 76), if selected { card_sel } else { card_bg });
                    if selected {
                        fill(dc, region(left, top, left + scale(6), 76), mint);
                    }
                    let inset = left + scale(26);
                    let label =
                        if entry.is_game { format!("{}   [Xbox Series game]", entry.name) } else { entry.name.clone() };
                    text(
                        dc,
                        &label,
                        region(inset, top + 18, right - scale(20), 42),
                        scale(28),
                        600,
                        if entry.is_game { mint } else { snow },
                    );
                }
            }

            if !menu.browser_error.is_empty() {
                text(dc, &menu.browser_error, region(left, 842, right, 40), scale(24), 600, amber);
            }
            text(
                dc,
                t("Triangle adds the current folder. Circle goes up one level."),
                region(left, 892, right, 40),
                scale(23),
                400,
                muted,
            );
            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(24),
                &[
                    (BtnIcon::Cross, t("Open")),
                    (BtnIcon::Triangle, t("Use folder")),
                    (BtnIcon::Circle, t("Up")),
                    (BtnIcon::Square, t("Cancel")),
                ],
            );
        }

        Screen::WebQr => {
            text(dc, t("Phone & PC Web Companion"), region(left, 56, right - scale(660), 76), scale(52), 700, snow);
            text(
                dc,
                t("Connect from the same Wi-Fi network to download logs ZIP or edit xcl.toml and folders."),
                region(left, 140, right, 44),
                scale(25),
                400,
                muted,
            );

            if let Some(web) = &menu.web {
                let card_px = scale(420);
                let qr_x = left;
                let qr_y = scale(230);
                if let Some(qr) = &menu.qr {
                    draw_qr(dc, qr, qr_x, qr_y, card_px);
                }

                let text_x = qr_x + card_px + scale(56);
                text(
                    dc,
                    t("1. Scan QR code with your phone camera"),
                    region(text_x, 250, right, 44),
                    scale(30),
                    600,
                    snow,
                );
                text(
                    dc,
                    t("Or open this address in any browser:"),
                    region(text_x, 314, right, 36),
                    scale(24),
                    400,
                    muted,
                );
                text(dc, &web.url, region(text_x, 360, right, 48), scale(36), 700, mint);
                text(dc, &format!("PIN:  {}", web.pin), region(text_x, 424, right, 48), scale(36), 700, snow);

                text(dc, t("What you can do from the browser:"), region(text_x, 514, right, 36), scale(25), 600, snow);
                text(
                    dc,
                    t("• Download trinityxsx-logs.zip for GitHub bug reports (no FTP needed)"),
                    region(text_x, 560, right, 34),
                    scale(22),
                    400,
                    muted,
                );
                text(
                    dc,
                    t("• Edit per-game xcl.toml with syntax validation"),
                    region(text_x, 600, right, 34),
                    scale(22),
                    400,
                    muted,
                );
                text(
                    dc,
                    t("• Edit game_paths.txt, profile and UI language"),
                    region(text_x, 640, right, 34),
                    scale(22),
                    400,
                    muted,
                );
                text(
                    dc,
                    t("Server stops automatically when you launch a game."),
                    region(left, 710, right, 36),
                    scale(22),
                    400,
                    muted,
                );
            } else {
                text(
                    dc,
                    "Web server could not bind a local port on this system.",
                    region(left, 320, right, 48),
                    scale(30),
                    600,
                    amber,
                );
            }

            draw_button_hints(
                dc,
                left,
                right,
                scale(954),
                scale(24),
                &[(BtnIcon::Circle, t("Back")), (BtnIcon::Pill("L1"), t("Back to library"))],
            );
        }
    }

    if !mem_dc.is_null() && !mem_bmp.is_null() && dc == mem_dc {
        BitBlt(screen_dc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);
        SelectObject(mem_dc, old_bmp);
    }
    if !mem_bmp.is_null() {
        DeleteObject(mem_bmp);
    }
    if !mem_dc.is_null() {
        DeleteDC(mem_dc);
    }
    EndPaint(window, &ps);
}

unsafe extern "system" fn window_proc(window: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    if msg == WM_ERASEBKGND {
        return 1;
    }
    let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Menu;
    let Some(menu) = pointer.as_mut() else { return DefWindowProcW(window, msg, wparam, lparam) };
    let pressed = match msg {
        WM_PAINT => {
            paint(window, menu);
            return 0;
        }
        WM_TIMER => {
            if menu.poll_install_job() {
                InvalidateRect(window, null(), 0);
            }
            if menu.web.as_ref().is_some_and(|w| w.take_restart_requested()) {
                let _ = library::request_app_restart();
                menu.chosen = None;
                DestroyWindow(window);
                return 0;
            }
            if menu.web.as_ref().is_some_and(|w| w.take_dirty()) {
                menu.scan();
                menu.status = "Updated from Web Companion".into();
                InvalidateRect(window, null(), 0);
            } else if menu.screen == Screen::Library {
                menu.watch_ticks = menu.watch_ticks.saturating_add(1);
                if menu.watch_ticks >= AUTO_WATCH_TICKS {
                    menu.watch_ticks = 0;
                    let sig = library::folders_signature(&menu.paths);
                    if sig != menu.folder_sig {
                        menu.scan();
                        menu.status = tr(menu.ui.language, "Library updated automatically").into();
                        InvalidateRect(window, null(), 0);
                    }
                }
            }
            menu.pad_buttons()
        }
        WM_KEYDOWN => match wparam {
            0x26 => UP,
            0x28 => DOWN,
            0x25 => LEFT,
            0x27 => RIGHT,
            0x0d => CROSS,
            0x1b | 0x08 => CIRCLE,
            0x46 | 0x20 => SQUARE,
            0x52 | 0x54 => TRIANGLE,
            0x53 => R1,    // 'S' -> Game settings
            0x57 => L1,    // 'W' -> Web / QR
            0x42 => L2,    // 'B' -> Favorite
            0x4f => R2,    // 'O' -> Sort
            0x50 => START, // 'P' -> Profile & Language
            _ => 0,
        },
        _ => return DefWindowProcW(window, msg, wparam, lparam),
    };
    if pressed != 0 {
        if menu.input(pressed) {
            DestroyWindow(window);
        } else {
            InvalidateRect(window, null(), 0);
        }
    }
    0
}

fn choose(primary_root: Option<PathBuf>) -> Result<Option<(String, PathBuf)>, String> {
    let read_pad = unsafe {
        super::system_proc("xinput1_4.dll", "XInputGetState")
            .or_else(|| super::system_proc("xinput1_3.dll", "XInputGetState"))
            .map(|proc| std::mem::transmute::<usize, ReadPad>(proc))
    };
    let paths_file = library::paths_file();
    let ui_file = library::ui_settings_file();
    let profile_file = library::runtime_profile_file();
    let ui = library::load_ui_settings(&ui_file);
    let framegen = library::read_framegen_setting(&profile_file);
    let show_fps = library::read_show_fps_setting(&profile_file);
    let active_framegen = std::env::var("PW_FRAMEGEN")
        .ok()
        .and_then(|v| v.trim().parse::<u8>().ok())
        .filter(|&n| (2..=4).contains(&n))
        .unwrap_or(0);
    let active_show_fps = show_fps;
    let paths = library::load_or_init_paths(&paths_file, primary_root.as_deref());
    let web = WebCompanion::start(paths_file.clone());
    let qr = web.as_ref().and_then(|w| library_art::encode_qr(&w.qr_url));
    if let Some(w) = &web {
        println!("TrinityXSX: Web Companion at {} (PIN {})", w.url, w.pin);
    }

    let mut menu = Menu {
        paths_file,
        ui_file,
        profile_file,
        ui,
        framegen,
        active_framegen,
        show_fps,
        active_show_fps,
        storage_line: String::new(),
        app_row: 0,
        paths,
        catalog: Catalog::default(),
        folder_sig: 0,
        watch_ticks: 0,
        screen: Screen::Library,
        selected: 0,
        cover_cache: library_art::CoverCache::default(),
        preview_key: None,
        preview_pe_kind: PeLaunchKind::XboxSeries,
        preview_has_custom_cfg: false,
        preview_saves: (0, 0),
        preview_shader_cache: None,
        installers: Vec::new(),
        installer_row: 0,
        delete_installer_after: false,
        install_job: None,
        settings_cfg: Config::default(),
        settings_row: 0,
        settings_preset: 0,
        path_row: 0,
        browser_path: String::new(),
        browser_entries: Vec::new(),
        browser_row: 0,
        browser_error: String::new(),
        web,
        qr,
        previous: None,
        hold_dir: 0,
        hold_ticks: 0,
        chosen: None,
        status: String::new(),
        read_pad,
    };
    menu.scan();
    unsafe {
        let class = super::wide("TrinityXSXLibrary");
        let instance = GetModuleHandleW(null());
        let desc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&desc) == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let title = super::wide("TrinityXSX");
        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        if window.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        SetWindowLongPtrW(window, GWLP_USERDATA, &mut menu as *mut Menu as isize);
        ShowWindow(window, SW_SHOW);
        SetTimer(window, 1, 25, None);
        println!("TrinityXSX: library window opened (XInput={})", read_pad.is_some());
        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // Shut down Web Companion cleanly BEFORE in-process game launch!
    if let Some(mut web) = menu.web.take() {
        web.shutdown();
    }
    Ok(menu.chosen)
}

/// Run one title of the benchmark queue (`library::bench`). The library is asked to start again
/// afterwards, whatever the title does, so the queue moves on to the next title.
fn run_bench(job: library::bench::Job) -> i32 {
    let _ = library::request_app_restart();
    let paths = library::load_or_init_paths(&library::paths_file(), None);
    let catalog = library::scan_paths(&paths);
    let Some(game) = catalog.games.iter().find(|g| g.identity.eq_ignore_ascii_case(&job.identity)) else {
        println!("TrinityXSX: benchmark title {} is not in the library", job.identity);
        library::bench::record_failure(&job, "not-found");
        return 0;
    };
    if matches!(library_gog::detect_pe_kind(&game.executable), PeLaunchKind::PcNative { .. }) {
        library::bench::record_failure(&job, "unsupported");
        return 0;
    }
    if let Err(e) = library::prepare(game) {
        println!("TrinityXSX: benchmark title {} cannot start: {e}", job.identity);
        library::bench::record_failure(&job, "prepare-failed");
        return 0;
    }
    for (name, value) in &job.env {
        std::env::set_var(name, value);
    }
    println!("TrinityXSX: benchmark {} -> {}", game.executable.display(), job.out.display());
    xcl::launcher::run_with(&game.executable, &[], true)
}

pub fn run_direct(folder: PathBuf) -> i32 {
    library::ensure_shader_cache_dirs();
    match library::inspect_game_folder(&folder) {
        Ok(game) => match library::prepare(&game) {
            Ok(()) => {
                println!("TrinityXSX: direct forwarder starting {}", game.executable.display());
                xcl::launcher::run_with(&game.executable, &[], true)
            }
            Err(error) => {
                eprintln!("TrinityXSX: {error}");
                2
            }
        },
        Err(error) => {
            eprintln!("TrinityXSX: cannot inspect {}: {error}", folder.display());
            2
        }
    }
}

pub fn run(root: Option<PathBuf>, list: bool) -> i32 {
    for dir in library::ensure_shader_cache_dirs() {
        println!("TrinityXSX: created shader cache folder {}", dir.display());
    }
    if list {
        if let Some(root) = root {
            match library::scan(&root) {
                Ok((games, skipped)) => {
                    for game in games {
                        println!("{}\t{}\t{}\t{}", game.name, game.identity, game.version, game.executable.display());
                    }
                    println!("Skipped folders: {skipped}");
                    return 0;
                }
                Err(error) => {
                    eprintln!("TrinityXSX: {error}");
                    return 2;
                }
            }
        }
        let paths = library::load_or_init_paths(&library::paths_file(), None);
        let catalog = library::scan_paths(&paths);
        for game in catalog.games {
            println!("{}\t{}\t{}\t{}", game.name, game.identity, game.version, game.executable.display());
        }
        println!("Skipped folders: {}", catalog.skipped);
        return 0;
    }
    if let Some(job) = library::bench::next_job(&library::storage_root_dir()) {
        return run_bench(job);
    }
    if let Some(executable) = library::take_restart_game_target() {
        println!("TrinityXSX: restarting last game {}", executable.display());
        let _ = library::save_last_launched_game(&executable);
        return xcl::launcher::run_with(&executable, &[], true);
    }
    match std::thread::spawn(move || choose(root)).join() {
        Ok(Ok(Some((name, executable)))) => {
            let pe_kind = library_gog::detect_pe_kind(&executable);
            println!("TrinityXSX: starting {} [{}]", executable.display(), pe_kind.badge());
            let _ = library::save_last_launched_game(&executable);
            if let PeLaunchKind::PcNative { is_64bit, prefer_opengl } = pe_kind {
                if library_gog::ps5_pc_handoff_available() {
                    match library_gog::write_pc_handoff(&name, &executable, is_64bit, prefer_opengl) {
                        Ok(()) => {
                            println!("TrinityXSX: handing off PC/GOG game '{name}' to prospero-win");
                            return 0;
                        }
                        Err(err) => {
                            eprintln!("TrinityXSX: PC handoff failed ({err}), falling back to XCL");
                        }
                    }
                }
            }
            xcl::launcher::run_with(&executable, &[], true)
        }
        Ok(Ok(None)) => 0,
        Ok(Err(error)) => {
            eprintln!("TrinityXSX: {error}");
            2
        }
        Err(_) => {
            eprintln!("TrinityXSX: library thread ended unexpectedly");
            2
        }
    }
}
