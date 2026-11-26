//! txin-sender: this PC's keyboard and mouse to TrinityXSX on a PS5, over the LAN.
//!
//!     txin-sender <PS5 address> [--port 9396]     capture with Ctrl+Alt+F12, stop with Ctrl+Alt+F12
//!     txin-sender <PS5 address> --test            no capture: hold W for a moment and move the mouse in a circle
//!
//! Low-level keyboard and mouse hooks see every event; while capturing they hide it from Windows and the
//! state goes to the PS5 (protocol.rs). Mouse motion comes from Raw Input when Windows delivers it while the
//! hook hides the event, and otherwise from the hook's would-be cursor position against the cursor, which
//! stays where capture began.

mod capture;
mod protocol;

use capture::{Capture, Outcome, Verdict};
use std::cell::RefCell;
use std::ffi::c_void;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Msg {
    hwnd: isize,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: Point,
    private: u32,
}

#[repr(C)]
struct KbdLlHook {
    vk: u32,
    scan: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
struct MsLlHook {
    pt: Point,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra: usize,
}

#[repr(C)]
struct WndClassW {
    style: u32,
    wndproc: unsafe extern "system" fn(isize, u32, usize, isize) -> isize,
    cls_extra: i32,
    wnd_extra: i32,
    instance: isize,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
struct RawInputDevice {
    usage_page: u16,
    usage: u16,
    flags: u32,
    target: isize,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32")]
extern "system" {
    fn SetWindowsHookExW(id: i32, proc_: HookProc, module: isize, thread: u32) -> isize;
    fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
    fn UnhookWindowsHookEx(hook: isize) -> i32;
    fn GetMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32) -> i32;
    fn TranslateMessage(msg: *const Msg) -> i32;
    fn DispatchMessageW(msg: *const Msg) -> isize;
    fn GetCursorPos(point: *mut Point) -> i32;
    fn RegisterClassW(class: *const WndClassW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: isize,
        menu: isize,
        instance: isize,
        param: *mut c_void,
    ) -> isize;
    fn DefWindowProcW(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize;
    fn RegisterRawInputDevices(devices: *const RawInputDevice, count: u32, size: u32) -> i32;
    fn GetRawInputData(input: isize, command: u32, data: *mut c_void, size: *mut u32, header: u32) -> u32;
    fn SetTimer(hwnd: isize, id: usize, ms: u32, proc_: *const c_void) -> usize;
    fn PostThreadMessageW(thread: u32, msg: u32, wparam: usize, lparam: isize) -> i32;
    fn MessageBeep(kind: u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> isize;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn SetConsoleCtrlHandler(handler: unsafe extern "system" fn(u32) -> i32, add: i32) -> i32;
}

const WH_KEYBOARD_LL: i32 = 13;
const WH_MOUSE_LL: i32 = 14;
const WM_QUIT: u32 = 0x0012;
const WM_TIMER: u32 = 0x0113;
const WM_INPUT: u32 = 0x00ff;
const WM_KEYDOWN: usize = 0x0100;
const WM_SYSKEYDOWN: usize = 0x0104;
const WM_MOUSEMOVE: usize = 0x0200;
const WM_LBUTTONDOWN: usize = 0x0201;
const WM_LBUTTONUP: usize = 0x0202;
const WM_RBUTTONDOWN: usize = 0x0204;
const WM_RBUTTONUP: usize = 0x0205;
const WM_MBUTTONDOWN: usize = 0x0207;
const WM_MBUTTONUP: usize = 0x0208;
const WM_MOUSEWHEEL: usize = 0x020a;
const WM_XBUTTONDOWN: usize = 0x020b;
const WM_XBUTTONUP: usize = 0x020c;
const WM_MOUSEHWHEEL: usize = 0x020e;
const HWND_MESSAGE: isize = -3;
const RIDEV_INPUTSINK: u32 = 0x100;
const RID_INPUT: u32 = 0x1000_0003;
const RIM_TYPEMOUSE: u32 = 0;
const MOUSE_MOVE_ABSOLUTE: u16 = 1;
const HEARTBEAT: Duration = Duration::from_millis(50);

struct App {
    capture: Capture,
    socket: UdpSocket,
    session: u32,
    sequence: u32,
    anchor: Point,
    raw_seen: bool,
    sent: u64,
    last_sent: Instant,
}

impl App {
    fn send(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        let bytes = self.capture.packet(self.session, self.sequence).encode();
        if self.socket.send(&bytes).is_ok() {
            self.sent += 1;
        }
        self.last_sent = Instant::now();
    }

    fn apply(&mut self, outcome: Outcome) -> Verdict {
        if outcome.toggled {
            if self.capture.on {
                let mut p = Point::default();
                unsafe { GetCursorPos(&mut p) };
                self.anchor = p;
                self.raw_seen = false;
                println!("capture ON  - the keyboard and mouse go to the PS5 (Ctrl+Alt+F12 stops)");
            } else {
                println!("capture OFF - the keyboard and mouse are this PC's again ({} datagrams sent)", self.sent);
            }
            unsafe { MessageBeep(0) };
        }
        if outcome.changed || outcome.toggled {
            self.send();
            if outcome.toggled && !self.capture.on {
                self.send(); // two more: the release must arrive
                self.send();
            }
        }
        outcome.verdict
    }
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static HOOKS: RefCell<(isize, isize)> = const { RefCell::new((0, 0)) };
}
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

fn with_app(f: impl FnOnce(&mut App) -> Verdict) -> Verdict {
    APP.with(|a| a.borrow_mut().as_mut().map(f).unwrap_or(Verdict::Pass))
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 {
        let k = &*(lparam as *const KbdLlHook);
        let down = wparam == WM_KEYDOWN || wparam == WM_SYSKEYDOWN;
        if with_app(|app| {
            let o = app.capture.key(k.vk as u8, down);
            app.apply(o)
        }) == Verdict::Swallow
        {
            return 1;
        }
    }
    CallNextHookEx(HOOKS.with(|h| h.borrow().0), code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 {
        let m = &*(lparam as *const MsLlHook);
        let verdict = with_app(|app| {
            let outcome = match wparam {
                WM_MOUSEMOVE => {
                    let (dx, dy) = if app.raw_seen { (0, 0) } else { (m.pt.x - app.anchor.x, m.pt.y - app.anchor.y) };
                    app.capture.motion(dx, dy)
                }
                WM_LBUTTONDOWN | WM_LBUTTONUP => app.capture.button(0, wparam == WM_LBUTTONDOWN),
                WM_RBUTTONDOWN | WM_RBUTTONUP => app.capture.button(1, wparam == WM_RBUTTONDOWN),
                WM_MBUTTONDOWN | WM_MBUTTONUP => app.capture.button(2, wparam == WM_MBUTTONDOWN),
                WM_XBUTTONDOWN | WM_XBUTTONUP => app.capture.button(3, wparam == WM_XBUTTONDOWN),
                WM_MOUSEWHEEL => app.capture.wheel((m.mouse_data >> 16) as u16 as i16 as i32),
                WM_MOUSEHWHEEL => app.capture.wheel(0),
                _ => return Verdict::Pass,
            };
            app.apply(outcome)
        });
        if verdict == Verdict::Swallow {
            return 1;
        }
    }
    CallNextHookEx(HOOKS.with(|h| h.borrow().1), code, wparam, lparam)
}

unsafe extern "system" fn window_proc(hwnd: isize, msg: u32, wparam: usize, lparam: isize) -> isize {
    match msg {
        WM_INPUT => {
            let mut buffer = [0u8; 64];
            let mut size = buffer.len() as u32;
            let header = (4 + 4 + 2 * std::mem::size_of::<usize>()) as u32;
            let got = GetRawInputData(lparam, RID_INPUT, buffer.as_mut_ptr().cast(), &mut size, header);
            let at = header as usize;
            if got != u32::MAX && got as usize >= at + 24 {
                let kind = u32::from_le_bytes(buffer[0..4].try_into().unwrap());
                let flags = u16::from_le_bytes(buffer[at..at + 2].try_into().unwrap());
                let dx = i32::from_le_bytes(buffer[at + 12..at + 16].try_into().unwrap());
                let dy = i32::from_le_bytes(buffer[at + 16..at + 20].try_into().unwrap());
                if kind == RIM_TYPEMOUSE && flags & MOUSE_MOVE_ABSOLUTE == 0 && (dx != 0 || dy != 0) {
                    with_app(|app| {
                        if app.capture.on {
                            app.raw_seen = true;
                            let o = app.capture.motion(dx, dy);
                            app.apply(o);
                        }
                        Verdict::Pass
                    });
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_TIMER => {
            with_app(|app| {
                if app.capture.on && app.last_sent.elapsed() >= HEARTBEAT {
                    app.send();
                }
                Verdict::Pass
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe extern "system" fn console_ctrl(_kind: u32) -> i32 {
    PostThreadMessageW(MAIN_THREAD.load(Ordering::SeqCst), WM_QUIT, 0, 0);
    1
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn session_id() -> u32 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    (nanos as u32) ^ ((nanos >> 32) as u32) ^ unsafe { GetCurrentProcessId() }.rotate_left(16)
}

fn usage() -> ! {
    eprintln!("usage: txin-sender <PS5 address> [--port {}] [--test]", protocol::PORT);
    eprintln!("  Ctrl+Alt+F12 starts and stops sending this PC's keyboard and mouse to the PS5.");
    eprintln!("  --test sends a short pattern (W held, the mouse in a circle) without capturing anything.");
    std::process::exit(2);
}

fn connect(host: &str, port: u16) -> std::io::Result<(UdpSocket, SocketAddr)> {
    let target = (host, port)
        .to_socket_addrs()?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no IPv4 address"))?;
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect(target)?;
    Ok((socket, target))
}

fn test_pattern(socket: UdpSocket, session: u32) {
    let mut app = App {
        capture: Capture::default(),
        socket,
        session,
        sequence: 0,
        anchor: Point::default(),
        raw_seen: false,
        sent: 0,
        last_sent: Instant::now(),
    };
    // Capture on as the shortcut would (its Ctrl and Alt are Windows'), then W for 0.3 s and a 2 s circle.
    app.capture.key(0xa2, true);
    app.capture.key(0xa4, true);
    app.capture.key(capture::VK_F12, true);
    app.capture.key(0xa2, false);
    app.capture.key(0xa4, false);
    app.capture.key(capture::VK_F12, false);
    app.send();
    let o = app.capture.key(b'W', true);
    app.apply(o);
    std::thread::sleep(Duration::from_millis(300));
    let o = app.capture.key(b'W', false);
    app.apply(o);
    let (mut x, mut y) = (0i32, 0i32);
    for step in 1..=200 {
        let angle = step as f64 / 200.0 * std::f64::consts::TAU;
        let (nx, ny) = ((angle.cos() * 150.0 - 150.0) as i32, (angle.sin() * 150.0) as i32);
        let o = app.capture.motion(nx - x, ny - y);
        (x, y) = (nx, ny);
        app.apply(o);
        std::thread::sleep(Duration::from_millis(10));
    }
    app.capture.key(0xa2, true);
    app.capture.key(0xa4, true);
    let o = app.capture.key(capture::VK_F12, true);
    app.apply(o);
    println!("test pattern sent: {} datagrams, session {:08x}", app.sent, session);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut host = None;
    let mut port = protocol::PORT;
    let mut test = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--port" => port = args.next().and_then(|p| p.parse().ok()).unwrap_or_else(|| usage()),
            "--test" => test = true,
            "-h" | "--help" => usage(),
            _ if host.is_none() && !a.starts_with('-') => host = Some(a),
            _ => usage(),
        }
    }
    let host = host.unwrap_or_else(|| usage());
    let (socket, target) = connect(&host, port).unwrap_or_else(|e| {
        eprintln!("txin-sender: {host}:{port}: {e}");
        std::process::exit(1);
    });
    let session = session_id();
    if test {
        test_pattern(socket, session);
        return;
    }

    unsafe {
        MAIN_THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("TrinityXSXNetInput");
        let wc = WndClassW {
            style: 0,
            wndproc: window_proc,
            cls_extra: 0,
            wnd_extra: 0,
            instance,
            icon: 0,
            cursor: 0,
            background: 0,
            menu_name: std::ptr::null(),
            class_name: class.as_ptr(),
        };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            0,
            instance,
            std::ptr::null_mut(),
        );
        let mouse = RawInputDevice { usage_page: 1, usage: 2, flags: RIDEV_INPUTSINK, target: hwnd };
        if hwnd == 0 || RegisterRawInputDevices(&mouse, 1, std::mem::size_of::<RawInputDevice>() as u32) == 0 {
            eprintln!("txin-sender: Raw Input is unavailable; mouse motion comes from the hook alone");
        }
        SetTimer(hwnd, 1, HEARTBEAT.as_millis() as u32, std::ptr::null());

        APP.with(|a| {
            *a.borrow_mut() = Some(App {
                capture: Capture::default(),
                socket,
                session,
                sequence: 0,
                anchor: Point::default(),
                raw_seen: false,
                sent: 0,
                last_sent: Instant::now(),
            })
        });
        let kb = SetWindowsHookExW(WH_KEYBOARD_LL, keyboard_hook, instance, 0);
        let ms = SetWindowsHookExW(WH_MOUSE_LL, mouse_hook, instance, 0);
        if kb == 0 || ms == 0 {
            eprintln!("txin-sender: could not install the keyboard and mouse hooks");
            std::process::exit(1);
        }
        HOOKS.with(|h| *h.borrow_mut() = (kb, ms));
        SetConsoleCtrlHandler(console_ctrl, 1);

        println!("txin-sender: PS5 at {target}, session {session:08x}");
        println!("Ctrl+Alt+F12 starts and stops capturing; Ctrl+C here quits.");
        let mut msg = std::mem::zeroed::<Msg>();
        while GetMessageW(&mut msg, 0, 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UnhookWindowsHookEx(kb);
        UnhookWindowsHookEx(ms);
    }
    // Whatever was held is released on the PS5 before quitting.
    APP.with(|a| {
        if let Some(app) = a.borrow_mut().as_mut() {
            app.capture = Capture::default();
            for _ in 0..3 {
                app.send();
            }
        }
    });
}
