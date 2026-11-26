//! Built-in LAN Web Companion for TrinityXSX (clean shutdown before game launch).
//!
//! Lets users on the same Wi-Fi scan a QR code on their phone/PC to:
//! - Download a ready-to-attach `trinityxsx-logs.zip` (no FTP or Python required)
//! - Edit `game_paths.txt` from a phone/PC keyboard
//! - View & edit per-game `xcl.toml` settings with TOML validation

use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::library;

const PREFERRED_PORT: u16 = 8360;
const MAX_LOG_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;
const MAX_SAVE_ZIP_BYTES: usize = 32 * 1024 * 1024;
const MAX_SAVE_UNCOMPRESSED_BYTES: usize = 64 * 1024 * 1024;
const MAX_SAVE_FILES: usize = 4096;
/// Wrong PINs allowed before the Web Companion starts refusing PIN checks.
const PIN_FREE_ATTEMPTS: u32 = 5;
/// First lockout; doubles with every further wrong PIN up to `PIN_MAX_LOCKOUT`.
const PIN_BASE_LOCKOUT: Duration = Duration::from_secs(30);
const PIN_MAX_LOCKOUT: Duration = Duration::from_secs(600);
/// Distinct wrong PINs remembered so that repeating one is not counted again.
const PIN_REMEMBERED_WRONG: usize = 64;
/// Session cookie set once the PIN is accepted, so the PIN does not stay in links, history or logs.
const SESSION_COOKIE: &str = "txsx_session";

pub struct WebCompanion {
    pub url: String,
    pub qr_url: String,
    pub pin: String,
    stop: Arc<AtomicBool>,
    pub dirty: Arc<AtomicBool>,
    pub restart_requested: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WebCompanion {
    pub fn start(paths_file: PathBuf) -> Option<Self> {
        let listener =
            TcpListener::bind(("0.0.0.0", PREFERRED_PORT)).or_else(|_| TcpListener::bind(("0.0.0.0", 0))).ok()?;
        let port = listener.local_addr().ok()?.port();
        let _ = listener.set_nonblocking(true);

        let ip = detect_lan_ip().unwrap_or_else(|| "127.0.0.1".into());
        let pin = generate_pin();
        let url = format!("http://{ip}:{port}");
        let qr_url = qr_login_url(&url, &pin);

        let stop = Arc::new(AtomicBool::new(false));
        let dirty = Arc::new(AtomicBool::new(false));
        let restart_requested = Arc::new(AtomicBool::new(false));
        let stop_bg = Arc::clone(&stop);
        let dirty_bg = Arc::clone(&dirty);
        let restart_bg = Arc::clone(&restart_requested);
        let pin_bg = pin.clone();
        let session_bg = generate_session_token();

        let thread = thread::spawn(move || {
            let mut guard = PinGuard::default();
            while !stop_bg.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
                        handle_connection(
                            stream,
                            &pin_bg,
                            &session_bg,
                            &mut guard,
                            &paths_file,
                            &dirty_bg,
                            &restart_bg,
                        );
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
        });

        Some(Self { url, qr_url, pin, stop, dirty, restart_requested, thread: Some(thread) })
    }

    pub fn take_dirty(&self) -> bool {
        self.dirty.swap(false, Ordering::AcqRel)
    }

    pub fn take_restart_requested(&self) -> bool {
        self.restart_requested.swap(false, Ordering::AcqRel)
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for WebCompanion {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn detect_lan_ip() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    for target in ["192.0.2.1:80", "10.255.255.255:80", "8.8.8.8:80"] {
        if sock.connect(target).is_ok() {
            if let Ok(addr) = sock.local_addr() {
                if let IpAddr::V4(v4) = addr.ip() {
                    if !v4.is_loopback() && !v4.is_unspecified() {
                        return Some(v4.to_string());
                    }
                }
            }
        }
    }
    None
}

/// The QR code carries the PIN so one scan logs in; the server trades it for the session cookie and
/// redirects to the same page without it.
fn qr_login_url(url: &str, pin: &str) -> String {
    format!("{url}/?pin={pin}")
}

/// 128-bit session token. `RandomState` keys come from the OS RNG; SipHash of a counter and the time
/// under those keys is not predictable without them.
fn generate_session_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let mut out = String::with_capacity(32);
    for i in 0..2u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(nanos);
        h.write_u64(i);
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

fn generate_pin() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0x5A3C_1234);
    // `RandomState` keys come from the OS RNG, so the PIN does not follow from the start time alone.
    let mut os_seeded = std::collections::hash_map::RandomState::new().build_hasher();
    os_seeded.write_u64(nanos);
    let mut x = os_seeded.finish() ^ nanos ^ ((std::process::id() as u64) << 17) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    format!("{:06}", x % 1_000_000)
}

/// Limits PIN guessing from the LAN. The count is global, not per client: a client on the LAN picks its
/// own address. A correct PIN does not reset it either, since the dashboard polls with the PIN every few
/// seconds and would otherwise hand a guesser a fresh set of tries each time. The PIN changes whenever the
/// Web Companion restarts, which is also when the count starts over.
///
/// Only distinct wrong PINs count. A dashboard tab left open from before a restart keeps polling with the
/// old PIN; counting every poll would lock the user out of the new session with no attacker involved.
/// Repeating a value already known to be wrong tells a guesser nothing, so the limit is as strong.
#[derive(Default)]
struct PinGuard {
    failures: u32,
    locked_until: Option<Instant>,
    wrong: Vec<String>,
}

enum PinCheck {
    Accepted,
    Rejected,
    LockedOut(Duration),
}

impl PinGuard {
    fn check(&mut self, supplied: &str, pin: &str, now: Instant) -> PinCheck {
        if let Some(until) = self.locked_until {
            if now < until {
                return PinCheck::LockedOut(until - now);
            }
            self.locked_until = None;
        }
        if constant_time_eq(supplied.as_bytes(), pin.as_bytes()) {
            return PinCheck::Accepted;
        }
        if self.wrong.iter().any(|w| w == supplied) {
            return PinCheck::Rejected;
        }
        if self.wrong.len() < PIN_REMEMBERED_WRONG && supplied.len() <= 16 {
            self.wrong.push(supplied.to_owned());
        }
        self.failures = self.failures.saturating_add(1);
        if self.failures >= PIN_FREE_ATTEMPTS {
            let doublings = (self.failures - PIN_FREE_ATTEMPTS).min(16);
            let lockout = PIN_BASE_LOCKOUT.saturating_mul(1 << doublings).min(PIN_MAX_LOCKOUT);
            self.locked_until = Some(now + lockout);
            return PinCheck::LockedOut(lockout);
        }
        PinCheck::Rejected
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn handle_connection(
    mut stream: TcpStream,
    pin: &str,
    session: &str,
    guard: &mut PinGuard,
    paths_file: &Path,
    dirty: &AtomicBool,
    restart_requested: &AtomicBool,
) {
    let Some(request) = read_http_request(&mut stream) else {
        return;
    };
    let HttpRequest { method, target, body, .. } = &request;
    let (method, body) = (method.as_str(), body.as_slice());
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let params = parse_query(query);

    // Forms and fetch() calls from another site must not act with the user's cookie. Browsers send
    // Origin on every POST; when it is there it has to be this server.
    if method != "GET" && !origin_allowed(request.origin.as_deref(), request.host.as_deref()) {
        send_response(&mut stream, "403 Forbidden", "text/plain; charset=utf-8", b"Cross-site request refused", None);
        return;
    }

    let has_session = request.session.as_deref().is_some_and(|s| constant_time_eq(s.as_bytes(), session.as_bytes()));
    let supplied_pin = params.iter().find(|(k, _)| k == "pin").map(|(_, v)| v.as_str()).filter(|p| !p.is_empty());

    if !has_session {
        // No valid session and no PIN (first visit, favicon, a tab from before a restart): only the
        // login page, not counted as a guess. 401 for anything but the page itself, so the dashboard
        // script can tell its session is gone.
        let Some(supplied_pin) = supplied_pin else {
            let html = render_login_page(path, "");
            let status = if path == "/" { "200 OK" } else { "401 Unauthorized" };
            send_response(&mut stream, status, "text/html; charset=utf-8", html.as_bytes(), None);
            return;
        };
        match guard.check(supplied_pin, pin, Instant::now()) {
            PinCheck::Accepted => {
                if method == "GET" {
                    // Trade the PIN for the cookie and drop it from the address bar and history.
                    let cookie = format!("{SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Lax");
                    send_redirect_with_headers(&mut stream, &without_pin(path, query), &[("Set-Cookie", &cookie)]);
                    return;
                }
            }
            PinCheck::Rejected => {
                let html = render_login_page(path, "Wrong PIN. Check the PIN shown on your TV.");
                send_response(&mut stream, "403 Forbidden", "text/html; charset=utf-8", html.as_bytes(), None);
                return;
            }
            PinCheck::LockedOut(wait) => {
                // Round up, so a client that waits exactly this long is not refused again.
                let secs = (wait.as_secs() + u64::from(wait.subsec_nanos() > 0)).max(1);
                let html = render_login_page(path, &format!("Too many wrong PINs. Try again in {secs} s."));
                let retry_after = secs.to_string();
                send_response_with_headers(
                    &mut stream,
                    "429 Too Many Requests",
                    "text/html; charset=utf-8",
                    html.as_bytes(),
                    &[("Retry-After", retry_after.as_str())],
                );
                return;
            }
        }
    }

    match (method, path) {
        ("GET", "/") => {
            // `msg` is the result of the dashboard's own last POST. A link from another site could put any
            // text there and have the real dashboard show it, so it is shown only for this site's own
            // navigations (browsers that do not send Sec-Fetch-Site still get it).
            let own_navigation = request.fetch_site.as_deref().is_none_or(|site| site == "same-origin");
            let notice = params
                .iter()
                .find(|(k, _)| k == "msg")
                .map(|(_, v)| v.as_str())
                .filter(|_| own_navigation)
                .unwrap_or("");
            let html = render_dashboard(paths_file, notice);
            send_response(&mut stream, "200 OK", "text/html; charset=utf-8", html.as_bytes(), None);
        }
        ("GET", "/logs.zip") => {
            let zip = build_logs_zip(paths_file);
            send_response(
                &mut stream,
                "200 OK",
                "application/zip",
                &zip,
                Some("attachment; filename=\"trinityxsx-logs.zip\""),
            );
        }
        ("GET", "/logs/tail") => {
            let kind = params.iter().find(|(k, _)| k == "file").map(|(_, v)| v.as_str()).unwrap_or("session");
            let text = read_log_tail(kind);
            send_response(&mut stream, "200 OK", "text/plain; charset=utf-8", text.as_bytes(), None);
        }
        ("POST", "/clear-cache") => {
            let body_str = String::from_utf8_lossy(body);
            let form = parse_query(&body_str);
            let title =
                form.iter().find(|(k, _)| k == "title").map(|(_, v)| v.as_str()).filter(|s| is_safe_title_id(s));
            let (files, bytes) = library::clear_shader_cache(title);
            dirty.store(true, Ordering::Release);
            let msg =
                format!("Cleared {files} shader cache file(s) ({:.1} MiB freed).", (bytes as f64) / (1024.0 * 1024.0));
            send_redirect(&mut stream, &format!("/?msg={}", url_encode(&msg)));
        }
        ("POST", "/restart") => {
            restart_requested.store(true, Ordering::Release);
            let html = format!(
                "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>Restarting TrinityXSX</title><style>{CSS}</style></head><body><h1>Restarting TrinityXSX...</h1><div class=card><p>TrinityXSX is restarting on your PS5 to apply settings (such as Frame Generation).</p><p class=muted>Wait a few seconds, then scan the new QR code / PIN on your TV.</p></div></body></html>"
            );
            send_response(&mut stream, "200 OK", "text/html; charset=utf-8", html.as_bytes(), None);
        }
        ("GET", "/saves.zip") => {
            let title = params.iter().find(|(k, _)| k == "title").map(|(_, v)| v.as_str()).unwrap_or("");
            if !is_safe_title_id(title) {
                send_response(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", b"Invalid title", None);
            } else {
                let zip = build_saves_zip(title);
                let disp = format!("attachment; filename=\"{title}-saves.zip\"");
                send_response(&mut stream, "200 OK", "application/zip", &zip, Some(&disp));
            }
        }
        ("POST", "/saves/restore") => {
            let title = params.iter().find(|(k, _)| k == "title").map(|(_, v)| v.as_str()).unwrap_or("");
            if !is_safe_title_id(title) {
                send_response(&mut stream, "400 Bad Request", "text/plain; charset=utf-8", b"Invalid title", None);
            } else {
                match restore_saves_zip(title, body) {
                    Ok(count) => {
                        dirty.store(true, Ordering::Release);
                        let msg = format!("Restored {count} save file(s) for {title} (previous saves backed up to pls.before-restore).");
                        send_response(&mut stream, "200 OK", "text/plain; charset=utf-8", msg.as_bytes(), None);
                    }
                    Err(e) => {
                        let msg = format!("Save restore failed: {e}");
                        send_response(
                            &mut stream,
                            "400 Bad Request",
                            "text/plain; charset=utf-8",
                            msg.as_bytes(),
                            None,
                        );
                    }
                }
            }
        }
        ("POST", "/preset") => {
            let body_str = String::from_utf8_lossy(body);
            let form = parse_query(&body_str);
            let title = form.iter().find(|(k, _)| k == "title").map(|(_, v)| v.as_str()).unwrap_or("");
            let preset_id = form.iter().find(|(k, _)| k == "preset").map(|(_, v)| v.as_str()).unwrap_or("recommended");
            let paths = library::read_paths(paths_file).unwrap_or_default();
            let catalog = library::scan_paths(&paths);
            let msg = if let Some(game) = catalog.games.iter().find(|g| g.identity == title) {
                let (cur, _) = library::load_game_config(game);
                let updated = library::apply_named_preset(game, &cur, preset_id);
                match library::save_game_config(game, &updated) {
                    Ok(()) => {
                        dirty.store(true, Ordering::Release);
                        format!("Applied preset '{preset_id}' to {title}.")
                    }
                    Err(e) => format!("Cannot save preset: {e}"),
                }
            } else {
                "Game not found in library.".into()
            };
            send_redirect(&mut stream, &format!("/?msg={}", url_encode(&msg)));
        }
        ("POST", "/paths") => {
            let body_str = String::from_utf8_lossy(body);
            let form = parse_query(&body_str);
            let raw = form.iter().find(|(k, _)| k == "paths").map(|(_, v)| v.as_str()).unwrap_or("");
            let mut next = Vec::new();
            for line in raw.lines() {
                let _ = library::push_unique_path(&mut next, line);
            }
            let msg = match library::write_paths(paths_file, &next) {
                Ok(()) => {
                    dirty.store(true, Ordering::Release);
                    format!("Saved {} game folder(s).", next.len())
                }
                Err(e) => format!("Error saving game_paths.txt: {e}"),
            };
            send_redirect(&mut stream, &format!("/?msg={}", url_encode(&msg)));
        }
        ("POST", "/ui") => {
            let body_str = String::from_utf8_lossy(body);
            let form = parse_query(&body_str);
            let ui_file = library::ui_settings_file();
            let mut ui = library::load_ui_settings(&ui_file);
            if let Some((_, lang_str)) = form.iter().find(|(k, _)| k == "lang") {
                if let Some(lang) = crate::library_i18n::Lang::parse(lang_str) {
                    ui.language = lang;
                }
            }
            if let Some((_, idx_str)) = form.iter().find(|(k, _)| k == "profile") {
                if let Ok(idx) = idx_str.parse::<usize>() {
                    if idx < ui.profiles.len() {
                        ui.active_profile = idx;
                    }
                }
            }
            let mut msg = String::new();
            if let Some((_, new_name)) = form.iter().find(|(k, _)| k == "new_profile") {
                if !new_name.trim().is_empty() {
                    if let Err(err) = ui.add_profile(new_name) {
                        msg = err.into();
                    }
                }
            }
            let mut fg_saved = None;
            if let Some((_, fg_str)) = form.iter().find(|(k, _)| k == "framegen") {
                let factor: u8 = match fg_str.trim().to_ascii_lowercase().as_str() {
                    "2" => 2,
                    "3" => 3,
                    "4" => 4,
                    _ => 0,
                };
                if let Err(e) = library::save_framegen_setting(&library::runtime_profile_file(), factor) {
                    msg = format!("Cannot save framegen setting: {e}");
                } else {
                    fg_saved = Some(factor);
                }
            }
            if msg.is_empty() {
                if let Some((_, fps_str)) = form.iter().find(|(k, _)| k == "show_fps") {
                    let enabled = !matches!(fps_str.trim().to_ascii_lowercase().as_str(), "false" | "0" | "off");
                    if let Err(e) = library::save_show_fps_setting(&library::runtime_profile_file(), enabled) {
                        msg = format!("Cannot save show_fps setting: {e}");
                    }
                }
            }
            if msg.is_empty() {
                msg = match library::save_ui_settings(&ui_file, &ui) {
                    Ok(()) => {
                        dirty.store(true, Ordering::Release);
                        let fg_note = match fg_saved {
                            Some(n) if (2..=4).contains(&n) => format!(" · FrameGen {n}x (restart to apply)"),
                            Some(_) => " · FrameGen Off".into(),
                            None => String::new(),
                        };
                        format!(
                            "Saved profile '{}' & language {}{}.",
                            ui.current_profile().name,
                            ui.language.display_name(),
                            fg_note
                        )
                    }
                    Err(e) => format!("Cannot save ui_settings.toml: {e}"),
                };
            }
            send_redirect(&mut stream, &format!("/?msg={}", url_encode(&msg)));
        }
        ("POST", "/config") => {
            let body_str = String::from_utf8_lossy(body);
            let form = parse_query(&body_str);
            let title = form.iter().find(|(k, _)| k == "title").map(|(_, v)| v.as_str()).unwrap_or("");
            let raw_toml = form.iter().find(|(k, _)| k == "toml").map(|(_, v)| v.as_str()).unwrap_or("");
            let msg = if !is_safe_title_id(title) {
                "Invalid title folder name.".into()
            } else {
                match toml::from_str::<Config>(raw_toml) {
                    Ok(_) => {
                        let cfg_path = library::config_path_for_title(title);
                        if let Some(parent) = cfg_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        match fs::write(&cfg_path, raw_toml.replace("\r\n", "\n")) {
                            Ok(()) => {
                                dirty.store(true, Ordering::Release);
                                format!("Saved xcl.toml for {title}.")
                            }
                            Err(e) => format!("Write failed: {e}"),
                        }
                    }
                    Err(e) => format!("Invalid TOML (not saved): {e}"),
                }
            };
            send_redirect(&mut stream, &format!("/?msg={}", url_encode(&msg)));
        }
        _ => {
            send_response(&mut stream, "404 Not Found", "text/plain; charset=utf-8", b"Not Found", None);
        }
    }
}

struct HttpRequest {
    method: String,
    target: String,
    body: Vec<u8>,
    /// Value of the `txsx_session` cookie, if sent.
    session: Option<String>,
    origin: Option<String>,
    host: Option<String>,
    /// `Sec-Fetch-Site`, sent by current browsers: `same-origin`, `same-site`, `cross-site` or `none`.
    fetch_site: Option<String>,
}

fn cookie_value(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k.trim() == name).then(|| v.trim().to_owned())
    })
}

fn read_http_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 8192];
    let header_end;
    loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
        if buf.len() > 16384 {
            return None;
        }
    }
    let header_text = String::from_utf8_lossy(&buf[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let req_line = lines.next()?;
    let mut parts = req_line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?.to_owned();
    let max_body = if target.starts_with("/saves/restore") { MAX_SAVE_ZIP_BYTES } else { MAX_HTTP_BODY_BYTES };
    let mut content_length = 0usize;
    let (mut session, mut origin, mut host, mut fetch_site) = (None, None, None, None);
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.eq_ignore_ascii_case("Content-Length") {
                content_length = v.trim().parse().unwrap_or(0).min(max_body);
            } else if k.eq_ignore_ascii_case("Cookie") {
                if let Some(value) = cookie_value(v, SESSION_COOKIE) {
                    session = Some(value);
                }
            } else if k.eq_ignore_ascii_case("Origin") {
                origin = Some(v.trim().to_owned());
            } else if k.eq_ignore_ascii_case("Host") {
                host = Some(v.trim().to_owned());
            } else if k.eq_ignore_ascii_case("Sec-Fetch-Site") {
                fetch_site = Some(v.trim().to_ascii_lowercase());
            }
        }
    }
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > header_end + max_body {
            break;
        }
    }
    let body_end = (header_end + content_length).min(buf.len());
    Some(HttpRequest { method, target, body: buf[header_end..body_end].to_vec(), session, origin, host, fetch_site })
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn send_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8], disposition: Option<&str>) {
    match disposition {
        Some(disp) => send_response_with_headers(stream, status, content_type, body, &[("Content-Disposition", disp)]),
        None => send_response_with_headers(stream, status, content_type, body, &[]),
    }
}

fn send_response_with_headers(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) {
    let mut header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in extra_headers {
        header.push_str(&format!("{name}: {value}\r\n"));
    }
    header.push_str("\r\n");
    let _ = stream.write_all(header.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// `Origin` must name this server when present. `Host` is what the browser connected to.
fn origin_allowed(origin: Option<&str>, host: Option<&str>) -> bool {
    match (origin, host) {
        (None, _) => true,
        (Some(origin), Some(host)) => origin.eq_ignore_ascii_case(&format!("http://{host}")),
        (Some(_), None) => false,
    }
}

/// `path?query` without any `pin=` parameter, for the redirect after a PIN is accepted.
fn without_pin(path: &str, query: &str) -> String {
    let rest: Vec<&str> =
        query.split('&').filter(|pair| !pair.is_empty() && pair.split('=').next() != Some("pin")).collect();
    if rest.is_empty() {
        path.to_owned()
    } else {
        format!("{path}?{}", rest.join("&"))
    }
}

fn send_redirect_with_headers(stream: &mut TcpStream, location: &str, extra_headers: &[(&str, &str)]) {
    let mut resp = format!("HTTP/1.1 303 See Other\r\nLocation: {location}\r\nCache-Control: no-store\r\n");
    for (name, value) in extra_headers {
        resp.push_str(&format!("{name}: {value}\r\n"));
    }
    resp.push_str("Content-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn send_redirect(stream: &mut TcpStream, location: &str) {
    let resp =
        format!("HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

fn is_safe_title_id(title: &str) -> bool {
    !title.is_empty()
        && title.len() <= 80
        && title.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
        && !title.contains("..")
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else if b == b' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((h << 4) | l);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn parse_query(input: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in input.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.push((url_decode(k), url_decode(v)));
    }
    out
}

const CSS: &str = "body{font-family:system-ui,-apple-system,sans-serif;background:#0e171d;color:#eef7f8;max-width:860px;margin:0 auto;padding:20px;line-height:1.45}h1{margin:0 0 6px;color:#85e6bd}h2{margin:24px 0 8px;font-size:1.15rem;color:#85e6bd}.card{background:#1c2a31;border-radius:10px;padding:16px;margin:14px 0}.btn{display:inline-block;background:#85e6bd;color:#0e171d;font-weight:700;padding:10px 18px;border-radius:8px;text-decoration:none;border:0;cursor:pointer;font-size:1rem}textarea,input[type=text]{width:100%;box-sizing:border-box;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px;padding:10px;font-family:monospace;font-size:0.9rem}.muted{color:#b2c7cb;font-size:0.9rem}.notice{background:#274e48;border-left:4px solid #85e6bd;padding:10px 14px;border-radius:6px;margin:12px 0}";

fn render_login_page(path: &str, notice: &str) -> String {
    let notice =
        if notice.is_empty() { String::new() } else { format!("<div class=notice>{}</div>", html_escape(notice)) };
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>TrinityXSX</title><style>{CSS}</style></head><body><h1>TrinityXSX</h1><p class=muted>Enter the 6-digit PIN shown on your TV.</p>{notice}<div class=card><form method=GET action='{}'><input type=text name=pin placeholder='6-digit PIN' autofocus required><br><br><button class=btn type=submit>Connect</button></form></div></body></html>",
        html_escape(if path == "/logs.zip" { "/logs.zip" } else { "/" })
    )
}

fn render_dashboard(paths_file: &Path, notice: &str) -> String {
    let paths = library::read_paths(paths_file).unwrap_or_default();
    let catalog = library::scan_paths(&paths);
    let storage_items = library::storage_space_summary();
    let storage_line = if storage_items.is_empty() {
        String::new()
    } else {
        let gib = 1024.0 * 1024.0 * 1024.0;
        let parts = storage_items
            .iter()
            .map(|s| {
                format!(
                    "<code>{}</code> {:.1} / {:.1} GB free",
                    html_escape(&s.label),
                    (s.free_bytes as f64) / gib,
                    (s.total_bytes as f64) / gib
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        format!("<p class=muted>Storage: {parts}</p>")
    };
    let mut html = format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>TrinityXSX Web Companion</title><style>{CSS}</style></head><body><h1>TrinityXSX {}</h1><p class=muted>PS5 Web Companion &amp; Log Collector</p>{storage_line}",
        html_escape(library::TRINITYXSX_VERSION)
    );
    if !notice.is_empty() {
        html.push_str(&format!("<div class=notice>{}</div>", html_escape(notice)));
    }
    html.push_str(
        "<div class=card><h2>Diagnostic Logs &amp; Live Viewer</h2><p class=muted>Downloads only logs and <code>xcl.toml</code> configs (never game files or saves).</p><a class=btn href='/logs.zip'>Download trinityxsx-logs.zip</a>\
         <div style='margin-top:14px;display:flex;gap:8px;flex-wrap:wrap;align-items:center'>\
         <select id=logsel onchange='loadLog()' style='padding:8px;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px'>\
         <option value='session'>Latest PS5 session log</option>\
         <option value='xcl'>Latest game xcl.log</option>\
         <option value='startup'>Launcher startup.log</option>\
         <option value='vkd3d'>vkd3d-proton log</option>\
         </select>\
         <button class=btn style='background:#274e48;color:#eef7f8;padding:8px 14px;font-size:0.9rem' type=button onclick='loadLog()'>Refresh</button>\
         <label class=muted><input type=checkbox id=logauto onchange='toggleAutoLog()'> Auto-refresh (3s)</label>\
         </div>\
         <pre id=logbox style='margin:10px 0 0;padding:12px;background:#0e171d;border:1px solid #35505b;border-radius:6px;max-height:280px;overflow:auto;font-size:0.8rem;white-space:pre-wrap;word-break:break-word'>Click Refresh to view recent log lines...</pre></div>"
    );

    // Offline Profile, Language & Frame Generation card
    let ui = library::load_ui_settings(&library::ui_settings_file());
    let profile_path = library::runtime_profile_file();
    let fg = library::read_framegen_setting(&profile_path);
    let show_fps = library::read_show_fps_setting(&profile_path);
    let mut lang_opts = String::new();
    for l in crate::library_i18n::Lang::ALL {
        let sel = if l == ui.language { " selected" } else { "" };
        lang_opts.push_str(&format!("<option value='{}'{sel}>{}</option>", l.code(), l.display_name()));
    }
    let mut prof_opts = String::new();
    for (idx, p) in ui.profiles.iter().enumerate() {
        let sel = if idx == ui.active_profile { " selected" } else { "" };
        prof_opts.push_str(&format!(
            "<option value='{idx}'{sel}>{} (XUID {})</option>",
            html_escape(&p.name),
            p.user_id
        ));
    }
    let mut fg_opts = String::new();
    for (val, label) in [(0u8, "Off"), (2, "2x (30 -> 60 FPS)"), (3, "3x (20 -> 60 FPS)"), (4, "4x (15 -> 60 FPS)")] {
        let sel = if fg == val { " selected" } else { "" };
        let key = if val == 0 { "off".to_string() } else { val.to_string() };
        fg_opts.push_str(&format!("<option value='{key}'{sel}>{label}</option>"));
    }
    let fps_opts = format!(
        "<option value='true'{}>On (DXVK / FrameGen HUD)</option><option value='false'{}>Off</option>",
        if show_fps { " selected" } else { "" },
        if !show_fps { " selected" } else { "" }
    );
    let (cache_files, cache_bytes) = library::shader_cache_summary(None);
    html.push_str(&format!(
        "<div class=card><h2>Profile, Language &amp; Display</h2><form method=POST action='/ui'><label class=muted>Language:</label><br><select name=lang style='width:100%;padding:8px;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px'>{lang_opts}</select><br><br><label class=muted>Active offline profile:</label><br><select name=profile style='width:100%;padding:8px;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px'>{prof_opts}</select><br><br><label class=muted>Or create a new gamertag (1-15 letters/digits):</label><br><input type=text name=new_profile maxlength=15 placeholder='Optional new gamertag'><br><br><label class=muted>Frame Generation (MihawkGen — Vulkan / DXVK / D3D12):</label><br><select name=framegen style='width:100%;padding:8px;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px'>{fg_opts}</select><br><br><label class=muted>FPS HUD Overlay (restart to apply):</label><br><select name=show_fps style='width:100%;padding:8px;background:#0e171d;color:#eef7f8;border:1px solid #35505b;border-radius:6px'>{fps_opts}</select><br><br><button class=btn type=submit>Save settings</button></form>\
         <div style='margin-top:12px;display:flex;gap:10px;flex-wrap:wrap'>\
         <form method=POST action='/restart' onsubmit=\"return confirm('Restart TrinityXSX on PS5 now?')\"><button class=btn style='background:#f7c672;color:#0e171d;font-size:0.9rem;padding:8px 14px' type=submit>Restart TrinityXSX now</button></form>\
         <form method=POST action='/clear-cache' onsubmit=\"return confirm('Clear compiled shader cache? Games will recompile shaders on next launch.')\"><button class=btn style='background:#3b5d66;color:#eef7f8;font-size:0.9rem;padding:8px 14px' type=submit>Clear Shader Cache ({cache_files} files, {:.1} MiB)</button></form>\
         </div></div>",
        (cache_bytes as f64) / (1024.0 * 1024.0)
    ));

    // Game folders editor
    let joined_paths = paths.join("\n");
    html.push_str(&format!(
        "<div class=card><h2>Game Folders (<code>game_paths.txt</code>)</h2><p class=muted>One folder per line (e.g. <code>/mnt/ext1/xboxseries</code> or <code>C:\\XCL\\games</code>).</p><form method=POST action='/paths'><textarea name=paths rows=7>{}</textarea><br><br><button class=btn type=submit>Save folders</button></form></div>",
        html_escape(&joined_paths)
    ));

    // Per-game xcl.toml editors, presets & save backup/restore
    html.push_str("<h2>Per-Game Configuration, Presets &amp; Saves</h2>");
    if catalog.games.is_empty() {
        html.push_str("<div class=card><p class=muted>No Xbox Series games detected yet.</p></div>");
    } else {
        for game in &catalog.games {
            let (cfg, has_file) = library::load_game_config(game);
            let toml_text = toml::to_string_pretty(&cfg).unwrap_or_default();
            let (save_files, save_bytes) = library::saves_summary(game);
            let state = if has_file { "Custom xcl.toml" } else { "Default / Preset (not saved yet)" };
            let tested = library::lookup_tested_preset_label(&game.title_id, &game.identity)
                .map(|l| format!(" · <strong>{}</strong>", html_escape(l)))
                .unwrap_or_default();
            let mut preset_btns = String::new();
            for p in library::NAMED_PRESETS {
                preset_btns.push_str(&format!(
                    "<form method=POST action='/preset' style='display:inline-block;margin:4px 6px 4px 0'><input type=hidden name=title value='{}'><input type=hidden name=preset value='{}'><button class=btn style='background:#274e48;color:#eef7f8;font-size:0.82rem;padding:6px 10px' type=submit>{}</button></form>",
                    html_escape(&game.identity),
                    html_escape(p.id),
                    html_escape(p.label),
                ));
            }
            let title_enc = url_encode(&game.identity);
            html.push_str(&format!(
                "<div class=card><strong>{}</strong> <span class=muted>({} · TitleId {} · {} · {} · Saves: {} files, {:.1} MiB{})</span>\
                 <div style='margin:10px 0'><span class=muted>Quick presets:</span><br>{preset_btns}</div>\
                 <div style='margin:10px 0;padding:10px;background:#142027;border-radius:8px'>\
                 <span class=muted>Saves backup &amp; restore (<code>pls/</code>):</span><br>\
                 <a class=btn style='font-size:0.88rem;padding:7px 12px;margin:6px 8px 6px 0' href='/saves.zip?title={title_enc}'>Backup Saves (.zip)</a>\
                 <input type=file accept='.zip' id='zf-{title_enc}' style='color:#b2c7cb;font-size:0.85rem'>\
                 <button class=btn style='background:#3b5d66;color:#eef7f8;font-size:0.88rem;padding:7px 12px;margin-left:6px' type=button onclick=\"restoreSave('{title_enc}')\">Restore Saves (.zip)</button>\
                 </div>\
                 <form method=POST action='/config'><input type=hidden name=title value='{}'><textarea name=toml rows=11>{}</textarea><br><br><button class=btn type=submit>Save xcl.toml</button></form></div>",
                html_escape(&game.name),
                html_escape(&game.identity),
                html_escape(&game.title_id),
                html_escape(&game.version),
                state,
                save_files,
                (save_bytes as f64) / (1024.0 * 1024.0),
                tested,
                html_escape(&game.identity),
                html_escape(&toml_text),
            ));
        }
    }
    html.push_str(
        "<script>let logTimer=null;async function loadLog(){const sel=document.getElementById('logsel');const box=document.getElementById('logbox');if(!sel||!box)return;try{const r=await fetch('/logs/tail?file='+encodeURIComponent(sel.value));if(r.status===401||r.status===403||r.status===429){if(logTimer){clearInterval(logTimer);logTimer=null;}const cb=document.getElementById('logauto');if(cb)cb.checked=false;box.textContent='TrinityXSX restarted with a new PIN (or is locked). Scan the QR code on your TV again.';return;}box.textContent=await r.text();box.scrollTop=box.scrollHeight;}catch(e){box.textContent='Error loading log: '+e;}}function toggleAutoLog(){const cb=document.getElementById('logauto');if(logTimer){clearInterval(logTimer);logTimer=null;}if(cb&&cb.checked){loadLog();logTimer=setInterval(loadLog,3000);}}async function restoreSave(t){const inp=document.getElementById('zf-'+t);if(!inp||!inp.files||!inp.files[0]){alert('Choose a .zip backup file first.');return;}if(!confirm('Restore saves for '+decodeURIComponent(t)+'? Existing saves will be backed up to pls.before-restore.'))return;const r=await fetch('/saves/restore?title='+t,{method:'POST',body:await inp.files[0].arrayBuffer()});if(r.status===401||r.status===403||r.status===429){alert('TrinityXSX restarted with a new PIN (or is locked). Scan the QR code on your TV again.');return;}const txt=await r.text();window.location='/?msg='+encodeURIComponent(txt);}</script></body></html>"
    );
    html
}

fn read_tail_file(path: &Path, max_bytes: usize) -> Option<String> {
    let data = fs::read(path).ok()?;
    let slice = if data.len() > max_bytes { &data[data.len() - max_bytes..] } else { &data[..] };
    Some(String::from_utf8_lossy(slice).into_owned())
}

fn read_log_tail(kind: &str) -> String {
    const TAIL_BYTES: usize = 16 * 1024;
    let ps5_root = Path::new("Z:\\data\\homebrew\\PPSA83601");
    let candidate: Option<PathBuf> = match kind {
        "startup" => Some(ps5_root.join("startup.log")),
        "vkd3d" => Some(PathBuf::from("C:\\XCL\\vkd3d-trinityxsx-library.log")),
        "xcl" => {
            let xcl_root = crate::launcher::data_root();
            fs::read_dir(&xcl_root)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path().join("logs").join("xcl.log"))
                .filter(|p| p.is_file())
                .max_by_key(|p| fs::metadata(p).and_then(|m| m.modified()).ok())
        }
        _ => fs::read_dir(ps5_root.join("logs"))
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("session-") && n.ends_with(".log"))
            })
            .max(),
    };
    match candidate {
        Some(path) => match read_tail_file(&path, TAIL_BYTES) {
            Some(text) if !text.is_empty() => format!("--- {} ---\n{text}", library::display_path(&path)),
            _ => format!("Log file is empty or unreadable: {}", library::display_path(&path)),
        },
        None => "No matching log file found yet.".into(),
    }
}

// ---------------------------------------------------------------------------
// Minimal pure-Rust ZIP builder (DEFLATE via miniz_oxide + IEEE CRC-32)
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if (crc & 1) != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

pub fn build_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();

    for (name, data) in entries {
        let name_bytes = name.as_bytes();
        let offset = out.len() as u32;
        let crc = crc32(data);
        let compressed = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, payload): (u16, &[u8]) = if compressed.len() < data.len() { (8, &compressed) } else { (0, data) };
        let comp_len = payload.len() as u32;
        let uncomp_len = data.len() as u32;

        // Local file header
        out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&comp_len.to_le_bytes());
        out.extend_from_slice(&uncomp_len.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(payload);

        // Central directory header
        central.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&comp_len.to_le_bytes());
        central.extend_from_slice(&uncomp_len.to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attr
        central.extend_from_slice(&0u32.to_le_bytes()); // external attr
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End of central directory
    out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    let data = fs::read(path).ok()?;
    if data.len() <= MAX_LOG_FILE_BYTES {
        return Some(data);
    }
    let mut trimmed = Vec::with_capacity(MAX_LOG_FILE_BYTES + 64);
    trimmed.extend_from_slice(&data[..1024 * 1024]);
    trimmed.extend_from_slice(b"\n... [TRUNCATED BY TRINITYXSX WEB COMPANION] ...\n");
    trimmed.extend_from_slice(&data[data.len() - (MAX_LOG_FILE_BYTES - 1024 * 1024)..]);
    Some(trimmed)
}

pub fn build_logs_zip(paths_file: &Path) -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |arc: String, path: &Path| {
        if let Some(bytes) = read_capped(path) {
            files.push((arc, bytes));
        }
    };

    add("game_paths.txt".into(), paths_file);
    let ps5_root = Path::new("Z:\\data\\homebrew\\PPSA83601");
    if ps5_root.is_dir() {
        add("startup.log".into(), &ps5_root.join("startup.log"));
        add("wineserver-startup.log".into(), &ps5_root.join("wineserver-startup.log"));
        add("diag_dump.log".into(), &ps5_root.join("diag_dump.log"));
        add("hud_stats.txt".into(), &ps5_root.join("hud_stats.txt"));
        add("TRINITYXSX-BUILD.json".into(), &ps5_root.join("TRINITYXSX-BUILD.json"));
        add("logs/next.txt".into(), &ps5_root.join("logs/next.txt"));
        if let Ok(rd) = fs::read_dir(ps5_root.join("logs")) {
            let mut logs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("session-") && n.ends_with(".log"))
                })
                .collect();
            logs.sort();
            for p in logs {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    add(format!("logs/{name}"), &p);
                }
            }
        }
    }
    add("vkd3d-trinityxsx-library.log".into(), Path::new("C:\\XCL\\vkd3d-trinityxsx-library.log"));

    let xcl_root = crate::launcher::data_root();
    if let Ok(rd) = fs::read_dir(&xcl_root) {
        let mut dirs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for dir in dirs {
            let Some(title) = dir.file_name().and_then(|n| n.to_str()) else { continue };
            if title.eq_ignore_ascii_case("dxil") {
                continue;
            }
            add(format!("titles/{title}/xcl.toml"), &dir.join("xcl.toml"));
            add(format!("titles/{title}/logs/xcl.log"), &dir.join("logs/xcl.log"));
            add(format!("titles/{title}/logs/report.json"), &dir.join("logs/report.json"));
        }
    }

    if files.is_empty() {
        files.push(("README.txt".into(), b"No TrinityXSX diagnostic logs found yet.\n".to_vec()));
    }
    build_zip(&files)
}

pub fn build_saves_zip(title: &str) -> Vec<u8> {
    let root = library::saves_dir_for_title(title);
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total_bytes = 0usize;
    if root.is_dir() {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            let mut items: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
            items.sort();
            for path in items {
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    if entries.len() >= MAX_SAVE_FILES {
                        break;
                    }
                    let Ok(rel) = path.strip_prefix(&root) else { continue };
                    let Ok(data) = fs::read(&path) else { continue };
                    if total_bytes.saturating_add(data.len()) > MAX_SAVE_UNCOMPRESSED_BYTES {
                        continue;
                    }
                    total_bytes += data.len();
                    let arc = rel.to_string_lossy().replace('\\', "/");
                    entries.push((arc, data));
                }
            }
        }
    }
    if entries.is_empty() {
        entries.push(("README.txt".into(), format!("No save files found yet for {title}.\n").into_bytes()));
    }
    build_zip(&entries)
}

fn sanitize_zip_rel_path(raw: &str) -> Option<PathBuf> {
    let norm = raw.replace('\\', "/");
    if norm.is_empty() || norm.starts_with('/') || norm.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in norm.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return None,
            part => out.push(part),
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Pure-Rust ZIP reader (Stored + Deflated via miniz_oxide) with CRC-32 and path-traversal checks.
pub fn extract_zip(data: &[u8]) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    if data.len() < 22 {
        return Err("file is too small to be a ZIP archive".into());
    }
    // Locate End of Central Directory (EOCD) record near the end of the archive.
    let search_start = data.len().saturating_sub(65557);
    let eocd_pos = (search_start..=data.len() - 22)
        .rev()
        .find(|&i| &data[i..i + 4] == b"PK\x05\x06")
        .ok_or_else(|| "invalid ZIP archive (missing EOCD)".to_string())?;
    let total_entries = u16::from_le_bytes([data[eocd_pos + 10], data[eocd_pos + 11]]) as usize;
    let cd_size =
        u32::from_le_bytes([data[eocd_pos + 12], data[eocd_pos + 13], data[eocd_pos + 14], data[eocd_pos + 15]])
            as usize;
    let cd_offset =
        u32::from_le_bytes([data[eocd_pos + 16], data[eocd_pos + 17], data[eocd_pos + 18], data[eocd_pos + 19]])
            as usize;
    if total_entries > MAX_SAVE_FILES || cd_offset.saturating_add(cd_size) > data.len() {
        return Err("ZIP central directory out of bounds or too many files".into());
    }

    let mut cursor = cd_offset;
    let cd_end = cd_offset + cd_size;
    let mut out = Vec::new();
    let mut total_uncompressed = 0usize;

    for _ in 0..total_entries {
        if cursor + 46 > cd_end || &data[cursor..cursor + 4] != b"PK\x01\x02" {
            return Err("corrupt ZIP central directory header".into());
        }
        let method = u16::from_le_bytes([data[cursor + 10], data[cursor + 11]]);
        let expected_crc =
            u32::from_le_bytes([data[cursor + 16], data[cursor + 17], data[cursor + 18], data[cursor + 19]]);
        let comp_size =
            u32::from_le_bytes([data[cursor + 20], data[cursor + 21], data[cursor + 22], data[cursor + 23]]) as usize;
        let uncomp_size =
            u32::from_le_bytes([data[cursor + 24], data[cursor + 25], data[cursor + 26], data[cursor + 27]]) as usize;
        let name_len = u16::from_le_bytes([data[cursor + 28], data[cursor + 29]]) as usize;
        let extra_len = u16::from_le_bytes([data[cursor + 30], data[cursor + 31]]) as usize;
        let comment_len = u16::from_le_bytes([data[cursor + 32], data[cursor + 33]]) as usize;
        let lfh_offset =
            u32::from_le_bytes([data[cursor + 42], data[cursor + 43], data[cursor + 44], data[cursor + 45]]) as usize;

        let name_end = cursor + 46 + name_len;
        if name_end > cd_end {
            return Err("truncated ZIP filename".into());
        }
        let raw_name =
            std::str::from_utf8(&data[cursor + 46..name_end]).map_err(|_| "non-UTF-8 filename in ZIP".to_string())?;
        cursor = name_end + extra_len + comment_len;

        if raw_name.ends_with('/') || raw_name.ends_with('\\') {
            continue;
        }
        let rel = sanitize_zip_rel_path(raw_name).ok_or_else(|| format!("unsafe path in ZIP: {raw_name}"))?;
        if lfh_offset + 30 > data.len() || &data[lfh_offset..lfh_offset + 4] != b"PK\x03\x04" {
            return Err("corrupt ZIP local file header".into());
        }
        let lfh_name_len = u16::from_le_bytes([data[lfh_offset + 26], data[lfh_offset + 27]]) as usize;
        let lfh_extra_len = u16::from_le_bytes([data[lfh_offset + 28], data[lfh_offset + 29]]) as usize;
        let payload_start = lfh_offset + 30 + lfh_name_len + lfh_extra_len;
        let payload_end = payload_start.saturating_add(comp_size);
        if payload_end > data.len() {
            return Err("truncated ZIP entry payload".into());
        }
        let payload = &data[payload_start..payload_end];
        let inflated = match method {
            0 => payload.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(payload, MAX_SAVE_UNCOMPRESSED_BYTES)
                .map_err(|e| format!("deflate error in {raw_name}: {e:?}"))?,
            m => return Err(format!("unsupported ZIP compression method {m} in {raw_name}")),
        };
        if inflated.len() != uncomp_size || crc32(&inflated) != expected_crc {
            return Err(format!("CRC/size mismatch for {raw_name}"));
        }
        total_uncompressed = total_uncompressed.saturating_add(inflated.len());
        if total_uncompressed > MAX_SAVE_UNCOMPRESSED_BYTES {
            return Err("ZIP exceeds 64 MiB uncompressed safety limit".into());
        }
        out.push((rel, inflated));
    }
    Ok(out)
}

pub fn restore_saves_zip(title: &str, zip_bytes: &[u8]) -> Result<usize, String> {
    let entries = extract_zip(zip_bytes)?;
    if entries.is_empty() {
        return Err("ZIP archive contains no files".into());
    }
    let pls_dir = library::saves_dir_for_title(title);
    let backup_dir = pls_dir.with_file_name("pls.before-restore");
    if pls_dir.is_dir() {
        let _ = fs::remove_dir_all(&backup_dir);
        fs::rename(&pls_dir, &backup_dir).map_err(|e| format!("backup failed: {e}"))?;
    }
    fs::create_dir_all(&pls_dir).map_err(|e| format!("mkdir pls failed: {e}"))?;
    for (rel, bytes) in &entries {
        let dst = pls_dir.join(rel);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
        }
        fs::write(&dst, bytes).map_err(|e| format!("write {} failed: {e}", rel.display()))?;
    }
    Ok(entries.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_and_extracts_zip_roundtrip() {
        let input = vec![
            ("worlds/test/level.dat".into(), b"TrinityXSX save data 0123456789 repeated repeated repeated".to_vec()),
            ("small.bin".into(), vec![1, 2, 3, 4]),
        ];
        let zip = build_zip(&input);
        assert_eq!(&zip[0..4], b"PK\x03\x04");
        let extracted = extract_zip(&zip).unwrap();
        assert_eq!(extracted.len(), 2);
        assert_eq!(extracted[0].0, PathBuf::from("worlds/test/level.dat"));
        assert_eq!(extracted[0].1, input[0].1);
        assert_eq!(extracted[1].1, input[1].1);
    }

    #[test]
    fn rejects_zip_path_traversal() {
        let evil = build_zip(&[("../escape.txt".into(), b"nope".to_vec())]);
        assert!(extract_zip(&evil).is_err());
        assert!(sanitize_zip_rel_path("/etc/passwd").is_none());
        assert!(sanitize_zip_rel_path("C:\\Windows\\evil").is_none());
    }

    #[test]
    fn pin_guard_locks_out_after_free_attempts_and_backs_off() {
        let mut guard = PinGuard::default();
        let t0 = Instant::now();
        let mut n = 0u32;
        let mut next_wrong = || {
            n += 1;
            format!("{:06}", n)
        };
        for _ in 1..PIN_FREE_ATTEMPTS {
            assert!(matches!(guard.check(&next_wrong(), "123456", t0), PinCheck::Rejected));
        }
        assert!(matches!(guard.check("123456", "123456", t0), PinCheck::Accepted));
        // The last free attempt starts the first lockout.
        assert!(matches!(guard.check(&next_wrong(), "123456", t0), PinCheck::LockedOut(d) if d == PIN_BASE_LOCKOUT));
        // While locked even the right PIN is not checked, so a guesser learns nothing.
        assert!(matches!(guard.check("123456", "123456", t0 + Duration::from_secs(29)), PinCheck::LockedOut(_)));
        let t1 = t0 + PIN_BASE_LOCKOUT;
        assert!(matches!(guard.check("123456", "123456", t1), PinCheck::Accepted));
        // A correct PIN does not reset the count: the next wrong one doubles the lockout.
        assert!(
            matches!(guard.check(&next_wrong(), "123456", t1), PinCheck::LockedOut(d) if d == PIN_BASE_LOCKOUT * 2)
        );
        let mut t = t1;
        for _ in 0..40 {
            t += PIN_MAX_LOCKOUT;
            assert!(matches!(guard.check(&next_wrong(), "123456", t), PinCheck::LockedOut(d) if d <= PIN_MAX_LOCKOUT));
        }
        t += PIN_MAX_LOCKOUT;
        assert!(matches!(guard.check(&next_wrong(), "123456", t), PinCheck::LockedOut(d) if d == PIN_MAX_LOCKOUT));
    }

    #[test]
    fn pin_guard_counts_a_repeated_stale_pin_once() {
        // A dashboard tab from before a restart polls every 3 s with the old PIN.
        let mut guard = PinGuard::default();
        let t0 = Instant::now();
        for i in 0..1000u64 {
            let t = t0 + Duration::from_secs(3 * i);
            assert!(matches!(guard.check("111111", "123456", t), PinCheck::Rejected));
        }
        assert_eq!(guard.failures, 1);
        assert!(matches!(guard.check("123456", "123456", t0 + Duration::from_secs(3000)), PinCheck::Accepted));
    }

    #[test]
    fn constant_time_eq_matches_only_equal_input() {
        assert!(constant_time_eq(b"123456", b"123456"));
        assert!(!constant_time_eq(b"123456", b"123457"));
        assert!(!constant_time_eq(b"12345", b"123456"));
        assert!(!constant_time_eq(b"", b"123456"));
    }

    #[test]
    fn qr_code_logs_in_with_the_pin() {
        assert_eq!(qr_login_url("http://192.168.1.5:8360", "042137"), "http://192.168.1.5:8360/?pin=042137");
    }

    #[test]
    fn session_tokens_are_long_and_distinct() {
        let a = generate_session_token();
        let b = generate_session_token();
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn cookie_value_finds_the_session_among_others() {
        assert_eq!(cookie_value(" a=1; txsx_session=abc ; b=2", SESSION_COOKIE).as_deref(), Some("abc"));
        assert_eq!(cookie_value("txsx_session_old=x", SESSION_COOKIE), None);
        assert_eq!(cookie_value("", SESSION_COOKIE), None);
    }

    #[test]
    fn redirect_after_pin_drops_only_the_pin() {
        assert_eq!(without_pin("/", "pin=123456"), "/");
        assert_eq!(without_pin("/logs.zip", "pin=123456"), "/logs.zip");
        assert_eq!(without_pin("/", "msg=Saved%21&pin=1&x=2"), "/?msg=Saved%21&x=2");
        assert_eq!(without_pin("/", "pinned=1"), "/?pinned=1");
    }

    #[test]
    fn origin_must_match_host_when_sent() {
        assert!(origin_allowed(None, Some("192.168.1.5:8360")));
        assert!(origin_allowed(Some("http://192.168.1.5:8360"), Some("192.168.1.5:8360")));
        assert!(!origin_allowed(Some("http://evil.example"), Some("192.168.1.5:8360")));
        assert!(!origin_allowed(Some("null"), Some("192.168.1.5:8360")));
        assert!(!origin_allowed(Some("http://192.168.1.5:8360"), None));
    }

    #[test]
    fn dashboard_links_carry_no_pin() {
        let html = render_dashboard(Path::new("does-not-exist/game_paths.txt"), "");
        assert!(!html.contains("pin="), "PIN leaked into dashboard HTML");
    }

    /// One request through `handle_connection` over loopback; returns the raw response.
    fn roundtrip(request: &str, session: &str, guard: &mut PinGuard, paths_file: &Path) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let request = request.replace("{addr}", &addr.to_string());
        let client = thread::spawn(move || {
            let mut c = TcpStream::connect(addr).unwrap();
            c.write_all(request.as_bytes()).unwrap();
            let mut out = Vec::new();
            let _ = c.read_to_end(&mut out);
            String::from_utf8_lossy(&out).into_owned()
        });
        let (stream, _) = listener.accept().unwrap();
        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
        let (dirty, restart) = (AtomicBool::new(false), AtomicBool::new(false));
        handle_connection(stream, "123456", session, guard, paths_file, &dirty, &restart);
        client.join().unwrap()
    }

    #[test]
    fn http_pin_is_traded_for_a_cookie_and_cross_site_posts_are_refused() {
        let dir = std::env::temp_dir().join(format!("txsx-web-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let paths_file = dir.join("game_paths.txt");
        let session = "0123456789abcdef0123456789abcdef";
        let mut guard = PinGuard::default();

        // QR link: the PIN is accepted once, set as a cookie, and dropped from the URL.
        let r = roundtrip("GET /?pin=123456&msg=hi HTTP/1.1\r\nHost: {addr}\r\n\r\n", session, &mut guard, &paths_file);
        assert!(r.starts_with("HTTP/1.1 303"), "{r}");
        assert!(r.contains("\r\nLocation: /?msg=hi\r\n"), "{r}");
        assert!(r.contains(&format!("Set-Cookie: txsx_session={session}; Path=/; HttpOnly; SameSite=Lax")), "{r}");

        // With the cookie the dashboard loads, and its links carry no PIN.
        let r = roundtrip(
            &format!("GET / HTTP/1.1\r\nHost: {{addr}}\r\nCookie: other=1; txsx_session={session}\r\n\r\n"),
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.starts_with("HTTP/1.1 200"), "{r}");
        assert!(r.contains("Web Companion") && !r.contains("pin="), "{r}");

        // Another site linking to the dashboard cannot put its own text in the notice banner.
        let r = roundtrip(
            &format!(
                "GET /?msg=Download+the+fix+from+evil.example HTTP/1.1\r\nHost: {{addr}}\r\nSec-Fetch-Site: cross-site\r\nCookie: txsx_session={session}\r\n\r\n"
            ),
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.starts_with("HTTP/1.1 200") && !r.contains("evil.example"), "{r}");
        let r = roundtrip(
            &format!(
                "GET /?msg=Saved+3+folders HTTP/1.1\r\nHost: {{addr}}\r\nSec-Fetch-Site: same-origin\r\nCookie: txsx_session={session}\r\n\r\n"
            ),
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.contains("Saved 3 folders"), "{r}");

        // A stale cookie (from before a restart) gets 401 on the log poll and is not counted.
        let r = roundtrip(
            "GET /logs/tail HTTP/1.1\r\nHost: {addr}\r\nCookie: txsx_session=stale\r\n\r\n",
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.starts_with("HTTP/1.1 401"), "{r}");
        assert_eq!(guard.failures, 0);

        // A form posted from another site is refused even with a valid cookie, and writes nothing.
        let body = "paths=%2Fevil";
        let r = roundtrip(
            &format!(
                "POST /paths HTTP/1.1\r\nHost: {{addr}}\r\nOrigin: http://evil.example\r\nCookie: txsx_session={session}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.starts_with("HTTP/1.1 403"), "{r}");
        assert!(!paths_file.exists());

        // The same form from the dashboard itself goes through.
        let r = roundtrip(
            &format!(
                "POST /paths HTTP/1.1\r\nHost: {{addr}}\r\nOrigin: http://{{addr}}\r\nCookie: txsx_session={session}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            session,
            &mut guard,
            &paths_file,
        );
        assert!(r.starts_with("HTTP/1.1 303"), "{r}");
        assert!(r.contains("\r\nLocation: /?msg="), "{r}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn generated_pin_is_six_digits() {
        for _ in 0..32 {
            let pin = generate_pin();
            assert_eq!(pin.len(), 6);
            assert!(pin.bytes().all(|b| b.is_ascii_digit()));
        }
    }

    #[test]
    fn url_roundtrip_and_title_safety() {
        let s = "Saved 2 folder(s): /mnt/ext1/xboxseries";
        assert_eq!(url_decode(&url_encode(s)), s);
        assert!(is_safe_title_id("Microsoft.MinecraftBeta"));
        assert!(!is_safe_title_id("../escape"));
        assert!(!is_safe_title_id("a/b"));
    }
}
