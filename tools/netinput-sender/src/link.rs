//! One sender session: the capture state and the UDP socket it goes out on.

use crate::capture::{Capture, Outcome};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Port where the PS5 serves the web capture page and read-only `/status` endpoint.
pub const PS5_WEB_PORT: u16 = 8361;
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Result of a quick, read-only LAN probe to the PS5's `GET /status` on port 8361.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ps5WebStatus {
    /// PS5 is in browser-only mode (`udp=0`): UDP 9396 is closed and txin-sender packets will be ignored.
    BrowserOnly,
    /// PS5 is listening on both browser (TCP 8361) and txin-sender (UDP 9396).
    Both,
    /// Port 8361 is closed or answered unexpectedly (e.g. sender-only mode, off, or older build).
    Unknown,
}

pub fn parse_ps5_web_status(response: &str) -> Ps5WebStatus {
    let Some(first) = response.lines().next() else { return Ps5WebStatus::Unknown };
    if !first.contains(" 200 ") {
        return Ps5WebStatus::Unknown;
    }
    let body = response.split_once("\r\n\r\n").or_else(|| response.split_once("\n\n")).map(|(_, b)| b).unwrap_or("");
    let mut mode = "";
    let mut udp = None;
    for line in body.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "mode" => mode = v.trim(),
                "udp" => udp = Some(v.trim()),
                _ => {}
            }
        }
    }
    if udp == Some("0") || mode == "browser" {
        Ps5WebStatus::BrowserOnly
    } else if udp == Some("1") || mode == "both" {
        Ps5WebStatus::Both
    } else {
        Ps5WebStatus::Unknown
    }
}

pub fn probe_ps5_web_status_at(addr: SocketAddr) -> Ps5WebStatus {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, PROBE_TIMEOUT) else {
        return Ps5WebStatus::Unknown;
    };
    let _ = stream.set_read_timeout(Some(PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(PROBE_TIMEOUT));
    let req = format!("GET /status HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n", addr);
    if stream.write_all(req.as_bytes()).is_err() {
        return Ps5WebStatus::Unknown;
    }
    let mut buf = [0u8; 512];
    let mut used = 0;
    while used < buf.len() {
        match stream.read(&mut buf[used..]) {
            Ok(0) => break,
            Ok(n) => used += n,
            Err(_) => break,
        }
    }
    let Ok(text) = std::str::from_utf8(&buf[..used]) else {
        return Ps5WebStatus::Unknown;
    };
    parse_ps5_web_status(text)
}

pub fn probe_ps5_web_status(ip: Ipv4Addr) -> Ps5WebStatus {
    probe_ps5_web_status_at(SocketAddr::new(IpAddr::V4(ip), PS5_WEB_PORT))
}

/// Checks if the PS5 is running in Browser-only mode (UDP closed) and prints an actionable hint.
pub fn warn_if_ps5_browser_only(target: SocketAddr) {
    let IpAddr::V4(ip) = target.ip() else { return };
    if !private_ipv4(ip) {
        return;
    }
    match probe_ps5_web_status(ip) {
        Ps5WebStatus::BrowserOnly => {
            eprintln!("WARNING: PS5 at {ip} is currently in Browser-only mode (UDP {} is closed).", target.port());
            eprintln!(
                "  -> Open the Touchpad menu on PS5 (or press F9 in the browser) and switch PC INPUT to\n     \
                 \"BROWSER + SENDER\" or \"TXIN-SENDER\", or open http://{ip}:{PS5_WEB_PORT} directly."
            );
        }
        Ps5WebStatus::Both => {
            println!("txin-sender: PS5 at {ip} confirmed ready (Browser + txin-sender mode).");
        }
        Ps5WebStatus::Unknown => {}
    }
}

/// While capturing, a datagram at least this often, so the PS5 knows the sender is alive.
pub const HEARTBEAT: Duration = Duration::from_millis(50);

pub struct Link {
    pub capture: Capture,
    pub target: SocketAddr,
    pub sent: u64,
    socket: UdpSocket,
    session: u32,
    sequence: u32,
    last_sent: Instant,
}

/// Milliseconds since the program started.
pub fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// The addresses the PS5 receiver accepts datagrams from, and so the only ones worth sending to.
pub fn private_ipv4(ip: Ipv4Addr) -> bool {
    ip.is_private() || ip.is_link_local()
}

fn session_id() -> u32 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    (nanos as u32) ^ ((nanos >> 32) as u32) ^ std::process::id().rotate_left(16)
}

impl Link {
    pub fn connect(host: &str, port: u16) -> io::Result<Link> {
        let target = (host, port)
            .to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no IPv4 address"))?;
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.connect(target)?;
        Ok(Link {
            capture: Capture::default(),
            target,
            sent: 0,
            socket,
            session: session_id(),
            sequence: 0,
            last_sent: Instant::now(),
        })
    }

    pub fn is_private(&self) -> bool {
        matches!(self.target.ip(), IpAddr::V4(ip) if private_ipv4(ip))
    }

    pub fn session(&self) -> u32 {
        self.session
    }

    pub fn send(&mut self) {
        self.sequence = self.sequence.wrapping_add(1);
        let bytes = self.capture.packet(self.session, self.sequence).encode();
        if self.socket.send(&bytes).is_ok() {
            self.sent += 1;
        }
        self.last_sent = Instant::now();
    }

    /// Sends what an event changed; a stop is sent three times, as its release must arrive.
    pub fn deliver(&mut self, outcome: Outcome) {
        if outcome.changed || outcome.toggled {
            self.send();
            if outcome.toggled && !self.capture.on {
                self.send();
                self.send();
            }
        }
    }

    /// From a timer: a long Esc stops capturing; otherwise a heartbeat when one is due. Returns the outcome
    /// so the caller can tell the user about a panic stop.
    pub fn tick(&mut self) -> Outcome {
        let outcome = self.capture.tick(now_ms());
        if outcome.toggled {
            self.deliver(outcome);
        } else if self.capture.on && self.last_sent.elapsed() >= HEARTBEAT {
            self.send();
        }
        outcome
    }

    /// Stops capturing and makes sure the PS5 hears it.
    pub fn release(&mut self) {
        self.capture.stop();
        for _ in 0..3 {
            self.send();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parses_ps5_web_status() {
        assert_eq!(
            parse_ps5_web_status("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nmode=browser\nudp=0\n"),
            Ps5WebStatus::BrowserOnly
        );
        assert_eq!(
            parse_ps5_web_status("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nmode=both\nudp=1\n"),
            Ps5WebStatus::Both
        );
        assert_eq!(parse_ps5_web_status("HTTP/1.1 404 Not Found\r\n\r\n404 Not Found"), Ps5WebStatus::Unknown);
        assert_eq!(parse_ps5_web_status(""), Ps5WebStatus::Unknown);
    }

    #[test]
    fn probes_local_status_server() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            if let Ok((mut s, _)) = listener.accept() {
                let mut req = [0u8; 256];
                let _ = s.read(&mut req);
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 19\r\n\r\nmode=browser\nudp=0\n");
            }
        });
        assert_eq!(probe_ps5_web_status_at(addr), Ps5WebStatus::BrowserOnly);
        handle.join().unwrap();
    }
}
