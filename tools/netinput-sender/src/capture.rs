//! What the keyboard and mouse hooks decide, kept apart from Windows so it can be tested.
//!
//! Ctrl+Alt+F12 starts and stops capturing. While capturing, every key and button goes to the PS5 and is
//! hidden from Windows, except the release of something Windows saw pressed before capture began (Ctrl and
//! Alt of the shortcut itself): Windows gets that release, or it would think the key is still down.

use crate::protocol::{key_held, set_key, Packet, CAPTURING};

pub const VK_F12: u8 = 0x7b;
const CTRL: [u8; 3] = [0x11, 0xa2, 0xa3];
const ALT: [u8; 3] = [0x12, 0xa4, 0xa5];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Windows gets the event too.
    Pass,
    /// Only the PS5 gets it.
    Swallow,
}

#[derive(Default)]
pub struct Capture {
    pub on: bool,
    keys: [u8; 32],
    buttons: u8,
    /// Held by Windows: pressed while not capturing.
    windows_keys: [u8; 32],
    windows_buttons: u8,
    /// The F12 that toggled; its release is not anyone's.
    swallow_f12_up: bool,
    motion_x: u32,
    motion_y: u32,
    wheel: u32,
    wheel_rest: i32,
}

/// What one event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub verdict: Verdict,
    /// The state changed: send a datagram.
    pub changed: bool,
    /// Capture was switched (on or off) by this event.
    pub toggled: bool,
}

const PASS: Outcome = Outcome { verdict: Verdict::Pass, changed: false, toggled: false };
const SWALLOW: Outcome = Outcome { verdict: Verdict::Swallow, changed: false, toggled: false };

impl Capture {
    fn any(&self, vks: &[u8]) -> bool {
        vks.iter().any(|&vk| key_held(&self.keys, vk) || key_held(&self.windows_keys, vk))
    }

    pub fn key(&mut self, vk: u8, down: bool) -> Outcome {
        if vk == VK_F12 && down && self.any(&CTRL) && self.any(&ALT) {
            self.on = !self.on;
            self.swallow_f12_up = true;
            if !self.on {
                self.keys = [0; 32];
                self.buttons = 0;
            }
            return Outcome { verdict: Verdict::Swallow, changed: true, toggled: true };
        }
        if vk == VK_F12 && !down && self.swallow_f12_up {
            self.swallow_f12_up = false;
            return SWALLOW;
        }
        if !down && key_held(&self.windows_keys, vk) {
            set_key(&mut self.windows_keys, vk, false);
            return PASS;
        }
        if !self.on {
            if down {
                set_key(&mut self.windows_keys, vk, true);
            }
            return PASS;
        }
        let changed = key_held(&self.keys, vk) != down;
        set_key(&mut self.keys, vk, down);
        Outcome { verdict: Verdict::Swallow, changed, toggled: false }
    }

    /// Button 0 left, 1 right, 2 middle; others (X1, X2) are swallowed while capturing but not sent.
    pub fn button(&mut self, button: u8, down: bool) -> Outcome {
        let bit = if button < 3 { 1u8 << button } else { 0 };
        if !down && self.windows_buttons & bit != 0 {
            self.windows_buttons &= !bit;
            return PASS;
        }
        if !self.on {
            if down {
                self.windows_buttons |= bit;
            }
            return PASS;
        }
        let before = self.buttons;
        if down {
            self.buttons |= bit;
        } else {
            self.buttons &= !bit;
        }
        Outcome { verdict: Verdict::Swallow, changed: before != self.buttons, toggled: false }
    }

    /// Relative motion in pixels (down positive).
    pub fn motion(&mut self, dx: i32, dy: i32) -> Outcome {
        if !self.on {
            return PASS;
        }
        self.motion_x = self.motion_x.wrapping_add(dx as u32);
        self.motion_y = self.motion_y.wrapping_add(dy as u32);
        Outcome { verdict: Verdict::Swallow, changed: dx != 0 || dy != 0, toggled: false }
    }

    /// A WM_MOUSEWHEEL delta (120 per detent, forward positive).
    pub fn wheel(&mut self, delta: i32) -> Outcome {
        if !self.on {
            return PASS;
        }
        self.wheel_rest += delta;
        let detents = self.wheel_rest / 120;
        self.wheel_rest -= detents * 120;
        self.wheel = self.wheel.wrapping_add(detents as u32);
        Outcome { verdict: Verdict::Swallow, changed: detents != 0, toggled: false }
    }

    pub fn packet(&self, session: u32, sequence: u32) -> Packet {
        Packet {
            flags: if self.on { CAPTURING } else { 0 },
            session,
            sequence,
            keys: if self.on { self.keys } else { [0; 32] },
            buttons: if self.on { self.buttons } else { 0 },
            motion_x: self.motion_x,
            motion_y: self.motion_y,
            wheel: self.wheel,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle(c: &mut Capture) {
        assert_eq!(c.key(0xa2, true).verdict, Verdict::Pass.max_if(c.on));
        assert_eq!(c.key(0xa4, true).verdict, Verdict::Pass.max_if(c.on));
        let o = c.key(VK_F12, true);
        assert!(o.toggled && o.changed && o.verdict == Verdict::Swallow);
        assert_eq!(c.key(VK_F12, false).verdict, Verdict::Swallow);
    }

    impl Verdict {
        /// Pass, or Swallow when capture is already on.
        fn max_if(self, on: bool) -> Verdict {
            if on {
                Verdict::Swallow
            } else {
                self
            }
        }
    }

    #[test]
    fn off_passes_everything() {
        let mut c = Capture::default();
        assert_eq!(c.key(b'W', true), PASS);
        assert_eq!(c.button(0, true), PASS);
        assert_eq!(c.motion(5, 5), PASS);
        assert_eq!(c.wheel(120), PASS);
        assert_eq!(c.packet(1, 1).flags, 0);
    }

    #[test]
    fn shortcut_keys_are_released_to_windows() {
        let mut c = Capture::default();
        toggle(&mut c);
        assert!(c.on);
        // Ctrl and Alt were pressed before capture: Windows gets their releases, the PS5 never sees them.
        assert_eq!(c.key(0xa2, false).verdict, Verdict::Pass);
        assert_eq!(c.key(0xa4, false).verdict, Verdict::Pass);
        assert_eq!(c.packet(1, 1).keys, [0; 32]);
    }

    #[test]
    fn capture_sends_and_swallows() {
        let mut c = Capture::default();
        toggle(&mut c);
        c.key(0xa2, false);
        c.key(0xa4, false);
        let o = c.key(b'W', true);
        assert!(o.changed && o.verdict == Verdict::Swallow);
        assert!(!c.key(b'W', true).changed, "auto-repeat is no change");
        assert!(c.button(1, true).changed);
        c.motion(10, -4);
        c.motion(-3, 0);
        assert!(!c.wheel(60).changed);
        assert!(c.wheel(60).changed);
        let p = c.packet(9, 2);
        assert_eq!(p.flags, CAPTURING);
        assert!(key_held(&p.keys, b'W'));
        assert_eq!((p.buttons, p.motion_x as i32, p.motion_y as i32, p.wheel), (2, 7, -4, 1));
        assert_eq!(c.button(3, true).verdict, Verdict::Swallow);
        assert!(!c.button(3, true).changed);
    }

    #[test]
    fn stopping_releases_everything() {
        let mut c = Capture::default();
        toggle(&mut c);
        c.key(b'W', true);
        c.button(0, true);
        // Ctrl and Alt are still held from the start; F12 again stops.
        toggle(&mut c);
        assert!(!c.on);
        let p = c.packet(1, 3);
        assert_eq!((p.flags, p.keys, p.buttons), (0, [0; 32], 0));
        // The W and the button were never Windows'; their releases now pass (harmless).
        assert_eq!(c.key(b'W', false), PASS);
        assert_eq!(c.button(0, false), PASS);
    }

    #[test]
    fn a_button_held_before_capture_is_released_to_windows() {
        let mut c = Capture::default();
        c.button(0, true);
        toggle(&mut c);
        assert_eq!(c.button(0, false), PASS);
        assert_eq!(c.packet(1, 1).buttons, 0);
    }
}
