//! What the keyboard and mouse hooks decide, kept apart from Windows so it can be tested.
//!
//! Ctrl+Alt+F12 starts and stops capturing. While capturing, every key and button goes to the PS5 and is
//! hidden from Windows, except the release of something Windows saw pressed before capture began (Ctrl and
//! Alt of the shortcut itself): Windows gets that release, or it would think the key is still down.
//!
//! Panic: holding Esc for `PANIC_MS` while capturing stops capturing, so the PC is always recoverable even if
//! the shortcut does not get through. A short Esc still goes to the game.

use crate::protocol::{key_held, set_key, Packet, CAPTURING};

pub const VK_F12: u8 = 0x7b;
pub const VK_ESCAPE: u8 = 0x1b;
/// How long Esc must be held to stop capturing.
pub const PANIC_MS: u64 = 1500;
/// Longer than Windows' longest typematic delay (1 s): a key-down sooner after the last one is a repeat.
const REPEAT_GAP_MS: u64 = 1100;
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
    /// When the held Esc went down (ms), while capturing.
    esc_since: Option<u64>,
    /// The Esc that panicked; its release is not anyone's.
    swallow_esc_up: bool,
    /// When F12 and Esc last went down (ms), to tell auto-repeat from a new press.
    f12_down_ms: u64,
    esc_down_ms: u64,
}

/// What one event did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub verdict: Verdict,
    /// The state changed: send a datagram.
    pub changed: bool,
    /// Capture was switched (on or off) by this event.
    pub toggled: bool,
    /// It was switched off by a long Esc.
    pub panic: bool,
}

const PASS: Outcome = Outcome { verdict: Verdict::Pass, changed: false, toggled: false, panic: false };
const SWALLOW: Outcome = Outcome { verdict: Verdict::Swallow, changed: false, toggled: false, panic: false };
const TOGGLED: Outcome = Outcome { verdict: Verdict::Swallow, changed: true, toggled: true, panic: false };
const PANIC: Outcome = Outcome { verdict: Verdict::Swallow, changed: true, toggled: true, panic: true };

impl Capture {
    fn any(&self, vks: &[u8]) -> bool {
        vks.iter().any(|&vk| key_held(&self.keys, vk) || key_held(&self.windows_keys, vk))
    }

    /// Starts capturing (the web page's shortcut, or the hook's).
    pub fn start(&mut self) {
        self.on = true;
        self.esc_since = None;
        self.swallow_esc_up = false;
    }

    /// Stops capturing: nothing is held any more.
    pub fn stop(&mut self) {
        self.on = false;
        self.keys = [0; 32];
        self.buttons = 0;
        self.esc_since = None;
    }

    /// Esc held long enough: stop. Called on every event and by a timer, as a held key may not repeat.
    pub fn tick(&mut self, now_ms: u64) -> Outcome {
        match self.esc_since {
            Some(since) if self.on && now_ms.saturating_sub(since) >= PANIC_MS => {
                self.stop();
                self.swallow_esc_up = true;
                self.esc_down_ms = now_ms;
                PANIC
            }
            _ => PASS,
        }
    }

    pub fn key(&mut self, vk: u8, down: bool, now_ms: u64) -> Outcome {
        // Low-level hooks see auto-repeat as more key-downs. Repeats of the F12 that toggled are nobody's:
        // they neither toggle again nor reach the PS5 or Windows, even once Ctrl and Alt are let go first.
        // A down after a pause longer than any typematic delay is a new press (its release was lost, as on
        // the secure desktop), and counts.
        if vk == VK_F12 && down && self.swallow_f12_up {
            let repeat = now_ms.saturating_sub(self.f12_down_ms) < REPEAT_GAP_MS;
            self.f12_down_ms = now_ms;
            if repeat {
                return SWALLOW;
            }
            self.swallow_f12_up = false;
        }
        // Likewise the repeats of the Esc that panicked, until its release.
        if vk == VK_ESCAPE && down && self.swallow_esc_up && !self.on {
            let repeat = now_ms.saturating_sub(self.esc_down_ms) < REPEAT_GAP_MS;
            self.esc_down_ms = now_ms;
            if repeat {
                return SWALLOW;
            }
            self.swallow_esc_up = false;
        }
        if vk == VK_F12 && down {
            self.f12_down_ms = now_ms;
        }
        if vk == VK_ESCAPE && down {
            self.esc_down_ms = now_ms;
        }
        if vk == VK_F12 && down && self.any(&CTRL) && self.any(&ALT) {
            if self.on {
                self.stop();
            } else {
                self.start();
            }
            self.swallow_f12_up = true;
            return TOGGLED;
        }
        if vk == VK_F12 && !down && self.swallow_f12_up {
            self.swallow_f12_up = false;
            if key_held(&self.windows_keys, VK_F12) {
                set_key(&mut self.windows_keys, VK_F12, false);
                return PASS;
            }
            return SWALLOW;
        }
        if vk == VK_ESCAPE && self.on {
            if !down {
                self.esc_since = None;
            } else if self.esc_since.is_none() {
                self.esc_since = Some(now_ms);
            } else if self.tick(now_ms).panic {
                return PANIC;
            }
        }
        if vk == VK_ESCAPE && !down && self.swallow_esc_up {
            self.swallow_esc_up = false;
            return SWALLOW;
        }
        if !down && key_held(&self.windows_keys, vk) {
            set_key(&mut self.windows_keys, vk, false);
            // Windows gets the release it is owed; if the key was pressed again while capturing, the PS5 too.
            let changed = self.on && key_held(&self.keys, vk);
            set_key(&mut self.keys, vk, false);
            return Outcome { verdict: Verdict::Pass, changed, toggled: false, panic: false };
        }
        if !self.on {
            if down {
                set_key(&mut self.windows_keys, vk, true);
            }
            return PASS;
        }
        let changed = key_held(&self.keys, vk) != down;
        set_key(&mut self.keys, vk, down);
        Outcome { verdict: Verdict::Swallow, changed, toggled: false, panic: false }
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
        Outcome { verdict: Verdict::Swallow, changed: before != self.buttons, toggled: false, panic: false }
    }

    /// Relative motion in pixels (down positive).
    pub fn motion(&mut self, dx: i32, dy: i32) -> Outcome {
        if !self.on {
            return PASS;
        }
        self.motion_x = self.motion_x.wrapping_add(dx as u32);
        self.motion_y = self.motion_y.wrapping_add(dy as u32);
        Outcome { verdict: Verdict::Swallow, changed: dx != 0 || dy != 0, toggled: false, panic: false }
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
        Outcome { verdict: Verdict::Swallow, changed: detents != 0, toggled: false, panic: false }
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
        assert_eq!(c.key(0xa2, true, 0).verdict, Verdict::Pass.max_if(c.on));
        assert_eq!(c.key(0xa4, true, 0).verdict, Verdict::Pass.max_if(c.on));
        let o = c.key(VK_F12, true, 0);
        assert!(o.toggled && o.changed && o.verdict == Verdict::Swallow);
        assert_eq!(c.key(VK_F12, false, 0).verdict, Verdict::Swallow);
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
        assert_eq!(c.key(b'W', true, 0), PASS);
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
        assert_eq!(c.key(0xa2, false, 0).verdict, Verdict::Pass);
        assert_eq!(c.key(0xa4, false, 0).verdict, Verdict::Pass);
        assert_eq!(c.packet(1, 1).keys, [0; 32]);
    }

    #[test]
    fn capture_sends_and_swallows() {
        let mut c = Capture::default();
        toggle(&mut c);
        c.key(0xa2, false, 0);
        c.key(0xa4, false, 0);
        let o = c.key(b'W', true, 0);
        assert!(o.changed && o.verdict == Verdict::Swallow);
        assert!(!c.key(b'W', true, 0).changed, "auto-repeat is no change");
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
        c.key(b'W', true, 0);
        c.button(0, true);
        // Ctrl and Alt are still held from the start; F12 again stops.
        toggle(&mut c);
        assert!(!c.on);
        let p = c.packet(1, 3);
        assert_eq!((p.flags, p.keys, p.buttons), (0, [0; 32], 0));
        // The W and the button were never Windows'; their releases now pass (harmless).
        assert_eq!(c.key(b'W', false, 0), PASS);
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
    #[test]
    fn long_esc_panics() {
        let mut c = Capture::default();
        toggle(&mut c);
        c.key(0xa2, false, 0);
        c.key(0xa4, false, 0);
        c.key(b'W', true, 0);
        // A short Esc goes to the game.
        let o = c.key(VK_ESCAPE, true, 1000);
        assert!(o.changed && !o.toggled && o.verdict == Verdict::Swallow);
        assert!(!c.tick(1000 + PANIC_MS - 1).toggled);
        assert!(c.key(VK_ESCAPE, false, 1000 + PANIC_MS - 1).changed);
        assert!(c.on && !c.tick(10_000).toggled, "a released Esc never panics");
        // Held: the timer stops capturing, everything is released, and the Esc's release is nobody's.
        c.key(VK_ESCAPE, true, 20_000);
        let o = c.tick(20_000 + PANIC_MS);
        assert!(o.panic && o.toggled && o.changed && !c.on);
        assert_eq!(c.packet(1, 1).keys, [0; 32]);
        assert_eq!(c.key(VK_ESCAPE, false, 20_000 + PANIC_MS + 5).verdict, Verdict::Swallow);
        assert_eq!(c.key(VK_ESCAPE, false, 30_000), PASS);
    }

    #[test]
    fn long_esc_panics_on_key_repeat_too() {
        let mut c = Capture::default();
        c.start();
        c.key(VK_ESCAPE, true, 0);
        assert!(!c.key(VK_ESCAPE, true, 500).toggled);
        let o = c.key(VK_ESCAPE, true, PANIC_MS);
        assert!(o.panic && !c.on);
    }

    #[test]
    fn a_held_shortcut_toggles_once() {
        let mut c = Capture::default();
        c.key(0xa2, true, 0);
        c.key(0xa4, true, 0);
        assert!(c.key(VK_F12, true, 0).toggled);
        // Auto-repeat while the shortcut is held: no more toggles.
        for t in 1..20 {
            assert_eq!(c.key(VK_F12, true, t * 33), SWALLOW);
        }
        assert!(c.on);
        // Ctrl and Alt let go first, F12 still repeating: it never reaches the PS5.
        c.key(0xa2, false, 700);
        c.key(0xa4, false, 700);
        assert_eq!(c.key(VK_F12, true, 733), SWALLOW);
        assert_eq!(c.key(VK_F12, false, 760), SWALLOW);
        assert!(!key_held(&c.packet(1, 1).keys, VK_F12));
        // The next F12 is an ordinary key again.
        assert!(c.key(VK_F12, true, 900).changed);
        assert!(c.key(VK_F12, false, 950).changed);
        assert!(!key_held(&c.packet(1, 2).keys, VK_F12));
    }

    #[test]
    fn esc_repeats_after_a_panic_are_nobodys() {
        let mut c = Capture::default();
        c.start();
        c.key(VK_ESCAPE, true, 0);
        for t in [500, 533, 566, 1000, 1400] {
            c.key(VK_ESCAPE, true, t);
        }
        assert!(c.tick(PANIC_MS).panic);
        // Esc still held after the panic: its repeats and its release reach neither Windows nor the PS5.
        assert_eq!(c.key(VK_ESCAPE, true, PANIC_MS + 33), SWALLOW);
        assert_eq!(c.key(VK_ESCAPE, false, PANIC_MS + 100), SWALLOW);
        // The next session: a tap of Esc is pressed and released on the PS5.
        c.start();
        assert!(c.key(VK_ESCAPE, true, 10_000).changed);
        assert!(c.key(VK_ESCAPE, false, 10_050).changed);
        assert!(!key_held(&c.packet(1, 1).keys, VK_ESCAPE));
    }

    #[test]
    fn a_lost_release_does_not_eat_the_next_press() {
        // Ctrl+Alt+F12 starts capturing, but the F12 release never comes (the secure desktop took it).
        let mut c = Capture::default();
        c.key(0xa2, true, 0);
        c.key(0xa4, true, 0);
        assert!(c.key(VK_F12, true, 0).toggled);
        // Much later, Ctrl+Alt+F12 again: a new press, it stops capturing.
        assert!(c.key(VK_F12, true, 5_000).toggled);
        assert!(!c.on);
        // A panic whose Esc release was lost: a later Esc is Windows' again.
        let mut c = Capture::default();
        c.start();
        c.key(VK_ESCAPE, true, 0);
        assert!(c.tick(PANIC_MS).panic);
        assert_eq!(c.key(VK_ESCAPE, true, 9_000), PASS);
        assert_eq!(c.key(VK_ESCAPE, false, 9_050), PASS);
    }

    #[test]
    fn a_key_windows_holds_is_released_on_the_ps5_too() {
        let mut c = Capture::default();
        // Q goes down before capture (Windows has it), then is pressed again while capturing.
        c.key(b'Q', true, 0);
        toggle(&mut c);
        c.key(0xa2, false, 0);
        c.key(0xa4, false, 0);
        c.key(b'Q', true, 10);
        let o = c.key(b'Q', false, 20);
        assert!(o.changed && o.verdict == Verdict::Pass);
        assert!(!key_held(&c.packet(1, 1).keys, b'Q'));
    }

    #[test]
    fn a_stale_panic_flag_does_not_eat_the_next_esc() {
        let mut c = Capture::default();
        c.start();
        c.key(VK_ESCAPE, true, 0);
        assert!(c.tick(PANIC_MS).panic);
        // The web page drops the release while stopped, so the flag is still set when capture starts again.
        c.start();
        c.key(VK_ESCAPE, true, 10_000);
        let o = c.key(VK_ESCAPE, false, 10_100);
        assert!(o.changed && o.verdict == Verdict::Swallow);
        assert!(!crate::protocol::key_held(&c.packet(1, 1).keys, VK_ESCAPE));
    }

    #[test]
    fn esc_repeat_after_timer_panic_without_prior_repeats_is_swallowed() {
        let mut c = Capture::default();
        c.start();
        c.key(VK_ESCAPE, true, 0);
        assert!(c.tick(PANIC_MS).panic);
        // Even if no auto-repeat arrived before the panic timer fired, repeats right after are swallowed.
        assert_eq!(c.key(VK_ESCAPE, true, PANIC_MS + 33), SWALLOW);
        assert_eq!(c.key(VK_ESCAPE, false, PANIC_MS + 100), SWALLOW);
        // A later Esc press is Windows' again.
        assert_eq!(c.key(VK_ESCAPE, true, PANIC_MS + 200), PASS);
        assert_eq!(c.key(VK_ESCAPE, false, PANIC_MS + 250), PASS);
    }

    #[test]
    fn f12_held_before_shortcut_still_releases_to_windows() {
        let mut c = Capture::default();
        // F12 pressed while not capturing (Windows owns it), then Ctrl+Alt added and F12 repeats.
        assert_eq!(c.key(VK_F12, true, 0), PASS);
        assert_eq!(c.key(0xa2, true, 100), PASS);
        assert_eq!(c.key(0xa4, true, 120), PASS);
        assert!(c.key(VK_F12, true, 500).toggled);
        assert_eq!(c.key(VK_F12, true, 533), SWALLOW);
        // Releasing F12 must pass the release back to Windows so F12 does not stay stuck there.
        assert_eq!(c.key(VK_F12, false, 560), PASS);
        assert!(!key_held(&c.packet(1, 1).keys, VK_F12));
    }
}
