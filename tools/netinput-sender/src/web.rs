//! The capture page: `txin-sender` serves it on http://127.0.0.1:9397 and opens it in the browser.
//!
//! The page goes full screen, locks the keyboard (Keyboard Lock, so Esc, Alt+Tab and the Windows key reach
//! it) and the mouse (Pointer Lock, raw motion), and sends what it sees over a WebSocket as short text
//! commands; this side turns them into TXIN datagrams for the PS5. Keyboard Lock only works in a secure
//! context, which http://127.0.0.1 is and a page served by the PS5 over plain HTTP would not be.
//!
//! Only this computer can connect (127.0.0.1), and only the page itself (the Host and Origin headers), so
//! another site open in the browser cannot type into the PS5. Datagrams only go to private (LAN) addresses.

use crate::link::{now_ms, Link};
use crate::ws::{self, Frame, Request};
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub const WEB_PORT: u16 = 9397;
const PAGE: &str = include_str!("web/index.html");
const TICK: Duration = Duration::from_millis(25);
/// A capturing page that says nothing for this long (it sends `h` every 500 ms) has hung or is paused:
/// what it held is released, as the PS5's own server does, so no key stays down on the console.
const SILENT_MS: u64 = 1500;

#[link(name = "shell32")]
extern "system" {
    fn ShellExecuteW(
        hwnd: isize,
        operation: *const u16,
        file: *const u16,
        parameters: *const u16,
        directory: *const u16,
        show: i32,
    ) -> isize;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn open_browser(url: &str) {
    let (verb, file) = (wide("open"), wide(url));
    // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call.
    unsafe { ShellExecuteW(0, verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
}

/// Serves the page until the program is closed. `prefill` is a PS5 address for the page's field.
pub fn serve(prefill: Option<&str>, udp_port: u16, open: bool) -> io::Result<()> {
    let query = prefill.map(|h| format!("?ps5={h}")).unwrap_or_default();
    let url = format!("http://127.0.0.1:{WEB_PORT}/{query}");
    let listener = match TcpListener::bind(("127.0.0.1", WEB_PORT)) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            println!("txin-sender is already running: opening {url}");
            if open {
                open_browser(&url);
            }
            return Ok(());
        }
        Err(e) => return Err(e),
    };
    println!("txin-sender: the capture page is at {url}");
    println!("Keep this window open while you play; close it to quit.");
    if open {
        open_browser(&url);
    }
    for stream in listener.incoming().flatten() {
        thread::spawn(move || handle(stream, udp_port));
    }
    Ok(())
}

fn local_host(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v == format!("127.0.0.1:{WEB_PORT}") || v == format!("localhost:{WEB_PORT}"))
}

fn local_origin(value: Option<&str>) -> bool {
    matches!(value, Some(v) if v.strip_prefix("http://").is_some_and(|h| local_host(Some(h))))
}

fn respond(stream: &mut TcpStream, status: &str, kind: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(mut stream: TcpStream, udp_port: u16) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let Some(request) = ws::read_request(&mut stream) else { return };
    if !local_host(request.header("host")) {
        return respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
    }
    match (request.method.as_str(), request.path()) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE),
        ("GET", "/ws") if is_upgrade(&request) => {
            if !local_origin(request.header("origin")) {
                return respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
            }
            let accept = ws::accept_key(request.header("sec-websocket-key").unwrap_or(""));
            let _ = write!(
                stream,
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
                 Sec-WebSocket-Accept: {accept}\r\n\r\n"
            );
            let _ = stream.set_read_timeout(None);
            let _ = stream.set_nodelay(true);
            session(stream, udp_port);
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
}

fn is_upgrade(request: &Request) -> bool {
    request.header("upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
        && request.header("sec-websocket-key").is_some()
}

/// One page: commands in, TXIN datagrams out; a timer thread sends heartbeats, watches for a long Esc, and
/// releases everything if a capturing page goes silent.
fn session(stream: TcpStream, udp_port: u16) {
    println!("page connected");
    let Ok(writer) = stream.try_clone() else { return };
    let writer = Arc::new(Mutex::new(writer));
    let link: Arc<Mutex<Option<Link>>> = Arc::new(Mutex::new(None));
    let closed = Arc::new(AtomicBool::new(false));
    let heard = Arc::new(AtomicU64::new(now_ms()));
    {
        let (link, writer, closed, heard) = (link.clone(), writer.clone(), closed.clone(), heard.clone());
        thread::spawn(move || {
            let mut reported = u64::MAX;
            let mut ticks = 0u32;
            while !closed.load(Ordering::SeqCst) {
                thread::sleep(TICK);
                ticks += 1;
                let mut note = None;
                if let Some(link) = link.lock().unwrap().as_mut() {
                    let silent = now_ms().saturating_sub(heard.load(Ordering::SeqCst));
                    if link.capture.on && silent >= SILENT_MS {
                        link.release();
                        println!("page silent for {silent} ms - capture stopped, everything released");
                        note = Some("stopped".to_string());
                    } else if link.tick().panic {
                        println!("PANIC: Esc held - capture stopped ({} datagrams sent)", link.sent);
                        note = Some("panic".to_string());
                    } else if ticks.is_multiple_of(20) && link.sent != reported {
                        reported = link.sent;
                        note = Some(format!("sent {}", link.sent));
                    }
                }
                if let Some(note) = note {
                    let _ = ws::write_text(&mut *writer.lock().unwrap(), &note);
                }
            }
        });
    }
    let mut reader = stream;
    loop {
        let frame = ws::read_frame(&mut reader);
        heard.store(now_ms(), Ordering::SeqCst);
        let reply = match frame {
            Ok(Frame::Text(text)) => command(&mut link.lock().unwrap(), &text, udp_port),
            Ok(Frame::Ping(payload)) => {
                let _ = ws::write_pong(&mut *writer.lock().unwrap(), &payload);
                None
            }
            Ok(Frame::Other) => None,
            Ok(Frame::Close) | Err(_) => break,
        };
        if let Some(reply) = reply {
            if ws::write_text(&mut *writer.lock().unwrap(), &reply).is_err() {
                break;
            }
        }
    }
    closed.store(true, Ordering::SeqCst);
    println!("page disconnected");
    let mut link = link.lock().unwrap();
    if let Some(link) = link.as_mut() {
        link.release();
    }
}

/// One command from the page; the reply to send back, if any.
///
/// `target <host>` · `on` · `off` · `k <vk> <0|1>` · `b <button> <0|1>` · `m <dx> <dy>` · `w <detents>` ·
/// `h` (the page's heartbeat: it needs no answer, and like every frame it tells `session` the page is alive;
/// the datagram heartbeats to the PS5 go out by timer)
pub fn command(link: &mut Option<Link>, text: &str, udp_port: u16) -> Option<String> {
    let mut words = text.split_whitespace();
    let verb = words.next()?;
    let mut number = || words.next().and_then(|w| w.parse::<i32>().ok());
    if verb == "h" {
        return None;
    }
    if verb == "target" {
        let host = text["target".len()..].trim();
        if let Some(old) = link.as_mut() {
            old.release();
        }
        *link = None;
        return Some(match Link::connect(host, udp_port) {
            Ok(new) if new.is_private() => {
                println!("PS5 at {}, session {:08x}", new.target, new.session());
                let reply = format!("target {} {:08x}", new.target, new.session());
                *link = Some(new);
                reply
            }
            Ok(new) => format!("error {} is not on the local network", new.target.ip()),
            Err(e) => format!("error {host}: {e}"),
        });
    }
    let Some(link) = link.as_mut() else { return Some("error no PS5 address yet".into()) };
    if !link.capture.on && matches!(verb, "k" | "b" | "m" | "w") {
        return None; // the page only sends input while capturing; late events after a stop are dropped
    }
    let outcome = match verb {
        "on" => {
            link.capture.start();
            link.send();
            println!("capture ON");
            return Some("on".into());
        }
        "off" => {
            link.release();
            println!("capture OFF ({} datagrams sent)", link.sent);
            return Some("off".into());
        }
        "k" => {
            let (vk, down) = (number()?, number()?);
            link.capture.key(u8::try_from(vk).ok()?, down != 0, now_ms())
        }
        "b" => {
            let (button, down) = (number()?, number()?);
            link.capture.button(u8::try_from(button).ok()?, down != 0)
        }
        "m" => {
            let (dx, dy) = (number()?, number()?);
            link.capture.motion(dx.clamp(-32767, 32767), dy.clamp(-32767, 32767))
        }
        "w" => link.capture.wheel(number()?.clamp(-100, 100) * 120),
        _ => return Some(format!("error unknown command {verb}")),
    };
    link.deliver(outcome);
    outcome.panic.then(|| "panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::key_held;

    #[test]
    fn hosts_and_origins() {
        assert!(local_host(Some("127.0.0.1:9397")) && local_host(Some("localhost:9397")));
        assert!(!local_host(Some("evil.example:9397")) && !local_host(Some("127.0.0.1:80")) && !local_host(None));
        assert!(local_origin(Some("http://127.0.0.1:9397")) && local_origin(Some("http://localhost:9397")));
        assert!(!local_origin(Some("https://evil.example")) && !local_origin(Some("null")) && !local_origin(None));
    }

    #[test]
    fn commands() {
        let mut link = None;
        assert_eq!(command(&mut link, "h", 9), None, "the heartbeat needs no PS5 address");
        assert!(command(&mut link, "on", 9).unwrap().starts_with("error"));
        assert!(command(&mut link, "target 8.8.8.8", 9).unwrap().starts_with("error"));
        assert!(link.is_none());
        assert!(command(&mut link, "target 192.168.1.250", 9).unwrap().starts_with("target 192.168.1.250:9 "));
        assert_eq!(command(&mut link, "k 87 1", 9), None);
        assert!(!key_held(&link.as_ref().unwrap().capture.packet(0, 0).keys, 87), "not capturing yet");
        assert_eq!(command(&mut link, "on", 9).as_deref(), Some("on"));
        command(&mut link, "k 65 1", 9);
        command(&mut link, "b 1 1", 9);
        command(&mut link, "m 5 -2", 9);
        command(&mut link, "w -1", 9);
        let p = link.as_ref().unwrap().capture.packet(0, 0);
        assert!(key_held(&p.keys, 65));
        assert_eq!((p.buttons, p.motion_x as i32, p.motion_y as i32, p.wheel as i32), (2, 5, -2, -1));
        assert_eq!(command(&mut link, "k 999 1", 9), None, "not a virtual key");
        assert!(command(&mut link, "jump", 9).unwrap().starts_with("error"));
        assert_eq!(command(&mut link, "off", 9).as_deref(), Some("off"));
        let p = link.as_ref().unwrap().capture.packet(0, 0);
        assert_eq!((p.flags, p.keys, p.buttons), (0, [0; 32], 0));
    }
}
