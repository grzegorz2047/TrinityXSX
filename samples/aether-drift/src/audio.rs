// SPDX-License-Identifier: MIT
//! Procedural XAudio2 racing engine: continuous pitch-shifted anti-gravity turbine,
//! driving synthwave bass pulse, and arcade SFX.

use std::f32::consts::TAU;
use std::ffi::c_void;

#[link(name = "xaudio2_9", kind = "raw-dylib")]
extern "system" {
    fn XAudio2Create(pp: *mut *mut c_void, flags: u32, processor: u32) -> i32;
}

#[repr(C, packed)]
#[derive(Clone, Copy)]
struct WaveFormatEx {
    tag: u16,
    channels: u16,
    rate: u32,
    avg_bytes: u32,
    block_align: u16,
    bits: u16,
    cb_size: u16,
}

#[repr(C, packed)]
struct XAudio2Buffer {
    flags: u32,
    audio_bytes: u32,
    audio_data: *const u8,
    play_begin: u32,
    play_length: u32,
    loop_begin: u32,
    loop_length: u32,
    loop_count: u32,
    context: *mut c_void,
}

type CreateMasteringVoiceFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void, u32, u32, u32, *const u16, *const c_void, u32) -> i32;
type CreateSourceVoiceFn = unsafe extern "system" fn(*mut c_void, *mut *mut c_void, *const c_void, u32, f32, *const c_void, *const c_void, *const c_void) -> i32;
type SetVolumeFn = unsafe extern "system" fn(*mut c_void, f32, u32) -> i32;
type StartFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;
type StopFn = unsafe extern "system" fn(*mut c_void, u32, u32) -> i32;
type FlushBuffersFn = unsafe extern "system" fn(*mut c_void) -> i32;
type SubmitFn = unsafe extern "system" fn(*mut c_void, *const XAudio2Buffer, *const c_void) -> i32;
type SetFreqRatioFn = unsafe extern "system" fn(*mut c_void, f32, u32) -> i32;

unsafe fn vslot<F: Copy>(obj: *mut c_void, slot: usize) -> F {
    let vt = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vt.add(slot))
}

#[derive(Clone, Copy)]
pub enum Sfx {
    Beep = 0,
    Go = 1,
    BoostPad = 2,
    Turbo = 3,
    Scrape = 4,
    Lap = 5,
}

pub struct RaceAudio {
    _engine: *mut c_void,
    _master: *mut c_void,
    turbine_voice: *mut c_void,
    _turbine_buf: Vec<u8>,
    music_voice: *mut c_void,
    _music_buf: Vec<u8>,
    sfx_voices: Vec<*mut c_void>,
    next_sfx: usize,
    clips: [Vec<u8>; 6],
}

impl RaceAudio {
    pub fn init() -> Option<Self> {
        unsafe {
            let mut engine: *mut c_void = std::ptr::null_mut();
            if XAudio2Create(&mut engine, 0, 1) < 0 || engine.is_null() {
                return None;
            }
            let mut master: *mut c_void = std::ptr::null_mut();
            let create_master: CreateMasteringVoiceFn = vslot(engine, 7);
            if create_master(engine, &mut master, 2, 48_000, 0, std::ptr::null(), std::ptr::null(), 0) < 0 || master.is_null() {
                return None;
            }

            let wfx = WaveFormatEx {
                tag: 1,
                channels: 1,
                rate: 48_000,
                avg_bytes: 96_000,
                block_align: 2,
                bits: 16,
                cb_size: 0,
            };
            let create_src: CreateSourceVoiceFn = vslot(engine, 5);
            let make_voice = |max_freq: f32, vol: f32| -> Option<*mut c_void> {
                let mut v = std::ptr::null_mut();
                if create_src(engine, &mut v, &wfx as *const _ as *const c_void, 0, max_freq, std::ptr::null(), std::ptr::null(), std::ptr::null()) >= 0
                    && !v.is_null()
                {
                    let set_vol: SetVolumeFn = vslot(v, 12);
                    let _ = set_vol(v, vol, 0);
                    Some(v)
                } else {
                    None
                }
            };

            let turbine_voice = make_voice(3.0, 0.14)?;
            let turbine_buf = gen_turbine_loop();
            let tbuf = XAudio2Buffer {
                flags: 0,
                audio_bytes: turbine_buf.len() as u32,
                audio_data: turbine_buf.as_ptr(),
                play_begin: 0,
                play_length: 0,
                loop_begin: 0,
                loop_length: 0,
                loop_count: 255, // XAUDIO2_LOOP_INFINITE
                context: std::ptr::null_mut(),
            };
            let _ = vslot::<SubmitFn>(turbine_voice, 21)(turbine_voice, &tbuf, std::ptr::null());
            let _ = vslot::<StartFn>(turbine_voice, 19)(turbine_voice, 0, 0);

            let music_voice = make_voice(2.0, 0.16)?;
            let music_buf = gen_synthwave_loop();
            let mbuf = XAudio2Buffer {
                flags: 0,
                audio_bytes: music_buf.len() as u32,
                audio_data: music_buf.as_ptr(),
                play_begin: 0,
                play_length: 0,
                loop_begin: 0,
                loop_length: 0,
                loop_count: 255,
                context: std::ptr::null_mut(),
            };
            let _ = vslot::<SubmitFn>(music_voice, 21)(music_voice, &mbuf, std::ptr::null());
            let _ = vslot::<StartFn>(music_voice, 19)(music_voice, 0, 0);

            let mut sfx_voices = Vec::new();
            for _ in 0..5 {
                if let Some(v) = make_voice(2.0, 0.30) {
                    let _ = vslot::<StartFn>(v, 19)(v, 0, 0);
                    sfx_voices.push(v);
                }
            }

            let clips = [
                gen_tone(0.11, 520.0, 520.0, 0.45, false),     // Beep
                gen_tone(0.32, 880.0, 1046.5, 0.58, false),    // GO!
                gen_tone(0.24, 340.0, 1480.0, 0.55, true),     // Boost Pad
                gen_tone(0.50, 190.0, 740.0, 0.65, true),      // Turbo Ignition
                gen_tone(0.14, 150.0, 95.0, 0.45, true),       // Wall Scrape
                gen_lap_chime(),                               // Lap / Finish
            ];

            Some(Self {
                _engine: engine,
                _master: master,
                turbine_voice,
                _turbine_buf: turbine_buf,
                music_voice,
                _music_buf: music_buf,
                sfx_voices,
                next_sfx: 0,
                clips,
            })
        }
    }

    pub fn set_turbine(&self, speed_ratio: f32, boosting: bool, active: bool) {
        unsafe {
            let ratio = (0.62 + speed_ratio.clamp(0.0, 1.4) * 1.15 + if boosting { 0.25 } else { 0.0 }).clamp(0.5, 2.4);
            let vol = if active {
                (0.08 + speed_ratio.clamp(0.0, 1.2) * 0.14 + if boosting { 0.07 } else { 0.0 }).min(0.32)
            } else {
                0.03
            };
            let _ = vslot::<SetFreqRatioFn>(self.turbine_voice, 26)(self.turbine_voice, ratio, 0);
            let _ = vslot::<SetVolumeFn>(self.turbine_voice, 12)(self.turbine_voice, vol, 0);
            let mvol = if active { 0.16 } else { 0.11 };
            let _ = vslot::<SetVolumeFn>(self.music_voice, 12)(self.music_voice, mvol, 0);
        }
    }

    pub fn play(&mut self, sfx: Sfx) {
        if self.sfx_voices.is_empty() {
            return;
        }
        let v = self.sfx_voices[self.next_sfx % self.sfx_voices.len()];
        self.next_sfx = self.next_sfx.wrapping_add(1);
        let data = &self.clips[sfx as usize];
        let buf = XAudio2Buffer {
            flags: 0x40,
            audio_bytes: data.len() as u32,
            audio_data: data.as_ptr(),
            play_begin: 0,
            play_length: 0,
            loop_begin: 0,
            loop_length: 0,
            loop_count: 0,
            context: std::ptr::null_mut(),
        };
        unsafe {
            let _ = vslot::<StopFn>(v, 20)(v, 0, 0);
            let _ = vslot::<FlushBuffersFn>(v, 22)(v);
            let _ = vslot::<SubmitFn>(v, 21)(v, &buf, std::ptr::null());
            let _ = vslot::<StartFn>(v, 19)(v, 0, 0);
        }
    }
}

fn gen_turbine_loop() -> Vec<u8> {
    // Seamless 0.5s futuristic AG turbine hum (harmonic chord + soft air hiss)
    let n = 24_000;
    let mut out = Vec::with_capacity(n * 2);
    let mut rng = 0x5A3C_9102u32;
    for i in 0..n {
        let t = i as f32 / 48_000.0;
        let s1 = (TAU * 110.0 * t).sin() * 0.45;
        let s2 = (TAU * 220.0 * t).sin() * 0.28;
        let s3 = (TAU * 330.0 * t).sin() * 0.14;
        let s4 = (TAU * 660.0 * t).sin() * 0.08;
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        let air = (((rng >> 16) as f32 / 32768.0) - 1.0) * 0.08;
        let sample = ((s1 + s2 + s3 + s4 + air) * 0.72 * 32767.0).clamp(-32767.0, 32767.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn gen_synthwave_loop() -> Vec<u8> {
    // 128 BPM 4-bar driving cyberpunk bassline + soft kick pulse (7.5s seamless loop)
    let bpm = 128.0f32;
    let step_secs = 60.0 / bpm / 4.0; // 16th note
    let steps = 64usize;
    let step_samples = (step_secs * 48_000.0) as usize;
    let mut out = Vec::with_capacity(steps * step_samples * 2);
    // D minor cyberpunk progression: D2 -> Bb1 -> C2 -> A1
    let roots = [73.42f32, 58.27, 65.41, 55.00];
    let mut phase = 0.0f32;
    for st in 0..steps {
        let bar = st / 16;
        let in_bar = st % 16;
        let octave = if in_bar % 4 == 2 { 2.0 } else { 1.0 };
        let freq = roots[bar % 4] * octave;
        let is_kick = in_bar % 4 == 0;
        for i in 0..step_samples {
            let t = i as f32 / step_samples as f32;
            phase = (phase + freq / 48_000.0) % 1.0;
            // Warm filtered sawtooth bass
            let saw = (phase * 2.0 - 1.0) * 0.65 + (TAU * phase).sin() * 0.35;
            let env = (1.0 - t * 0.72) * (t * 20.0).min(1.0);
            let mut mix = saw * env * 0.42;
            if is_kick && t < 0.65 {
                let kt = t / 0.65;
                let kf = 120.0 * (1.0 - kt).powf(2.0) + 42.0;
                mix += (TAU * kf * (i as f32 / 48_000.0)).sin() * (1.0 - kt).powf(1.5) * 0.55;
            }
            let sample = (mix * 0.75 * 32767.0).clamp(-32767.0, 32767.0) as i16;
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    out
}

fn gen_tone(secs: f32, f0: f32, f1: f32, amp: f32, noise: bool) -> Vec<u8> {
    let n = (secs * 48_000.0) as usize;
    let mut out = Vec::with_capacity(n * 2);
    let mut phase = 0.0f32;
    let mut rng = 0x1357_2468u32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        phase += TAU * (f0 + (f1 - f0) * t) / 48_000.0;
        let env = (1.0 - t).powf(1.4);
        let mut s = phase.sin();
        if noise {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            s = s * 0.65 + ((((rng >> 16) as f32) / 32768.0) - 1.0) * 0.35;
        }
        let sample = (s * env * amp * 32767.0).clamp(-32767.0, 32767.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn gen_lap_chime() -> Vec<u8> {
    let notes = [587.33f32, 783.99, 1174.66];
    let per = 4800;
    let mut out = Vec::with_capacity(notes.len() * per * 2);
    for &f in &notes {
        for i in 0..per {
            let t = i as f32 / per as f32;
            let env = (1.0 - t * 0.6) * (t * 20.0).min(1.0);
            let s = (TAU * f * (i as f32 / 48_000.0)).sin() * env * 0.45;
            out.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
        }
    }
    out
}
