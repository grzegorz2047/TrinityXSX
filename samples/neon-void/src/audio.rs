// SPDX-License-Identifier: MIT
//! Procedural XAudio2 PCM16 arcade synthesizer (`xaudio2_9.dll` -> XCL -> WASAPI -> PS5).

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

unsafe fn vslot<F: Copy>(obj: *mut c_void, slot: usize) -> F {
    let vt = *(obj as *const *const usize);
    std::mem::transmute_copy(&*vt.add(slot))
}

#[derive(Clone, Copy)]
pub enum Sfx {
    Laser = 0,
    Pop = 1,
    Emp = 2,
    Wave = 3,
    Hit = 4,
}

pub struct SynthAudio {
    _engine: *mut c_void,
    _master: *mut c_void,
    voices: Vec<*mut c_void>,
    next_voice: usize,
    clips: [Vec<u8>; 5],
}

impl SynthAudio {
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
                tag: 1, // WAVE_FORMAT_PCM
                channels: 1,
                rate: 48_000,
                avg_bytes: 96_000,
                block_align: 2,
                bits: 16,
                cb_size: 0,
            };
            let create_src: CreateSourceVoiceFn = vslot(engine, 5);
            let mut voices = Vec::new();
            for _ in 0..6 {
                let mut v: *mut c_void = std::ptr::null_mut();
                if create_src(
                    engine,
                    &mut v,
                    &wfx as *const _ as *const c_void,
                    0,
                    2.0,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                ) >= 0
                    && !v.is_null()
                {
                    let set_vol: SetVolumeFn = vslot(v, 12);
                    let _ = set_vol(v, 0.28, 0);
                    let start: StartFn = vslot(v, 19);
                    let _ = start(v, 0, 0);
                    voices.push(v);
                }
            }
            if voices.is_empty() {
                return None;
            }

            let clips = [
                gen_sweep(0.065, 1180.0, 340.0, 0.35, false), // Laser
                gen_sweep(0.11, 460.0, 90.0, 0.50, true),     // Enemy Pop
                gen_sweep(0.45, 220.0, 38.0, 0.80, true),     // EMP Nova Boom
                gen_arpeggio(),                               // Wave Clear
                gen_sweep(0.24, 170.0, 52.0, 0.72, true),     // Shield Hit
            ];

            Some(Self { _engine: engine, _master: master, voices, next_voice: 0, clips })
        }
    }

    pub fn play(&mut self, sfx: Sfx) {
        if self.voices.is_empty() {
            return;
        }
        let voice = self.voices[self.next_voice % self.voices.len()];
        self.next_voice = self.next_voice.wrapping_add(1);
        let data = &self.clips[sfx as usize];
        let buf = XAudio2Buffer {
            flags: 0x40, // XAUDIO2_END_OF_STREAM
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
            let stop: StopFn = vslot(voice, 20);
            let flush: FlushBuffersFn = vslot(voice, 22);
            let submit: SubmitFn = vslot(voice, 21);
            let start: StartFn = vslot(voice, 19);
            let _ = stop(voice, 0, 0);
            let _ = flush(voice);
            let _ = submit(voice, &buf, std::ptr::null());
            let _ = start(voice, 0, 0);
        }
    }
}

fn gen_sweep(secs: f32, f0: f32, f1: f32, amp: f32, noise: bool) -> Vec<u8> {
    let n = (secs * 48_000.0) as usize;
    let mut out = Vec::with_capacity(n * 2);
    let mut phase = 0.0f32;
    let mut rng = 0x1234_5678u32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let freq = f0 + (f1 - f0) * t;
        phase += TAU * freq / 48_000.0;
        let env = (1.0 - t).powf(1.6);
        let mut s = phase.sin();
        if noise {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            let nval = ((rng >> 16) as f32 / 32768.0) - 1.0;
            s = s * 0.65 + nval * 0.35;
        }
        let sample = (s * env * amp * 32767.0).clamp(-32767.0, 32767.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

fn gen_arpeggio() -> Vec<u8> {
    let notes = [523.25f32, 659.25, 783.99, 1046.50];
    let per = 4800; // 100 ms per note
    let mut out = Vec::with_capacity(notes.len() * per * 2);
    for &freq in &notes {
        for i in 0..per {
            let t = i as f32 / per as f32;
            let env = (1.0 - t * 0.65) * (t * 25.0).min(1.0);
            let s = (TAU * freq * (i as f32 / 48_000.0)).sin() * env * 0.42;
            let sample = (s * 32767.0) as i16;
            out.extend_from_slice(&sample.to_le_bytes());
        }
    }
    out
}
