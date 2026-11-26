//! One sender session: the capture state and the UDP socket it goes out on.

use crate::capture::{Capture, Outcome};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
