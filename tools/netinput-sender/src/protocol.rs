//! TXIN v1: one 64-byte UDP datagram carrying the sender's whole keyboard and mouse state.
//! The receiver is prospero-win's src/pw_netinput.c (patches/prospero-win/0012); keep the two in step.
//!
//! ```text
//! offset size field (little-endian)
//!      0    4 magic "TXIN"
//!      4    1 version (1)
//!      5    1 flags: bit 0 capturing (0: the receiver releases everything)
//!      6    2 reserved, 0
//!      8    4 session: random per sender start
//!     12    4 sequence: +1 per datagram
//!     16   32 held Windows virtual keys, bit (vk & 7) of byte vk >> 3
//!     48    1 held mouse buttons: bit 0 left, 1 right, 2 middle
//!     49    3 reserved, 0
//!     52    4 mouse motion x, total since the session began (wraps)
//!     56    4 mouse motion y, total (down positive)
//!     60    4 wheel detents, total (up positive)
//! ```

pub const PORT: u16 = 9396;
pub const LEN: usize = 64;
pub const VERSION: u8 = 1;
pub const CAPTURING: u8 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Packet {
    pub flags: u8,
    pub session: u32,
    pub sequence: u32,
    pub keys: [u8; 32],
    pub buttons: u8,
    pub motion_x: u32,
    pub motion_y: u32,
    pub wheel: u32,
}

impl Packet {
    pub fn encode(&self) -> [u8; LEN] {
        let mut out = [0u8; LEN];
        out[0..4].copy_from_slice(b"TXIN");
        out[4] = VERSION;
        out[5] = self.flags;
        out[8..12].copy_from_slice(&self.session.to_le_bytes());
        out[12..16].copy_from_slice(&self.sequence.to_le_bytes());
        out[16..48].copy_from_slice(&self.keys);
        out[48] = self.buttons & 7;
        out[52..56].copy_from_slice(&self.motion_x.to_le_bytes());
        out[56..60].copy_from_slice(&self.motion_y.to_le_bytes());
        out[60..64].copy_from_slice(&self.wheel.to_le_bytes());
        out
    }
}

pub fn set_key(keys: &mut [u8; 32], vk: u8, down: bool) {
    let bit = 1u8 << (vk & 7);
    if down {
        keys[(vk >> 3) as usize] |= bit;
    } else {
        keys[(vk >> 3) as usize] &= !bit;
    }
}

pub fn key_held(keys: &[u8; 32], vk: u8) -> bool {
    keys[(vk >> 3) as usize] & (1 << (vk & 7)) != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same datagram tests/test_pw_netinput.c builds and parses on the receiving side.
    #[test]
    fn layout_matches_the_receiver() {
        let mut p = Packet {
            flags: CAPTURING,
            session: 0x1122_3344,
            sequence: 7,
            buttons: 5,
            motion_x: (-3i32) as u32,
            motion_y: 9,
            wheel: 2,
            ..Default::default()
        };
        set_key(&mut p.keys, b'W', true);
        let b = p.encode();
        assert_eq!(&b[0..6], b"TXIN\x01\x01");
        assert_eq!(&b[8..12], &[0x44, 0x33, 0x22, 0x11]);
        assert_eq!(&b[12..16], &[7, 0, 0, 0]);
        assert_eq!(b[16 + (b'W' >> 3) as usize], 1 << (b'W' & 7));
        assert_eq!(b[48], 5);
        assert_eq!(&b[52..56], &[0xfd, 0xff, 0xff, 0xff]);
        assert_eq!(&b[56..60], &[9, 0, 0, 0]);
        assert_eq!(&b[60..64], &[2, 0, 0, 0]);
        assert!(b[6..8].iter().chain(&b[49..52]).all(|&x| x == 0));
    }

    #[test]
    fn keys() {
        let mut k = [0u8; 32];
        set_key(&mut k, 0xff, true);
        set_key(&mut k, 0xa0, true);
        assert!(key_held(&k, 0xff) && key_held(&k, 0xa0) && !key_held(&k, 0xa1));
        set_key(&mut k, 0xa0, false);
        assert!(!key_held(&k, 0xa0));
    }
}
