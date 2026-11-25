// SPDX-License-Identifier: MIT
//! AETHER DRIFT — Open-source 3D 1080p 60 FPS anti-gravity racer for Xbox Series X|S (Scarlett)
//! running natively on PS5 via TrinityXSX.

mod assets;
mod audio;
mod hud;
mod math;
mod scarlett;
mod track;

use std::collections::VecDeque;
use std::fs;
use std::path::Path;
use std::time::Instant;

use assets::{emit_mesh_reflected, emit_mesh_world, GameAssets};
use audio::{RaceAudio, Sfx};
use hud::{fmt_time, HudBatch};
use math::{Mat4, Vec3};
use scarlett::{
    init_console_runtime, ConsoleIdentity, Pad, Renderer, Vertex3D, BTN_CIRCLE, BTN_CROSS,
    BTN_DOWN, BTN_L1, BTN_LEFT, BTN_R1, BTN_RIGHT, BTN_SQUARE, BTN_START, BTN_TRIANGLE, BTN_UP,
    SCREEN_H, SCREEN_W,
};
use track::{Circuit, TRACKS, TRACK_COUNT, TRACK_HALF_WIDTH};

const TOTAL_LAPS: u32 = 3;
const TOTAL_RACERS: usize = 8;
const TRAIL_POINTS: usize = 22;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RaceMode {
    GrandPrix,
    TimeTrial,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Menu,
    Countdown,
    Racing,
    Finished,
}

struct Racer {
    is_player: bool,
    s: f32,           // total progress along spline (can exceed circuit.total_len across laps)
    lateral: f32,     // -TRACK_HALF_WIDTH .. +TRACK_HALF_WIDTH
    lat_vel: f32,
    speed: f32,       // world units / sec (1 unit/s ~= 3.6 km/h -> 250 u/s = 900 km/h)
    roll: f32,
    pitch: f32,
    boost_energy: f32,
    boost_timer: f32,
    pad_cooldown: f32,
    lap: u32,
    finished: bool,
    finish_time: f32,
    ai_target_lat: f32,
    ai_skill: f32,
    ai_lane_timer: f32,
    body_col: [f32; 4],
    glow_col: [f32; 4],
    trail: VecDeque<(Vec3, Vec3)>, // (world_pos, right_vec)
}

struct Spark {
    pos: Vec3,
    vel: Vec3,
    life: f32,
    col: [f32; 4],
}

#[derive(Clone, Copy)]
struct GhostFrame {
    s: f32,
    lat: f32,
    roll: f32,
}

#[derive(Default)]
struct TrackRecords {
    best_race: [f32; TRACK_COUNT],
    best_lap: [f32; TRACK_COUNT],
    ghosts: [Vec<GhostFrame>; TRACK_COUNT],
}

impl TrackRecords {
    fn load(pls: Option<&Path>) -> Self {
        let mut r = Self::default();
        let Some(dir) = pls else { return r };
        if let Ok(txt) = fs::read_to_string(dir.join("records.txt")) {
            for line in txt.lines() {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() == 3 {
                    if let Some(idx) = TRACKS.iter().position(|t| t.id == parts[0]) {
                        r.best_race[idx] = parts[1].parse().unwrap_or(0.0);
                        r.best_lap[idx] = parts[2].parse().unwrap_or(0.0);
                    }
                }
            }
        }
        for (idx, meta) in TRACKS.iter().enumerate() {
            if let Ok(txt) = fs::read_to_string(dir.join(format!("ghost_{}.txt", meta.id))) {
                for line in txt.lines() {
                    let p: Vec<&str> = line.split(',').collect();
                    if p.len() == 3 {
                        if let (Ok(s), Ok(lat), Ok(roll)) = (p[0].parse(), p[1].parse(), p[2].parse()) {
                            r.ghosts[idx].push(GhostFrame { s, lat, roll });
                        }
                    }
                }
            }
        }
        r
    }

    fn save(&self, pls: Option<&Path>, gamertag: &str) {
        let Some(dir) = pls else { return };
        let _ = fs::create_dir_all(dir);
        let mut body = format!("# AETHER DRIFT PLS SAVE ({gamertag})\n");
        for (i, meta) in TRACKS.iter().enumerate() {
            body.push_str(&format!("{},{:.3},{:.3}\n", meta.id, self.best_race[i], self.best_lap[i]));
        }
        let _ = fs::write(dir.join("records.txt"), body);
    }

    fn save_ghost(&self, pls: Option<&Path>, track_idx: usize) {
        let Some(dir) = pls else { return };
        let _ = fs::create_dir_all(dir);
        let mut body = String::new();
        for g in &self.ghosts[track_idx] {
            body.push_str(&format!("{:.2},{:.2},{:.3}\n", g.s, g.lat, g.roll));
        }
        let _ = fs::write(dir.join(format!("ghost_{}.txt", TRACKS[track_idx].id)), body);
    }
}

struct App {
    assets: GameAssets,
    phase: Phase,
    mode: RaceMode,
    track_idx: usize,
    circuit: Circuit,
    racers: Vec<Racer>,
    sparks: Vec<Spark>,
    records: TrackRecords,
    cur_ghost: Vec<GhostFrame>,
    ghost_sample_acc: f32,
    time: f32,
    countdown: f32,
    last_beep: i32,
    race_time: f32,
    lap_start_time: f32,
    best_lap_this_race: f32,
    new_record: bool,
    lap_banner: f32,
    lap_banner_txt: String,
    cam_pos: Vec3,
    cam_target: Vec3,
    cam_up: Vec3,
    cam_fov: f32,
    cam_shake: f32,
    rng: u64,
}

impl App {
    fn new(renderer: &mut Renderer, id: &ConsoleIdentity) -> Result<Self, String> {
        let assets = GameAssets::load_meshes();
        let circuit = Circuit::build(0, &assets.skyscraper);
        renderer.upload_static_world(&circuit.static_world)?;
        let records = TrackRecords::load(id.pls_dir.as_deref());
        records.save(id.pls_dir.as_deref(), &id.gamertag);

        let mut app = Self {
            assets,
            phase: Phase::Menu,
            mode: RaceMode::GrandPrix,
            track_idx: 0,
            circuit,
            racers: Vec::new(),
            sparks: Vec::with_capacity(256),
            records,
            cur_ghost: Vec::new(),
            ghost_sample_acc: 0.0,
            time: 0.0,
            countdown: 3.2,
            last_beep: 4,
            race_time: 0.0,
            lap_start_time: 0.0,
            best_lap_this_race: 0.0,
            new_record: false,
            lap_banner: 0.0,
            lap_banner_txt: String::new(),
            cam_pos: Vec3::new(0.0, 50.0, -40.0),
            cam_target: Vec3::new(0.0, 26.0, 40.0),
            cam_up: Vec3::UP,
            cam_fov: 64.0f32.to_radians(),
            cam_shake: 0.0,
            rng: 0xAE74_0001_2026_1125,
        };
        app.spawn_grid();

            // Optional PC screenshot demo mode (AETHER_AUTO_DEMO=1) jumps right into banked-curve action!
        if std::env::var_os("AETHER_AUTO_DEMO").is_some() {
            app.phase = Phase::Racing;
            app.race_time = 74.32;
            app.lap_start_time = 74.0;
            let base_s = app.circuit.total_len * 0.145;
            for (i, r) in app.racers.iter_mut().enumerate() {
                r.lap = 2;
                r.speed = if r.is_player { 234.0 } else { 230.0 };
                r.boost_timer = if r.is_player { 2.5 } else { 0.0 };
                r.s = match i {
                    0 => base_s,
                    1 => base_s + 26.0,
                    2 => base_s + 52.0,
                    3 => base_s + 78.0,
                    _ => base_s - 22.0 * (i as f32),
                };
                r.lateral = match i {
                    0 => 3.5,
                    1 => -1.5,
                    2 => 5.2,
                    3 => -4.8,
                    _ => ((i as f32) - 4.0) * 2.5,
                };
                r.roll = -0.22;
            }
        }
        Ok(app)
    }

    fn next_f(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((self.rng >> 32) as f32) / (u32::MAX as f32)
    }

    fn load_track(&mut self, idx: usize, renderer: &mut Renderer) -> Result<(), String> {
        self.track_idx = idx % TRACK_COUNT;
        self.circuit = Circuit::build(self.track_idx, &self.assets.skyscraper);
        renderer.upload_static_world(&self.circuit.static_world)?;
        self.spawn_grid();
        Ok(())
    }

    fn spawn_grid(&mut self) {
        self.racers.clear();
        self.sparks.clear();
        self.cur_ghost.clear();
        self.ghost_sample_acc = 0.0;

        let palettes: [([f32; 4], [f32; 4]); TOTAL_RACERS] = [
            ([0.42, 0.68, 0.78, 1.0], [0.16, 0.96, 1.00, 1.0]), // Player: Metallic Titanium-Cyan + Neon Cyan
            ([0.82, 0.36, 0.10, 1.0], [1.00, 0.62, 0.16, 1.0]), // Rival 1: Solar Orange
            ([0.78, 0.16, 0.48, 1.0], [1.00, 0.30, 0.78, 1.0]), // Rival 2: Cyber Magenta
            ([0.36, 0.92, 0.46, 1.0], [0.44, 1.00, 0.65, 1.0]), // Rival 3: Neon Mint
            ([0.56, 0.34, 0.96, 1.0], [0.74, 0.48, 1.00, 1.0]), // Rival 4: Ultraviolet
            ([0.96, 0.80, 0.18, 1.0], [1.00, 0.92, 0.35, 1.0]), // Rival 5: Chrome Gold
            ([0.94, 0.22, 0.26, 1.0], [1.00, 0.36, 0.36, 1.0]), // Rival 6: Crimson
            ([0.24, 0.62, 0.98, 1.0], [0.38, 0.82, 1.00, 1.0]), // Rival 7: Cobalt
        ];

        let count = if self.mode == RaceMode::GrandPrix { TOTAL_RACERS } else { 1 };
        for i in 0..count {
            let is_player = i == 0;
            // Player starts mid-pack (P5) in Grand Prix for thrilling overtakes, P1 in Time Trial
            let grid_slot = if self.mode == RaceMode::GrandPrix {
                if is_player { 4 } else if i <= 4 { i - 1 } else { i }
            } else {
                0
            };
            let s = self.circuit.total_len - 18.0 - (grid_slot as f32) * 14.0;
            let lat = if grid_slot % 2 == 0 { -5.2 } else { 5.2 };
            let skill = 0.84 + (i as f32 / TOTAL_RACERS as f32) * 0.20;
            self.racers.push(Racer {
                is_player,
                s,
                lateral: lat,
                lat_vel: 0.0,
                speed: 0.0,
                roll: 0.0,
                pitch: 0.0,
                boost_energy: 0.65,
                boost_timer: 0.0,
                pad_cooldown: 0.0,
                lap: 1,
                finished: false,
                finish_time: 0.0,
                ai_target_lat: lat,
                ai_skill: skill,
                ai_lane_timer: 1.5 + i as f32 * 0.4,
                body_col: palettes[i].0,
                glow_col: palettes[i].1,
                trail: VecDeque::with_capacity(TRAIL_POINTS + 2),
            });
        }

        // Snap camera behind player immediately
        let p = &self.racers[0];
        let sm = self.circuit.sample_at(p.s);
        let ship_pos = sm.pos.add(sm.right.mul(p.lateral)).add(sm.up.mul(2.1));
        self.cam_pos = ship_pos.sub(sm.forward.mul(15.0)).add(sm.up.mul(5.8));
        self.cam_target = ship_pos.add(sm.forward.mul(26.0));
        self.cam_up = sm.up;
    }

    fn start_race(&mut self) {
        self.spawn_grid();
        self.phase = Phase::Countdown;
        self.countdown = 3.2;
        self.last_beep = 4;
        self.race_time = 0.0;
        self.lap_start_time = 0.0;
        self.best_lap_this_race = 0.0;
        self.new_record = false;
        self.lap_banner = 0.0;
    }

    fn player_position(&self) -> usize {
        let ps = self.racers[0].s;
        1 + self.racers.iter().skip(1).filter(|r| r.s > ps).count()
    }

    fn update(&mut self, dt: f32, pad: &mut Pad, audio: &mut Option<RaceAudio>, renderer: &mut Renderer, id: &ConsoleIdentity) {
        self.time += dt;
        self.cam_shake = (self.cam_shake - dt * 2.5).max(0.0);
        self.lap_banner = (self.lap_banner - dt).max(0.0);

        // Update wall-scrape sparks
        self.sparks.retain_mut(|sp| {
            sp.pos = sp.pos.add(sp.vel.mul(dt));
            sp.vel.y -= 35.0 * dt;
            sp.life -= dt;
            sp.life > 0.0
        });

        match self.phase {
            Phase::Menu => {
                if pad.pressed(BTN_LEFT) || pad.pressed(BTN_UP) {
                    let next = (self.track_idx + TRACK_COUNT - 1) % TRACK_COUNT;
                    let _ = self.load_track(next, renderer);
                    if let Some(a) = audio.as_mut() { a.play(Sfx::Beep); }
                } else if pad.pressed(BTN_RIGHT) || pad.pressed(BTN_DOWN) {
                    let next = (self.track_idx + 1) % TRACK_COUNT;
                    let _ = self.load_track(next, renderer);
                    if let Some(a) = audio.as_mut() { a.play(Sfx::Beep); }
                } else if pad.pressed(BTN_SQUARE) || pad.pressed(BTN_TRIANGLE) {
                    self.mode = if self.mode == RaceMode::GrandPrix {
                        RaceMode::TimeTrial
                    } else {
                        RaceMode::GrandPrix
                    };
                    if let Some(a) = audio.as_mut() { a.play(Sfx::Beep); }
                } else if pad.pressed(BTN_CROSS) {
                    self.start_race();
                    if let Some(a) = audio.as_mut() { a.play(Sfx::BoostPad); }
                    pad.pulse(0.35, 0.55, 0.14);
                }

                // Scenic cinematic orbit camera sweeping along the glowing circuit
                let s = (self.time * 48.0).rem_euclid(self.circuit.total_len);
                let sm = self.circuit.sample_at(s);
                let ahead = self.circuit.sample_at(s + 52.0);
                self.cam_pos = sm.pos.add(sm.up.mul(13.0)).add(sm.right.mul((self.time * 0.4).sin() * 14.0));
                self.cam_target = ahead.pos.add(ahead.up.mul(3.0));
                self.cam_up = Vec3::UP;
                self.cam_fov = 66.0f32.to_radians();
                if let Some(a) = audio.as_mut() { a.set_turbine(0.15, false, false); }
                pad.update_rumble(0.0, 0.0);
                return;
            }
            Phase::Countdown => {
                self.countdown -= dt;
                let sec = self.countdown.ceil() as i32;
                if sec != self.last_beep && sec >= 1 && sec <= 3 {
                    self.last_beep = sec;
                    if let Some(a) = audio.as_mut() { a.play(Sfx::Beep); }
                    pad.pulse(0.25, 0.25, 0.09);
                }
                if self.countdown <= 0.0 {
                    self.phase = Phase::Racing;
                    if let Some(a) = audio.as_mut() { a.play(Sfx::Go); }
                    pad.pulse(0.85, 0.85, 0.25);
                    self.lap_banner_txt = "GO!".into();
                    self.lap_banner = 1.1;
                }
            }
            Phase::Racing => {
                if pad.pressed(BTN_START) {
                    self.phase = Phase::Menu;
                    return;
                }
                self.race_time += dt;
            }
            Phase::Finished => {
                if pad.pressed(BTN_CROSS) {
                    self.start_race();
                    return;
                }
                if pad.pressed(BTN_CIRCLE) {
                    self.phase = Phase::Menu;
                    return;
                }
            }
        }

        // Update all racers (player + 7 AI hovercrafts)
        let racing = self.phase == Phase::Racing;
        let finished_autopilot = self.phase == Phase::Finished;
        let circuit_len = self.circuit.total_len;
        let player_s = self.racers[0].s;

        let mut spark_spawns: Vec<(Vec3, Vec3, [f32; 4])> = Vec::new();
        let mut sfx_events: Vec<Sfx> = Vec::new();
        let mut player_pulse: Option<(f32, f32, f32)> = None;

        for r in &mut self.racers {
            r.pad_cooldown = (r.pad_cooldown - dt).max(0.0);
            r.boost_timer = (r.boost_timer - dt).max(0.0);

            let prev_s = r.s;
            let sm = self.circuit.sample_at(r.s);

            if racing || finished_autopilot {
                if r.is_player && racing {
                    // DualSense analog throttle (R2 or Cross)
                    let mut throttle = pad.r2;
                    if pad.held(BTN_CROSS) {
                        throttle = throttle.max(1.0);
                    }
                    let brake = pad.l2;
                    let mut steer = pad.lx;
                    if pad.held(BTN_LEFT) { steer -= 1.0; }
                    if pad.held(BTN_RIGHT) { steer += 1.0; }
                    steer = steer.clamp(-1.0, 1.0);

                    // WipEout-style L1 / R1 Airbrakes: snappy lateral carve with tiny speed trim
                    let air_l: f32 = if pad.held(BTN_L1) { 1.0 } else { 0.0 };
                    let air_r: f32 = if pad.held(BTN_R1) { 1.0 } else { 0.0 };
                    let air_net: f32 = air_r - air_l;

                    // Turbo Boost activation (Square / Triangle)
                    if r.boost_energy >= 0.35 && r.boost_timer <= 0.0 && (pad.pressed(BTN_SQUARE) || pad.pressed(BTN_TRIANGLE)) {
                        r.boost_timer = 1.65;
                        r.boost_energy = (r.boost_energy - 0.45).max(0.0);
                        sfx_events.push(Sfx::Turbo);
                        player_pulse = Some((0.90, 0.85, 0.35));
                    }

                    // Recharge boost steadily at high speed & while drafting/carving
                    if r.boost_timer <= 0.0 {
                        r.boost_energy = (r.boost_energy + dt * (0.04 + (r.speed / 260.0) * 0.05)).min(1.0);
                    }

                    let top_speed = if r.boost_timer > 0.0 { 295.0 } else { 222.0 };
                    let accel = if r.boost_timer > 0.0 { 215.0 } else { 112.0 };
                    let target_spd = if throttle > 0.02 {
                        top_speed * (0.30 + 0.70 * throttle)
                    } else {
                        0.0
                    };
                    if r.speed < target_spd {
                        r.speed = (r.speed + accel * dt).min(target_spd);
                    } else {
                        let drag = 36.0 + brake * 185.0 + air_net.abs() * 14.0;
                        r.speed = (r.speed - drag * dt).max(0.0);
                    }

                    // Smooth lateral anti-gravity drift physics
                    let steer_force = steer * 46.0 + air_net * 34.0;
                    r.lat_vel += (steer_force - r.lat_vel * 4.4) * dt;
                    r.lateral += r.lat_vel * dt;
                    let target_roll = (-steer * 0.48 - air_net * 0.32).clamp(-0.78, 0.78);
                    r.roll += (target_roll - r.roll) * (dt * 9.0).min(1.0);
                    r.pitch += ((if r.boost_timer > 0.0 { -0.09 } else { 0.0 }) - r.pitch) * (dt * 6.0).min(1.0);
                } else {
                    // Competitive AI racers (plus player autopilot after crossing finish line)
                    r.ai_lane_timer -= dt;
                    if r.ai_lane_timer <= 0.0 {
                        r.ai_lane_timer = 1.4 + (r.s.sin().abs()) * 2.2;
                        r.ai_target_lat = ((r.s * 0.017 + r.ai_skill * 11.0).sin()) * (TRACK_HALF_WIDTH - 4.5);
                    }
                    // Gentle rubber-banding in Grand Prix so races stay close and exciting
                    let gap = r.s - player_s;
                    let rubber = if r.is_player {
                        1.0
                    } else if gap > 140.0 {
                        0.90
                    } else if gap < -140.0 {
                        1.10
                    } else {
                        1.0
                    };
                    let ai_top = (198.0 + r.ai_skill * 28.0) * rubber + if r.boost_timer > 0.0 { 55.0 } else { 0.0 };
                    r.speed += (ai_top - r.speed) * (dt * 1.4).min(1.0);
                    let diff = r.ai_target_lat - r.lateral;
                    r.lat_vel += (diff * 4.2 - r.lat_vel * 3.8) * dt;
                    r.lateral += r.lat_vel * dt;
                    r.roll += ((-r.lat_vel * 0.025).clamp(-0.55, 0.55) - r.roll) * (dt * 6.0).min(1.0);
                }

                r.s += r.speed * dt;

                // Energy guardrail bounce & wall sparks
                let limit = TRACK_HALF_WIDTH - 2.3;
                if r.lateral.abs() > limit {
                    let sign = r.lateral.signum();
                    r.lateral = sign * limit;
                    r.lat_vel = -sign * r.lat_vel.abs().max(9.0) * 0.45;
                    r.speed *= 0.94;
                    if r.is_player && racing {
                        sfx_events.push(Sfx::Scrape);
                        player_pulse = Some((0.55, 0.70, 0.12));
                        let sp_pos = sm.pos.add(sm.right.mul(r.lateral + sign * 1.5)).add(sm.up.mul(1.8));
                        let sp_vel = sm.forward.mul(r.speed * 0.35).sub(sm.right.mul(sign * 16.0)).add(sm.up.mul(9.0));
                        spark_spawns.push((sp_pos, sp_vel, [1.0, 0.72, 0.22, 1.0]));
                    }
                }

                // Chevron Boost Pad detection
                if r.pad_cooldown <= 0.0 {
                    if let Some(bx) = sm.boost_x {
                        if (r.lateral - bx).abs() < 4.2 {
                            r.pad_cooldown = 0.75;
                            r.boost_timer = r.boost_timer.max(0.85);
                            r.speed = (r.speed + 52.0).min(305.0);
                            r.boost_energy = (r.boost_energy + 0.22).min(1.0);
                            if r.is_player && racing {
                                sfx_events.push(Sfx::BoostPad);
                                player_pulse = Some((0.65, 0.85, 0.22));
                            }
                        }
                    }
                }

                // Lap crossing detection (crossing 0 -> total_len boundary)
                let start_base = circuit_len; // grid starts just before circuit_len
                let prev_lap = ((prev_s - start_base).max(0.0) / circuit_len).floor() as u32 + 1;
                let curr_lap = ((r.s - start_base).max(0.0) / circuit_len).floor() as u32 + 1;
                if r.s >= start_base && curr_lap > prev_lap && !r.finished {
                    if curr_lap > TOTAL_LAPS {
                        r.finished = true;
                        r.finish_time = self.race_time;
                    } else {
                        r.lap = curr_lap;
                    }
                }
            }

            // Record glowing 3D ribbon thruster trail
            let sm_now = self.circuit.sample_at(r.s);
            let hover_bob = (self.time * 5.2 + r.ai_skill * 7.0).sin() * 0.22;
            let ship_pos = sm_now.pos.add(sm_now.right.mul(r.lateral)).add(sm_now.up.mul(2.05 + hover_bob));
            if r.speed > 18.0 {
                let tail = ship_pos.sub(sm_now.forward.mul(3.4));
                r.trail.push_front((tail, sm_now.right));
                while r.trail.len() > TRAIL_POINTS {
                    r.trail.pop_back();
                }
            } else {
                r.trail.clear();
            }
        }

        // Apply player events, lap timing & PLS saving
        for sfx in sfx_events {
            if let Some(a) = audio.as_mut() { a.play(sfx); }
        }
        if let Some((l, h, d)) = player_pulse {
            pad.pulse(l, h, d);
            self.cam_shake = self.cam_shake.max(l * 0.55);
        }
        for (pos, vel, col) in spark_spawns {
            for _ in 0..8 {
                let jitter = Vec3::new((self.next_f() - 0.5) * 18.0, self.next_f() * 14.0, (self.next_f() - 0.5) * 18.0);
                let life = 0.35 + self.next_f() * 0.25;
                self.sparks.push(Spark { pos, vel: vel.add(jitter), life, col });
            }
        }

        // Record ghost telemetry in Time Trial (10 Hz)
        if racing && self.mode == RaceMode::TimeTrial {
            self.ghost_sample_acc += dt;
            if self.ghost_sample_acc >= 0.1 {
                self.ghost_sample_acc -= 0.1;
                let p = &self.racers[0];
                self.cur_ghost.push(GhostFrame { s: p.s, lat: p.lateral, roll: p.roll });
            }
        }

        // Detect player lap increment or finish
        if racing {
            let p_lap = self.racers[0].lap;
            let p_fin = self.racers[0].finished;
            let expected_lap = if self.lap_banner_txt == "FINAL LAP!" {
                TOTAL_LAPS
            } else {
                p_lap
            };
            let _ = expected_lap;

            // Check lap timing transition using lap_start_time
            if p_fin {
                let lap_dt = self.race_time - self.lap_start_time;
                if self.best_lap_this_race == 0.0 || lap_dt < self.best_lap_this_race {
                    self.best_lap_this_race = lap_dt;
                }
                self.phase = Phase::Finished;
                if let Some(a) = audio.as_mut() { a.play(Sfx::Lap); }
                pad.pulse(0.6, 0.9, 0.35);

                let ti = self.track_idx;
                let mut changed = false;
                if self.records.best_race[ti] == 0.0 || self.race_time < self.records.best_race[ti] {
                    self.records.best_race[ti] = self.race_time;
                    self.new_record = true;
                    changed = true;
                    if self.mode == RaceMode::TimeTrial && !self.cur_ghost.is_empty() {
                        self.records.ghosts[ti] = self.cur_ghost.clone();
                        self.records.save_ghost(id.pls_dir.as_deref(), ti);
                    }
                }
                if self.records.best_lap[ti] == 0.0 || self.best_lap_this_race < self.records.best_lap[ti] {
                    self.records.best_lap[ti] = self.best_lap_this_race;
                    changed = true;
                }
                if changed {
                    self.records.save(id.pls_dir.as_deref(), &id.gamertag);
                }
            } else {
                let completed_laps = ((self.racers[0].s - circuit_len).max(0.0) / circuit_len).floor() as u32;
                let prev_completed = if self.lap_start_time == 0.0 { 0 } else { p_lap.saturating_sub(1) };
                if completed_laps > prev_completed || (p_lap == 2 && self.lap_start_time == 0.0) {
                    let lap_dt = self.race_time - self.lap_start_time;
                    if lap_dt > 5.0 {
                        if self.best_lap_this_race == 0.0 || lap_dt < self.best_lap_this_race {
                            self.best_lap_this_race = lap_dt;
                        }
                        self.lap_start_time = self.race_time;
                        self.lap_banner = 2.0;
                        self.lap_banner_txt = if p_lap == TOTAL_LAPS {
                            "FINAL LAP!".into()
                        } else {
                            format!("LAP {p_lap} / {TOTAL_LAPS}")
                        };
                        if let Some(a) = audio.as_mut() { a.play(Sfx::Lap); }
                    }
                }
            }
        }

        // Dynamic Chase Camera locked tightly behind the sculpted hero craft (zero longitudinal lag at 900 km/h!)
        let p = &self.racers[0];
        let sm = self.circuit.sample_at(p.s);
        let ahead = self.circuit.sample_at(p.s + 38.0);
        let hover_bob = (self.time * 5.2 + p.ai_skill * 7.0).sin() * 0.16;
        let ship_pos = sm.pos.add(sm.right.mul(p.lateral)).add(sm.up.mul(1.85 + hover_bob));

        let speed_ratio = (p.speed / 230.0).clamp(0.0, 1.35);
        let back_dist = 10.6 + speed_ratio * 1.4 + if p.boost_timer > 0.0 { 1.3 } else { 0.0 };
        let up_dist = 3.75 + speed_ratio * 0.35;
        let shake_v = if self.cam_shake > 0.01 {
            sm.right.mul((self.time * 67.0).sin() * self.cam_shake * 0.45)
                .add(sm.up.mul((self.time * 53.0).cos() * self.cam_shake * 0.35))
        } else {
            Vec3::ZERO
        };

        let desired_pos = ship_pos
            .sub(sm.forward.mul(back_dist))
            .add(sm.up.mul(up_dist))
            .add(sm.right.mul(-p.roll * 2.2))
            .add(shake_v);
        let desired_tgt = ahead.pos.add(ahead.right.mul(p.lateral * 0.55)).add(ahead.up.mul(2.1));
        let desired_up = sm.up.lerp(Vec3::UP, 0.30).norm();
        let target_fov = (64.0 + speed_ratio * 10.0 + if p.boost_timer > 0.0 { 7.0 } else { 0.0 }).to_radians();

        // Keep exact longitudinal distance behind ship so hero craft always fills foreground like concept art!
        self.cam_pos = desired_pos;
        self.cam_target = self.cam_target.lerp(desired_tgt, (dt * 18.0).min(1.0));
        self.cam_up = self.cam_up.lerp(desired_up, (dt * 8.0).min(1.0)).norm();
        self.cam_fov += (target_fov - self.cam_fov) * (dt * 6.0).min(1.0);

        // Live XAudio2 turbine pitch + subtle continuous high-speed rumble
        if let Some(a) = audio.as_mut() {
            a.set_turbine(speed_ratio, p.boost_timer > 0.0, racing);
        }
        let cont_low = if racing && p.boost_timer > 0.0 { 0.45 } else { 0.0 };
        let cont_high = if racing { (speed_ratio * 0.14).clamp(0.0, 0.18) } else { 0.0 };
        pad.update_rumble(cont_low, cont_high);
    }

    fn build_dynamic_scene(
        &self,
        craft3d: &mut Vec<Vertex3D>,
        dyn_refl3d: &mut Vec<Vertex3D>,
        fx3d: &mut Vec<Vertex3D>,
    ) {
        craft3d.clear();
        dyn_refl3d.clear();
        fx3d.clear();

        // 1. Draw all 8 Sculpted Blender 5.2 Anti-Gravity Hovercrafts + TRUE Wet-Track Mirror Reflections!
        for r in &self.racers {
            let sm = self.circuit.sample_at(r.s);
            let bob = (self.time * 5.2 + r.ai_skill * 7.0).sin() * 0.16;
            let pos = sm.pos.add(sm.right.mul(r.lateral)).add(sm.up.mul(1.85 + bob));
            let plane_origin = sm.pos.add(sm.right.mul(r.lateral));
            let mesh = if r.is_player {
                &self.assets.hero_craft
            } else {
                &self.assets.rival_craft
            };

            let (sr, cr) = r.roll.sin_cos();
            let right = sm.right.mul(cr).add(sm.up.mul(sr)).norm();
            let up0 = sm.forward.cross(right).norm();
            let (sp, cp) = r.pitch.sin_cos();
            let fwd = sm.forward.mul(cp).add(up0.mul(sp)).norm();
            let up = fwd.cross(right).norm();

            emit_mesh_world(
                mesh,
                craft3d,
                pos,
                right,
                up,
                fwd,
                Vec3::new(1.05, 1.05, 1.05),
                r.body_col,
                r.glow_col,
                None,
            );
            // Real 3D mirrored reflection of this hovercraft on the wet track surface!
            emit_mesh_reflected(
                mesh,
                dyn_refl3d,
                pos,
                right,
                up,
                fwd,
                Vec3::new(1.05, 1.05, 1.05),
                plane_origin,
                sm.up,
                r.body_col,
                r.glow_col,
            );

            // Multi-layered volumetric blazing plasma exhaust plumes (+ mirrored plumes on wet track!)
            emit_thruster_plumes(
                fx3d,
                Some(dyn_refl3d),
                pos,
                right,
                up,
                fwd,
                plane_origin,
                sm.up,
                r.is_player,
                r.glow_col,
                r.boost_timer > 0.0,
                (r.speed / 230.0).clamp(0.25, 1.4),
            );
            emit_trail(fx3d, &r.trail, r.glow_col, r.boost_timer > 0.0);
        }

        // 2. Time Trial Ghost Ship (translucent cyan hologram Blender craft!)
        if self.mode == RaceMode::TimeTrial && self.phase == Phase::Racing {
            let ghost = &self.records.ghosts[self.track_idx];
            if !ghost.is_empty() {
                let idx = ((self.race_time * 10.0) as usize).min(ghost.len() - 1);
                let gf = ghost[idx];
                let sm = self.circuit.sample_at(gf.s);
                let pos = sm.pos.add(sm.right.mul(gf.lat)).add(sm.up.mul(2.0));
                let (sr, cr) = gf.roll.sin_cos();
                let right = sm.right.mul(cr).add(sm.up.mul(sr)).norm();
                let up = sm.forward.cross(right).norm();
                emit_mesh_world(
                    &self.assets.hero_craft,
                    fx3d,
                    pos,
                    right,
                    up,
                    sm.forward,
                    Vec3::new(1.05, 1.05, 1.05),
                    [0.22, 0.95, 1.0, 0.35],
                    [0.35, 1.0, 1.0, 0.55],
                    Some(0.32),
                );
            }
        }

        // 3. Wall-scrape sparks
        for sp in &self.sparks {
            let sz = 0.45 * (sp.life / 0.5).clamp(0.2, 1.0);
            let a = sp.pos.sub(Vec3::new(sz, 0.0, 0.0));
            let b = sp.pos.add(Vec3::new(sz, sz, 0.0));
            let c = sp.pos.sub(sp.vel.mul(0.035));
            let col = [sp.col[0], sp.col[1], sp.col[2], sp.life.clamp(0.0, 1.0)];
            fx3d.extend_from_slice(&[
                Vertex3D { pos: a.to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col, mat: [0.0, 2.4] },
                Vertex3D { pos: b.to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col, mat: [0.0, 2.4] },
                Vertex3D { pos: c.to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col, mat: [0.0, 2.4] },
            ]);
        }

        // 4. Diagonal storm rain + hyperspeed motion streaks around camera (concept art rain!)
        let speed_f = if self.phase == Phase::Racing {
            (self.racers[0].speed / 230.0).clamp(0.25, 1.4)
        } else {
            0.35
        };
        let fwd = self.cam_target.sub(self.cam_pos).norm();
        let right = self.cam_up.cross(fwd).norm();
        let up = fwd.cross(right).norm();
        let streak_count = if self.racers[0].boost_timer > 0.0 { 96 } else { 64 };
        for i in 0..streak_count {
            let fi = i as f32;
            let ang = fi * 2.39996;
            let rad = 7.0 + ((fi * 7.13).sin() * 0.5 + 0.5) * 28.0;
            let z_off = ((fi * 19.7 + self.time * (90.0 * speed_f)) % 88.0) + 5.0;
            let z_pos = 93.0 - z_off;
            let fall = (self.time * 42.0 + fi * 13.1) % 36.0;
            let base = self.cam_pos
                .add(fwd.mul(z_pos))
                .add(right.mul(ang.cos() * rad))
                .add(up.mul(ang.sin() * rad * 0.6 + 14.0 - fall));
            let rain_dir = fwd.mul(-0.65 * speed_f).add(up.mul(-1.0)).add(right.mul(-0.18)).norm();
            let len = 2.6 + speed_f * 4.2;
            let tip = base.add(rain_dir.mul(len));
            let w = right.mul(0.055);
            let alpha = (0.14 + speed_f * 0.16) * (1.0 - (z_pos / 93.0)).clamp(0.0, 1.0);
            let col = if self.racers[0].boost_timer > 0.0 {
                [0.52, 0.96, 1.0, alpha]
            } else {
                [0.68, 0.84, 0.98, alpha]
            };
            fx3d.extend_from_slice(&[
                Vertex3D { pos: base.sub(w).to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col, mat: [0.0, 1.2] },
                Vertex3D { pos: base.add(w).to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col, mat: [0.0, 1.2] },
                Vertex3D { pos: tip.to_arr(), normal: [0.0, 1.0, 0.0], uv: [0.0, 0.0], col: [col[0], col[1], col[2], 0.0], mat: [0.0, 1.2] },
            ]);
        }
    }

    fn build_hud(&self, h: &mut HudBatch, fps: f32, id: &ConsoleIdentity) {
        h.clear();
        let snow = [0.95, 0.98, 1.0, 1.0];
        let cyan = [0.18, 0.95, 0.98, 1.0];
        let gold = [1.0, 0.86, 0.28, 1.0];
        let mag = [1.0, 0.30, 0.75, 1.0];
        let muted = [0.70, 0.82, 0.88, 0.95];
        let panel = [0.04, 0.08, 0.13, 0.78];

        match self.phase {
            Phase::Menu => {
                // Sleek AA title overlay floating over the live 3D twilight megacity circuit
                h.skewed_rect(80.0, 74.0, 760.0, 148.0, 26.0, panel);
                h.skewed_rect(80.0, 74.0, 760.0, 6.0, 26.0, cyan);
                h.text(124.0, 100.0, "AETHER DRIFT", 8.0, snow);
                h.text(126.0, 168.0, &format!("ANTI-GRAVITY RACING   |   {}   |   {}", id.gamertag, id.console_label), 2.4, cyan);

                // Track selector cards
                for (i, meta) in TRACKS.iter().enumerate() {
                    let y = 278.0 + i as f32 * 154.0;
                    let sel = i == self.track_idx;
                    let bg = if sel { [0.08, 0.22, 0.30, 0.90] } else { [0.04, 0.08, 0.13, 0.72] };
                    h.skewed_rect(90.0, y, 860.0, 134.0, 20.0, bg);
                    if sel {
                        h.skewed_rect(90.0, y, 12.0, 134.0, 20.0, cyan);
                    }
                    h.text(128.0, y + 20.0, meta.name, 4.5, if sel { snow } else { muted });
                    h.text(128.0, y + 66.0, meta.subtitle, 2.2, if sel { cyan } else { muted });
                    let best_str = format!(
                        "{}   |   BEST RACE {}   |   BEST LAP {}",
                        meta.difficulty,
                        fmt_time(self.records.best_race[i]),
                        fmt_time(self.records.best_lap[i])
                    );
                    h.text(128.0, y + 98.0, &best_str, 2.1, gold);
                }

                // Mode card
                let mode_str = if self.mode == RaceMode::GrandPrix {
                    "MODE: GRAND PRIX (8 RACERS, 3 LAPS)"
                } else {
                    "MODE: TIME TRIAL (SOLO + SAVED GHOST)"
                };
                h.skewed_rect(90.0, 764.0, 860.0, 76.0, 16.0, panel);
                h.skewed_rect(90.0, 764.0, 860.0, 4.0, 16.0, mag);
                h.text(128.0, 788.0, mode_str, 3.0, snow);

                // Controls panel on right
                h.skewed_rect(1120.0, 278.0, 710.0, 460.0, 20.0, panel);
                h.skewed_rect(1120.0, 278.0, 710.0, 5.0, 20.0, cyan);
                h.text(1158.0, 306.0, "DUALSENSE CONTROLS", 3.6, cyan);
                let lines = [
                    "R2 / X            ANALOG THROTTLE",
                    "LEFT STICK        STEER & BANK CRAFT",
                    "L1 / R1           LEFT / RIGHT AIRBRAKE",
                    "SQUARE / TRIANGLE TURBO BOOST",
                    "L2                HARD BRAKE",
                    "OPTIONS           RETURN TO TRACK SELECT",
                    "TOUCHPAD          TRINITYXSX QUICK MENU",
                ];
                for (k, line) in lines.iter().enumerate() {
                    h.text(1158.0, 368.0 + k as f32 * 48.0, line, 2.4, snow);
                }

                // Bottom action prompt
                h.rect(0.0, SCREEN_H as f32 - 72.0, SCREEN_W as f32, 72.0, [0.03, 0.06, 0.10, 0.88]);
                h.text(90.0, SCREEN_H as f32 - 48.0, "D-PAD SELECT TRACK    SQUARE SWITCH MODE    X START RACE    TOUCHPAD MENU", 2.8, cyan);
            }
            Phase::Countdown | Phase::Racing | Phase::Finished => {
                let p = &self.racers[0];
                let kmh = (p.speed * 3.6) as u32;
                let pos = self.player_position();
                let glass_bg = [0.03, 0.08, 0.16, 0.62];
                let border_c = [0.32, 0.88, 1.0, 0.85];

                // Top-left stacked concept-art glassmorphism POS & LAP cards
                h.glass_panel(42.0, 32.0, 210.0, 108.0, 0.0, glass_bg, border_c);
                h.text(64.0, 48.0, if self.mode == RaceMode::GrandPrix { "POS" } else { "MODE" }, 2.4, muted);
                if self.mode == RaceMode::GrandPrix {
                    h.text(64.0, 82.0, &format!("{pos}/{TOTAL_RACERS}"), 5.4, snow);
                } else {
                    h.text(64.0, 84.0, "TRIAL", 4.4, cyan);
                }

                h.glass_panel(42.0, 152.0, 210.0, 108.0, 0.0, glass_bg, border_c);
                h.text(64.0, 168.0, "LAP", 2.4, muted);
                h.text(64.0, 202.0, &format!("{}/{TOTAL_LAPS}", p.lap.min(TOTAL_LAPS)), 5.4, snow);

                // Top-right concept-art slanted TIME card
                h.glass_panel(1600.0, 32.0, 272.0, 108.0, 16.0, glass_bg, border_c);
                h.text(1748.0, 48.0, "TIME:", 2.4, muted);
                h.text(1624.0, 84.0, &fmt_time(self.race_time), 4.4, snow);

                // Bottom-left concept-art glassmorphism Circuit Minimap
                let mx = 42.0;
                let my = 796.0;
                let ms = 242.0;
                h.glass_panel(mx, my, ms, ms, 0.0, [0.03, 0.07, 0.15, 0.66], border_c);
                let pad_m = 24.0;
                let pts = &self.circuit.minimap_pts;
                for i in 0..pts.len() {
                    let a = pts[i];
                    let b = pts[(i + 1) % pts.len()];
                    let ax = mx + pad_m + a[0] * (ms - pad_m * 2.0);
                    let ay = my + pad_m + a[1] * (ms - pad_m * 2.0);
                    let bx = mx + pad_m + b[0] * (ms - pad_m * 2.0);
                    let by = my + pad_m + b[1] * (ms - pad_m * 2.0);
                    h.line(ax, ay, bx, by, 7.0, [0.16, 0.24, 0.34, 0.85]);
                    h.line(ax, ay, bx, by, 2.5, [0.68, 0.84, 0.94, 0.92]);
                }
                for r in self.racers.iter().rev() {
                    let sm = self.circuit.sample_at(r.s);
                    let mp = self.circuit.minimap_coord(sm.pos);
                    let rx = mx + pad_m + mp[0] * (ms - pad_m * 2.0);
                    let ry = my + pad_m + mp[1] * (ms - pad_m * 2.0);
                    if r.is_player {
                        // Bright cyan directional player chevron on minimap
                        h.tri([rx, ry - 11.0], [rx + 8.0, ry + 9.0], [rx - 8.0, ry + 9.0], cyan);
                    } else {
                        let c = [r.glow_col[0], r.glow_col[1], r.glow_col[2], 1.0];
                        h.rect(rx - 4.0, ry - 4.0, 8.0, 8.0, c);
                    }
                }
                h.text(mx + 12.0, my + ms - 22.0, &format!("{fps:.0} FPS"), 1.8, muted);

                // Bottom-right concept-art Speedometer + Segmented Slanted Boost Bar!
                let spd_col = if p.boost_timer > 0.0 { gold } else { cyan };
                h.text(1468.0, 916.0, "SPEED:", 2.5, muted);
                h.text(1588.0, 898.0, &format!("{kmh:03}"), 6.2, snow);
                h.text(1720.0, 916.0, "KM/H", 2.6, muted);

                // 6 slanted glowing segmented boost cells (matching concept art!)
                let cells = 6usize;
                for c_idx in 0..cells {
                    let cx = 1480.0 + c_idx as f32 * 40.0;
                    let cy = 964.0;
                    let frac = (c_idx as f32 + 0.5) / cells as f32;
                    let lit = p.boost_energy >= frac;
                    let col = if lit { spd_col } else { [0.10, 0.18, 0.28, 0.65] };
                    h.skewed_rect(cx, cy, 32.0, 34.0, 12.0, col);
                }
                // Secondary sub-bar below boost cells
                h.glass_panel(1480.0, 1008.0, 240.0, 16.0, 6.0, [0.05, 0.12, 0.20, 0.72], border_c);
                h.skewed_rect(1482.0, 1010.0, 236.0 * p.boost_energy, 12.0, 5.0, spd_col);
                h.glass_panel(1748.0, 956.0, 126.0, 68.0, 14.0, glass_bg, border_c);
                h.text(1772.0, 982.0, if p.boost_timer > 0.0 { "BOOST" } else { "TURBO" }, 2.4, spd_col);

                // Countdown / Lap banner
                if self.phase == Phase::Countdown {
                    let n = self.countdown.ceil().max(1.0) as i32;
                    let txt = format!("{n}");
                    let w = HudBatch::text_w(&txt, 18.0);
                    h.text((SCREEN_W as f32 - w) * 0.5, 360.0, &txt, 18.0, gold);
                } else if self.lap_banner > 0.0 {
                    let w = HudBatch::text_w(&self.lap_banner_txt, 9.0);
                    h.text((SCREEN_W as f32 - w) * 0.5, 240.0, &self.lap_banner_txt, 9.0, cyan);
                }

                // Finish Podium Overlay
                if self.phase == Phase::Finished {
                    h.skewed_rect(490.0, 260.0, 940.0, 490.0, 24.0, [0.04, 0.08, 0.13, 0.92]);
                    h.skewed_rect(490.0, 260.0, 940.0, 7.0, 24.0, gold);
                    let title = if self.mode == RaceMode::GrandPrix {
                        format!("RACE COMPLETE  -  POSITION {pos}/{TOTAL_RACERS}")
                    } else {
                        "TIME TRIAL COMPLETE!".into()
                    };
                    h.text((SCREEN_W as f32 - HudBatch::text_w(&title, 4.5)) * 0.5, 308.0, &title, 4.5, gold);

                    let medal = if self.race_time <= self.circuit.meta.gold_secs {
                        "MEDAL: GOLD CLASS!"
                    } else if self.race_time <= self.circuit.meta.gold_secs + 12.0 {
                        "MEDAL: SILVER CLASS"
                    } else {
                        "MEDAL: BRONZE CLASS"
                    };
                    h.text((SCREEN_W as f32 - HudBatch::text_w(medal, 3.8)) * 0.5, 386.0, medal, 3.8, cyan);

                    let t1 = format!("TOTAL TIME:  {}", fmt_time(self.race_time));
                    let t2 = format!("BEST LAP:    {}", fmt_time(self.best_lap_this_race));
                    h.text(650.0, 464.0, &t1, 3.4, snow);
                    h.text(650.0, 518.0, &t2, 3.4, snow);
                    if self.new_record {
                        let rec = "NEW PERSONAL RECORD SAVED TO PLS!";
                        h.text((SCREEN_W as f32 - HudBatch::text_w(rec, 3.0)) * 0.5, 590.0, rec, 3.0, gold);
                    }
                    let act = "X RACE AGAIN    O TRACK SELECT    TOUCHPAD MENU";
                    h.text((SCREEN_W as f32 - HudBatch::text_w(act, 2.8)) * 0.5, 678.0, act, 2.8, cyan);
                }
            }
        }
    }
}

/// Blazing volumetric multi-layered dual plasma thruster plumes (+ real wet-track mirrored plumes!).
#[allow(clippy::too_many_arguments)]
fn emit_thruster_plumes(
    fx_out: &mut Vec<Vertex3D>,
    refl_out: Option<&mut Vec<Vertex3D>>,
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
    plane_origin: Vec3,
    plane_up: Vec3,
    is_hero: bool,
    glow_col: [f32; 4],
    boosting: bool,
    throttle: f32,
) {
    let pt = |x: f32, y: f32, z: f32| pos.add(right.mul(x)).add(up.mul(y)).add(fwd.mul(z));
    let reflect_pt = |p: Vec3| p.sub(plane_up.mul(2.0 * p.sub(plane_origin).dot(plane_up)));

    let noz_x = if is_hero { 1.55f32 } else { 1.20f32 };
    let noz_z = if is_hero { -3.55f32 } else { -3.50f32 };
    let outer_len = (2.6 + throttle * 2.1) * if boosting { 1.38 } else { 1.0 };
    let inner_len = outer_len * 0.55;

    let outer_col = if boosting {
        [0.35, 0.95, 1.0, 0.62]
    } else {
        [glow_col[0], glow_col[1], glow_col[2], 0.55]
    };
    let core_col = [0.92, 0.99, 1.0, 0.88];

    let mut refl_tmp: Vec<Vertex3D> = Vec::new();
    let emit_cone = |out: &mut Vec<Vertex3D>, refl: &mut Vec<Vertex3D>, sx: f32, rad: f32, len: f32, col: [f32; 4], emiss: f32| {
        let segs = 8usize;
        let tip = pt(sx, 0.12, noz_z - len);
        let tip_c = [col[0], col[1], col[2], 0.0];
        for k in 0..segs {
            let a0 = (k as f32 / segs as f32) * std::f32::consts::TAU;
            let a1 = ((k + 1) as f32 / segs as f32) * std::f32::consts::TAU;
            let p0 = pt(sx + a0.cos() * rad, 0.12 + a0.sin() * rad, noz_z);
            let p1 = pt(sx + a1.cos() * rad, 0.12 + a1.sin() * rad, noz_z);
            let n = [0.0, 1.0, 0.0];
            out.extend_from_slice(&[
                Vertex3D { pos: p0.to_arr(), normal: n, uv: [0.0, 0.0], col, mat: [0.0, emiss] },
                Vertex3D { pos: p1.to_arr(), normal: n, uv: [0.0, 0.0], col, mat: [0.0, emiss] },
                Vertex3D { pos: tip.to_arr(), normal: n, uv: [0.0, 0.0], col: tip_c, mat: [0.0, emiss] },
            ]);
            let rc = [col[0], col[1], col[2], col[3] * 0.75];
            refl.extend_from_slice(&[
                Vertex3D { pos: reflect_pt(p0).to_arr(), normal: n, uv: [0.0, 0.0], col: rc, mat: [0.0, emiss * 0.8] },
                Vertex3D { pos: reflect_pt(tip).to_arr(), normal: n, uv: [0.0, 0.0], col: tip_c, mat: [0.0, emiss * 0.8] },
                Vertex3D { pos: reflect_pt(p1).to_arr(), normal: n, uv: [0.0, 0.0], col: rc, mat: [0.0, emiss * 0.8] },
            ]);
        }
    };

    for sx in [-noz_x, noz_x] {
        emit_cone(fx_out, &mut refl_tmp, sx, 0.44, outer_len, outer_col, 2.2);
        emit_cone(fx_out, &mut refl_tmp, sx, 0.24, inner_len, core_col, 3.2);
    }
    if let Some(ro) = refl_out {
        ro.extend_from_slice(&refl_tmp);
    }
}

/// Glowing twin sleek 3D ribbon thruster trails behind each racing craft's left & right pods.
fn emit_trail(fx_out: &mut Vec<Vertex3D>, trail: &VecDeque<(Vec3, Vec3)>, glow_col: [f32; 4], boosting: bool) {
    if trail.len() < 2 {
        return;
    }
    let len = trail.len() as f32;
    let base_w = if boosting { 0.46 } else { 0.32 };
    for side in [-1.35f32, 1.35f32] {
        for i in 0..trail.len() - 1 {
            let t0 = 1.0 - (i as f32 / len);
            let t1 = 1.0 - ((i + 1) as f32 / len);
            let (p0, r0) = trail[i];
            let (p1, r1) = trail[i + 1];
            let o0 = p0.add(r0.mul(side));
            let o1 = p1.add(r1.mul(side));
            let w0 = r0.mul(base_w * t0);
            let w1 = r1.mul(base_w * t1);
            let c0 = [glow_col[0], glow_col[1], glow_col[2], t0 * t0 * 0.38];
            let c1 = [glow_col[0], glow_col[1], glow_col[2], t1 * t1 * 0.38];
            let n = [0.0, 1.0, 0.0];
            let va = Vertex3D { pos: o0.sub(w0).to_arr(), normal: n, uv: [0.0, 0.0], col: c0, mat: [0.0, 1.4] };
            let vb = Vertex3D { pos: o0.add(w0).to_arr(), normal: n, uv: [0.0, 0.0], col: c0, mat: [0.0, 1.4] };
            let vc = Vertex3D { pos: o1.add(w1).to_arr(), normal: n, uv: [0.0, 0.0], col: c1, mat: [0.0, 1.4] };
            let vd = Vertex3D { pos: o1.sub(w1).to_arr(), normal: n, uv: [0.0, 0.0], col: c1, mat: [0.0, 1.4] };
            fx_out.extend_from_slice(&[va, vb, vc, va, vc, vd]);
        }
    }
}

fn main() {
    let identity = init_console_runtime();
    println!(
        "AETHER DRIFT v2.0 (Blender PBR + Reflections + Bloom): gamertag={} console={} pls={:?}",
        identity.gamertag, identity.console_label, identity.pls_dir
    );

    let mut renderer = match Renderer::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("AETHER DRIFT: D3D12.X init failed: {e}");
            std::process::exit(1);
        }
    };
    let mut pad = Pad::init();
    let mut audio = RaceAudio::init();
    let mut app = match App::new(&mut renderer, &identity) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("AETHER DRIFT: track init failed: {e}");
            std::process::exit(1);
        }
    };

    let mut craft3d = Vec::with_capacity(32_768);
    let mut dyn_refl3d = Vec::with_capacity(32_768);
    let mut fx3d = Vec::with_capacity(24_576);
    let mut hud = HudBatch::new();

    let mut last = Instant::now();
    let mut fps_t = Instant::now();
    let mut fps_n = 0u32;
    let mut fps = 60.0f32;

    loop {
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().clamp(0.001, 0.05);
        last = now;

        pad.poll(dt);
        app.update(dt, &mut pad, &mut audio, &mut renderer, &identity);
        app.build_dynamic_scene(&mut craft3d, &mut dyn_refl3d, &mut fx3d);
        app.build_hud(&mut hud, fps, &identity);

        let view = Mat4::look_at_lh(app.cam_pos, app.cam_target, app.cam_up);
        let proj = Mat4::perspective_lh(app.cam_fov, SCREEN_W as f32 / SCREEN_H as f32, 0.5, 2800.0);
        let vp = proj.mul(view);
        let boost_glow = if app.racers.first().is_some_and(|r| r.boost_timer > 0.0) { 1.0 } else { 0.0 };

        if let Err(e) = renderer.present_frame(
            vp,
            app.cam_pos.to_arr(),
            app.time,
            app.circuit.meta.fog_rgb,
            boost_glow,
            &craft3d,
            &dyn_refl3d,
            &fx3d,
            &hud.verts,
        ) {
            eprintln!("AETHER DRIFT: PresentX error: {e}");
            break;
        }

        fps_n += 1;
        let el = fps_t.elapsed().as_secs_f32();
        if el >= 0.5 {
            fps = fps_n as f32 / el;
            fps_n = 0;
            fps_t = Instant::now();
        }
    }

    pad.update_rumble(0.0, 0.0);
}
