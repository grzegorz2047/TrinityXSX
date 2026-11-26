//! Just enough HTTP and WebSocket (RFC 6455) for the local capture page: one handshake, masked client
//! frames in, text frames out. No dependencies, so the sender still builds offline.

use std::io::{self, Read, Write};

/// The largest frame the page sends is a short text command; anything bigger is refused.
const MAX_PAYLOAD: u64 = 4096;

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476, 0xc3d2_e1f0];
    let mut message = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, &wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, x) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&x.to_be_bytes());
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The Sec-WebSocket-Accept answer to a client's Sec-WebSocket-Key.
pub fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{}258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key.trim()).as_bytes()))
}

/// An HTTP request's line and headers (names lower-cased).
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }
}

/// Reads a request head (a GET has no body); None when it is malformed or larger than 16 KiB.
pub fn read_request(stream: &mut impl Read) -> Option<Request> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() > 16 * 1024 || stream.read(&mut byte).ok()? == 0 {
            return None;
        }
        head.push(byte[0]);
    }
    let text = String::from_utf8(head).ok()?;
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?.to_string();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    Some(Request { method, target, headers })
}

pub enum Frame {
    Text(String),
    Ping(Vec<u8>),
    Close,
    Other,
}

/// Reads one client frame (clients always mask).
pub fn read_frame(stream: &mut impl Read) -> io::Result<Frame> {
    let mut head = [0u8; 2];
    stream.read_exact(&mut head)?;
    let opcode = head[0] & 0x0f;
    let masked = head[1] & 0x80 != 0;
    let mut length = (head[1] & 0x7f) as u64;
    if length == 126 {
        let mut b = [0u8; 2];
        stream.read_exact(&mut b)?;
        length = u16::from_be_bytes(b) as u64;
    } else if length == 127 {
        let mut b = [0u8; 8];
        stream.read_exact(&mut b)?;
        length = u64::from_be_bytes(b);
    }
    if !masked || length > MAX_PAYLOAD {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unmasked or oversized frame"));
    }
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask)?;
    let mut payload = vec![0u8; length as usize];
    stream.read_exact(&mut payload)?;
    for (i, b) in payload.iter_mut().enumerate() {
        *b ^= mask[i % 4];
    }
    Ok(match opcode {
        1 => Frame::Text(String::from_utf8_lossy(&payload).into_owned()),
        8 => Frame::Close,
        9 => Frame::Ping(payload),
        _ => Frame::Other,
    })
}

fn write_frame(stream: &mut impl Write, opcode: u8, payload: &[u8]) -> io::Result<()> {
    let mut frame = vec![0x80 | opcode];
    if payload.len() < 126 {
        frame.push(payload.len() as u8);
    } else {
        frame.push(126);
        frame.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(payload);
    stream.write_all(&frame)
}

pub fn write_text(stream: &mut impl Write, text: &str) -> io::Result<()> {
    write_frame(stream, 1, text.as_bytes())
}

pub fn write_pong(stream: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    write_frame(stream, 10, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_and_base64() {
        assert_eq!(base64(&sha1(b"abc")), "qZk+NkcGgWq6PiVxeFDCbJzQ2J0=");
        assert_eq!(base64(&sha1(b"")), "2jmj7l5rSw0yVb/vlWAYkK/YBwk=");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
    }

    /// RFC 6455 section 1.3.
    #[test]
    fn handshake_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn request_head() {
        let raw = b"GET /ws?x=1 HTTP/1.1\r\nHost: 127.0.0.1:9397\r\nUpgrade: websocket\r\n\r\nrest";
        let r = read_request(&mut &raw[..]).unwrap();
        assert_eq!((r.method.as_str(), r.path()), ("GET", "/ws"));
        assert_eq!(r.header("host"), Some("127.0.0.1:9397"));
        assert_eq!(r.header("upgrade"), Some("websocket"));
        assert!(read_request(&mut &b"GET / HTTP/1.1\r\n"[..]).is_none());
    }

    #[test]
    fn frames() {
        // "Hello" masked, from RFC 6455 section 5.7.
        let raw = [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58];
        assert!(matches!(read_frame(&mut &raw[..]).unwrap(), Frame::Text(t) if t == "Hello"));
        // Unmasked client frames are refused.
        assert!(read_frame(&mut &[0x81, 0x05, b'H', b'e', b'l', b'l', b'o'][..]).is_err());
        let mut out = Vec::new();
        write_text(&mut out, "Hi").unwrap();
        assert_eq!(out, [0x81, 2, b'H', b'i']);
        let mut out = Vec::new();
        write_text(&mut out, &"x".repeat(300)).unwrap();
        assert_eq!(&out[..4], &[0x81, 126, 1, 44]);
    }
}
