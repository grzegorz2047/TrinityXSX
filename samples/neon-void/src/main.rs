// SPDX-License-Identifier: MIT
//! NEON VOID XSX — Open-source Xbox Series X|S (Scarlett) 60 FPS twin-stick neon arcade
//! shooter & D3D12.X / DualSense / XAudio2 / Persistent Local Storage benchmark for TrinityXSX.

mod audio;
mod draw;
mod scarlett;

use std::f32::consts::TAU;
use std::fs;
use std::path::Path;
use std::time::Instant;

use audio::{Sfx, SynthAudio};
use draw::Batch;
use scarlett::{
    init_console_runtime, ConsoleIdentity, Pad, Renderer, BTN_CROSS, BTN_DOWN, BTN_LEFT, BTN_R1,
    BTN_RIGHT, BTN_SQUARE, BTN_UP, SCREEN_H, SCREEN_W,
};

const ARENA_L: f32 = 88.0;
const ARENA_R: f32 = SCREEN_W as f32 - 88.0;
const ARENA_T: f32 = 108.0;
const ARENA_B: f32 = SCREEN_H as f32 - 74.0;

struct Rng(u64);
impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
    fn f01(&mut self) -> f32 {
        (self.next_u32() as f32) / (u32::MAX as f32)
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f01()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EnemyKind {
    Drone,    // Magenta diamond
    Spinner,  // Orange fast hexagon
    Splitter, // Yellow triangle (splits into 2 shards)
    Shard,    // Small fast yellow dart
    Goliath,  // Heavy crimson octagon boss
}

struct Enemy {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    angle: f32,
    hp: i32,
    radius: f32,
    kind: EnemyKind,
    flash: f32,
}

struct Bullet {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max_life: f32,
    size: f32,
    col: [f32; 4],
}

struct Star {
    x: f32,
    y: f32,
    depth: f32,
}

struct SaveData {
    best_score: u64,
    best_wave: u32,
    total_kills: u64,
}

impl SaveData {
    fn load(pls_dir: Option<&Path>) -> Self {
        let mut s = Self { best_score: 0, best_wave: 1, total_kills: 0 };
        let Some(dir) = pls_dir else { return s };
        let Ok(text) = fs::read_to_string(dir.join("highscore.txt")) else { return s };
        for line in text.lines() {
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "best_score" => s.best_score = v.trim().parse().unwrap_or(0),
                    "best_wave" => s.best_wave = v.trim().parse().unwrap_or(1),
                    "total_kills" => s.total_kills = v.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        s
    }

    fn save(&self, pls_dir: Option<&Path>, gamertag: &str) {
        let Some(dir) = pls_dir else { return };
        let _ = fs::create_dir_all(dir);
        let body = format!(
            "gamertag={gamertag}\nbest_score={}\nbest_wave={}\ntotal_kills={}\n",
            self.best_score, self.best_wave, self.total_kills
        );
        let _ = fs::write(dir.join("highscore.txt"), body);
    }
}

struct Game {
    rng: Rng,
    px: f32,
    py: f32,
    pvx: f32,
    pvy: f32,
    aim_angle: f32,
    hp: i32,
    invuln: f32,
    fire_cd: f32,
    emp_charge: f32,
    emp_ring: f32,
    emp_x: f32,
    emp_y: f32,
    score: u64,
    combo: u32,
    combo_timer: f32,
    wave: u32,
    wave_kills_left: u32,
    spawn_cd: f32,
    wave_banner: f32,
    screen_shake: f32,
    game_over: bool,
    time: f32,
    stress_level: usize, // 0=OFF, 1=+256MB, 2=+512MB, 3=+1024MB
    stress_vram_mb: usize,
    enemies: Vec<Enemy>,
    bullets: Vec<Bullet>,
    particles: Vec<Particle>,
    stars: Vec<Star>,
    save: SaveData,
}

impl Game {
    fn new(id: &ConsoleIdentity) -> Self {
        let mut rng = Rng(0x5452_5801_2026_1125);
        let mut stars = Vec::with_capacity(140);
        for _ in 0..140 {
            stars.push(Star {
                x: rng.range(ARENA_L, ARENA_R),
                y: rng.range(ARENA_T, ARENA_B),
                depth: rng.range(0.25, 1.0),
            });
        }
        let save = SaveData::load(id.pls_dir.as_deref());
        let g = Self {
            rng,
            px: (ARENA_L + ARENA_R) * 0.5,
            py: (ARENA_T + ARENA_B) * 0.5,
            pvx: 0.0,
            pvy: 0.0,
            aim_angle: -std::f32::consts::FRAC_PI_2,
            hp: 5,
            invuln: 1.5,
            fire_cd: 0.0,
            emp_charge: 1.0,
            emp_ring: 0.0,
            emp_x: 0.0,
            emp_y: 0.0,
            score: 0,
            combo: 1,
            combo_timer: 0.0,
            wave: 1,
            wave_kills_left: 12,
            spawn_cd: 0.4,
            wave_banner: 2.2,
            screen_shake: 0.0,
            game_over: false,
            time: 0.0,
            stress_level: 0,
            stress_vram_mb: 0,
            enemies: Vec::with_capacity(128),
            bullets: Vec::with_capacity(256),
            particles: Vec::with_capacity(2400),
            stars,
            save,
        };
        g.save.save(id.pls_dir.as_deref(), &id.gamertag);
        g
    }

    fn reset(&mut self) {
        self.px = (ARENA_L + ARENA_R) * 0.5;
        self.py = (ARENA_T + ARENA_B) * 0.5;
        self.pvx = 0.0;
        self.pvy = 0.0;
        self.hp = 5;
        self.invuln = 1.5;
        self.fire_cd = 0.0;
        self.emp_charge = 1.0;
        self.emp_ring = 0.0;
        self.score = 0;
        self.combo = 1;
        self.combo_timer = 0.0;
        self.wave = 1;
        self.wave_kills_left = 12;
        self.spawn_cd = 0.4;
        self.wave_banner = 2.0;
        self.screen_shake = 0.0;
        self.game_over = false;
        self.enemies.clear();
        self.bullets.clear();
    }

    fn spawn_burst(&mut self, x: f32, y: f32, count: usize, col: [f32; 4], speed_scale: f32) {
        let cap = if self.stress_level > 0 { 2200 } else { 700 };
        for _ in 0..count {
            if self.particles.len() >= cap {
                break;
            }
            let ang = self.rng.range(0.0, TAU);
            let spd = self.rng.range(80.0, 420.0) * speed_scale;
            let life = self.rng.range(0.25, 0.75);
            self.particles.push(Particle {
                x,
                y,
                vx: ang.cos() * spd,
                vy: ang.sin() * spd,
                life,
                max_life: life,
                size: self.rng.range(3.0, 8.0),
                col,
            });
        }
    }

    fn spawn_enemy(&mut self) {
        // Spawn along arena edge away from player
        let edge = self.rng.next_u32() % 4;
        let (x, y) = match edge {
            0 => (self.rng.range(ARENA_L + 30.0, ARENA_R - 30.0), ARENA_T + 26.0),
            1 => (self.rng.range(ARENA_L + 30.0, ARENA_R - 30.0), ARENA_B - 26.0),
            2 => (ARENA_L + 26.0, self.rng.range(ARENA_T + 30.0, ARENA_B - 30.0)),
            _ => (ARENA_R - 26.0, self.rng.range(ARENA_T + 30.0, ARENA_B - 30.0)),
        };
        let roll = self.rng.f01();
        let (kind, hp, radius) = if self.wave >= 3 && roll < 0.14 {
            (EnemyKind::Goliath, 6 + (self.wave as i32 / 2), 34.0)
        } else if self.wave >= 2 && roll < 0.38 {
            (EnemyKind::Splitter, 2, 22.0)
        } else if roll < 0.68 {
            (EnemyKind::Spinner, 1, 19.0)
        } else {
            (EnemyKind::Drone, 1, 20.0)
        };
        self.enemies.push(Enemy { x, y, vx: 0.0, vy: 0.0, angle: 0.0, hp, radius, kind, flash: 0.0 });
    }

    fn update(&mut self, dt: f32, pad: &mut Pad, audio: &mut Option<SynthAudio>, renderer: &mut Renderer, id: &ConsoleIdentity) {
        self.time += dt;
        self.screen_shake = (self.screen_shake - dt * 28.0).max(0.0);
        self.wave_banner = (self.wave_banner - dt).max(0.0);

        // R1 cycles real D3D12.X VRAM + GPU particle stress mode
        if pad.pressed(BTN_R1) {
            self.stress_level = (self.stress_level + 1) % 4;
            let chunks = match self.stress_level {
                1 => 1,
                2 => 2,
                3 => 4,
                _ => 0,
            };
            self.stress_vram_mb = renderer.set_vram_stress_chunks(chunks);
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Wave);
            }
            pad.trigger_rumble(0.3, 0.5, 0.12);
        }

        // Ambient particles in stress mode to exercise vertex upload + PS5 RADV blending
        if self.stress_level > 0 {
            let extra = self.stress_level * 12;
            for _ in 0..extra {
                let x = self.rng.range(ARENA_L + 20.0, ARENA_R - 20.0);
                let y = self.rng.range(ARENA_T + 20.0, ARENA_B - 20.0);
                let c = if self.rng.next_u32() & 1 == 0 {
                    [0.35, 0.95, 0.85, 0.75]
                } else {
                    [0.98, 0.30, 0.78, 0.75]
                };
                self.spawn_burst(x, y, 1, c, 0.65);
            }
        }

        // Update particles even on game over so explosions finish smoothly
        self.particles.retain_mut(|p| {
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.vx *= 1.0 - (dt * 2.2).min(0.9);
            p.vy *= 1.0 - (dt * 2.2).min(0.9);
            p.life -= dt;
            p.life > 0.0
        });

        if self.game_over {
            if pad.pressed(BTN_CROSS) {
                self.reset();
                if let Some(a) = audio.as_mut() {
                    a.play(Sfx::Wave);
                }
            }
            return;
        }

        // Movement: Left stick + D-pad
        let mut mx = pad.lx;
        let mut my = -pad.ly;
        if pad.held(BTN_LEFT) { mx -= 1.0; }
        if pad.held(BTN_RIGHT) { mx += 1.0; }
        if pad.held(BTN_UP) { my -= 1.0; }
        if pad.held(BTN_DOWN) { my += 1.0; }
        let mlen = (mx * mx + my * my).sqrt();
        if mlen > 1.0 {
            mx /= mlen;
            my /= mlen;
        }
        let accel = 2600.0;
        self.pvx += mx * accel * dt;
        self.pvy += my * accel * dt;
        let drag = (-5.8 * dt).exp();
        self.pvx *= drag;
        self.pvy *= drag;
        self.px = (self.px + self.pvx * dt).clamp(ARENA_L + 22.0, ARENA_R - 22.0);
        self.py = (self.py + self.pvy * dt).clamp(ARENA_T + 22.0, ARENA_B - 22.0);

        // Engine thruster trail
        if mlen > 0.25 {
            self.aim_angle = if pad.rx.abs() < 0.01 && pad.ry.abs() < 0.01 {
                my.atan2(mx)
            } else {
                self.aim_angle
            };
            self.spawn_burst(
                self.px - mx * 16.0,
                self.py - my * 16.0,
                1,
                [0.40, 0.92, 0.95, 0.75],
                0.35,
            );
        }

        // Aim & Fire: Right Stick twin-stick OR Cross / R2 auto-aim
        let mut firing = false;
        if pad.rx.abs() > 0.01 || pad.ry.abs() > 0.01 {
            self.aim_angle = (-pad.ry).atan2(pad.rx);
            firing = true;
        } else if pad.held(BTN_CROSS) || pad.r2 > 0.25 {
            firing = true;
            if let Some(nearest) = self.enemies.iter().min_by(|a, b| {
                let da = (a.x - self.px).powi(2) + (a.y - self.py).powi(2);
                let db = (b.x - self.px).powi(2) + (b.y - self.py).powi(2);
                da.total_cmp(&db)
            }) {
                self.aim_angle = (nearest.y - self.py).atan2(nearest.x - self.px);
            }
        }

        self.fire_cd = (self.fire_cd - dt).max(0.0);
        if firing && self.fire_cd <= 0.0 {
            self.fire_cd = 0.095;
            let spd = 1120.0;
            let ca = self.aim_angle.cos();
            let sa = self.aim_angle.sin();
            // Twin parallel plasma bolts
            for side in [-7.0f32, 7.0f32] {
                let ox = -sa * side + ca * 20.0;
                let oy = ca * side + sa * 20.0;
                self.bullets.push(Bullet {
                    x: self.px + ox,
                    y: self.py + oy,
                    vx: ca * spd,
                    vy: sa * spd,
                    life: 1.4,
                });
            }
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Laser);
            }
            pad.trigger_rumble(0.0, 0.16, 0.03);
        }

        // EMP Shockwave Nova (Square or L2 when charged)
        self.emp_charge = (self.emp_charge + dt * 0.125).min(1.0);
        if self.emp_charge >= 1.0 && (pad.pressed(BTN_SQUARE) || pad.l2 > 0.65) {
            self.emp_charge = 0.0;
            self.emp_ring = 18.0;
            self.emp_x = self.px;
            self.emp_y = self.py;
            self.screen_shake = 16.0;
            self.spawn_burst(self.px, self.py, 48, [0.52, 0.96, 0.80, 0.95], 1.7);
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Emp);
            }
            pad.trigger_rumble(0.95, 0.85, 0.35);
        }
        if self.emp_ring > 0.0 {
            self.emp_ring += dt * 1450.0;
            if self.emp_ring > 760.0 {
                self.emp_ring = 0.0;
            }
        }

        // Combo decay & invulnerability
        self.invuln = (self.invuln - dt).max(0.0);
        if self.combo_timer > 0.0 {
            self.combo_timer -= dt;
            if self.combo_timer <= 0.0 {
                self.combo = 1;
            }
        }

        // Move bullets
        self.bullets.retain_mut(|b| {
            b.x += b.vx * dt;
            b.y += b.vy * dt;
            b.life -= dt;
            b.life > 0.0 && b.x > ARENA_L && b.x < ARENA_R && b.y > ARENA_T && b.y < ARENA_B
        });

        // Spawn enemies
        self.spawn_cd -= dt;
        let max_alive = (10 + self.wave * 3).min(48) as usize;
        if self.spawn_cd <= 0.0 && self.wave_kills_left > 0 && self.enemies.len() < max_alive {
            self.spawn_enemy();
            self.spawn_cd = (0.85 - self.wave as f32 * 0.06).max(0.20);
        }

        // Move enemies & resolve collisions
        let mut spawned_shards: Vec<Enemy> = Vec::new();
        let mut kills_this_frame = 0u32;
        let mut player_hit = false;

        for e in &mut self.enemies {
            e.flash = (e.flash - dt * 5.0).max(0.0);
            let dx = self.px - e.x;
            let dy = self.py - e.y;
            let dist = (dx * dx + dy * dy).sqrt().max(0.001);
            let speed = match e.kind {
                EnemyKind::Drone => 185.0 + self.wave as f32 * 12.0,
                EnemyKind::Spinner => 245.0 + self.wave as f32 * 14.0,
                EnemyKind::Splitter => 165.0 + self.wave as f32 * 10.0,
                EnemyKind::Shard => 310.0 + self.wave as f32 * 12.0,
                EnemyKind::Goliath => 115.0 + self.wave as f32 * 8.0,
            };
            let swirl = if e.kind == EnemyKind::Spinner {
                (self.time * 4.5 + e.angle).sin() * 0.65
            } else {
                0.0
            };
            let dir_x = dx / dist;
            let dir_y = dy / dist;
            let tx = dir_x * speed - dir_y * speed * swirl;
            let ty = dir_y * speed + dir_x * speed * swirl;
            e.vx += (tx - e.vx) * (dt * 4.0).min(1.0);
            e.vy += (ty - e.vy) * (dt * 4.0).min(1.0);
            e.x = (e.x + e.vx * dt).clamp(ARENA_L + e.radius, ARENA_R - e.radius);
            e.y = (e.y + e.vy * dt).clamp(ARENA_T + e.radius, ARENA_B - e.radius);
            e.angle += dt * match e.kind {
                EnemyKind::Goliath => 1.1,
                EnemyKind::Spinner => 5.0,
                _ => 2.6,
            };

            // EMP shockwave hit
            if self.emp_ring > 0.0 {
                let ed = ((e.x - self.emp_x).powi(2) + (e.y - self.emp_y).powi(2)).sqrt();
                if ed <= self.emp_ring + e.radius {
                    e.hp = 0;
                }
            }

            // Bullet hits
            if e.hp > 0 {
                for b in &mut self.bullets {
                    if b.life <= 0.0 {
                        continue;
                    }
                    let bd2 = (b.x - e.x).powi(2) + (b.y - e.y).powi(2);
                    let r = e.radius + 9.0;
                    if bd2 <= r * r {
                        b.life = 0.0;
                        e.hp -= 1;
                        e.flash = 1.0;
                        if e.hp <= 0 {
                            break;
                        }
                    }
                }
            }

            // Player collision
            if e.hp > 0 && self.invuln <= 0.0 && dist < e.radius + 18.0 {
                player_hit = true;
                e.hp = 0;
            }
        }

        let mut i = 0;
        while i < self.enemies.len() {
            if self.enemies[i].hp <= 0 {
                let e = self.enemies.swap_remove(i);
                kills_this_frame += 1;
                let (pts, col) = match e.kind {
                    EnemyKind::Drone => (100, [1.0, 0.28, 0.78, 1.0]),
                    EnemyKind::Spinner => (150, [1.0, 0.56, 0.16, 1.0]),
                    EnemyKind::Splitter => (200, [1.0, 0.90, 0.25, 1.0]),
                    EnemyKind::Shard => (75, [1.0, 0.96, 0.45, 1.0]),
                    EnemyKind::Goliath => (600, [1.0, 0.22, 0.30, 1.0]),
                };
                self.score += pts * self.combo as u64;
                self.combo = (self.combo + 1).min(25);
                self.combo_timer = 3.2;
                self.emp_charge = (self.emp_charge + 0.045).min(1.0);
                self.save.total_kills += 1;
                self.spawn_burst(e.x, e.y, if e.kind == EnemyKind::Goliath { 32 } else { 14 }, col, 1.0);

                if e.kind == EnemyKind::Splitter {
                    for s in [-1.0f32, 1.0f32] {
                        spawned_shards.push(Enemy {
                            x: (e.x + s * 16.0).clamp(ARENA_L + 16.0, ARENA_R - 16.0),
                            y: (e.y - s * 12.0).clamp(ARENA_T + 16.0, ARENA_B - 16.0),
                            vx: s * 220.0,
                            vy: -s * 140.0,
                            angle: 0.0,
                            hp: 1,
                            radius: 13.0,
                            kind: EnemyKind::Shard,
                            flash: 0.5,
                        });
                    }
                }
            } else {
                i += 1;
            }
        }
        self.enemies.extend(spawned_shards);

        if kills_this_frame > 0 {
            self.wave_kills_left = self.wave_kills_left.saturating_sub(kills_this_frame);
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Pop);
            }
            pad.trigger_rumble(0.25, 0.35, 0.07);
            if self.score > self.save.best_score {
                self.save.best_score = self.score;
            }
        }

        if player_hit {
            self.hp -= 1;
            self.invuln = 1.6;
            self.combo = 1;
            self.screen_shake = 14.0;
            self.spawn_burst(self.px, self.py, 26, [1.0, 0.25, 0.35, 1.0], 1.2);
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Hit);
            }
            pad.trigger_rumble(0.95, 0.65, 0.28);
            if self.hp <= 0 {
                self.game_over = true;
                self.save.best_score = self.save.best_score.max(self.score);
                self.save.best_wave = self.save.best_wave.max(self.wave);
                self.save.save(id.pls_dir.as_deref(), &id.gamertag);
            }
        }

        // Advance wave when all enemies cleared
        if self.wave_kills_left == 0 && self.enemies.is_empty() {
            self.wave += 1;
            self.wave_kills_left = 10 + self.wave * 5;
            self.wave_banner = 2.0;
            self.hp = (self.hp + 1).min(5);
            self.save.best_score = self.save.best_score.max(self.score);
            self.save.best_wave = self.save.best_wave.max(self.wave);
            self.save.save(id.pls_dir.as_deref(), &id.gamertag);
            if let Some(a) = audio.as_mut() {
                a.play(Sfx::Wave);
            }
            pad.trigger_rumble(0.4, 0.6, 0.2);
        }
    }

    fn render(&self, b: &mut Batch, fps: f32, id: &ConsoleIdentity) {
        b.clear();
        let sx = if self.screen_shake > 0.1 { (self.time * 91.0).sin() * self.screen_shake } else { 0.0 };
        let sy = if self.screen_shake > 0.1 { (self.time * 73.0).cos() * self.screen_shake } else { 0.0 };

        // Subtle animated synthwave grid
        let grid_col = [0.12, 0.28, 0.34, 0.35];
        let step = 80.0;
        let mut gx = ARENA_L + step;
        while gx < ARENA_R {
            b.line(gx + sx * 0.3, ARENA_T + sy * 0.3, gx + sx * 0.3, ARENA_B + sy * 0.3, 1.5, grid_col);
            gx += step;
        }
        let mut gy = ARENA_T + step;
        while gy < ARENA_B {
            b.line(ARENA_L + sx * 0.3, gy + sy * 0.3, ARENA_R + sx * 0.3, gy + sy * 0.3, 1.5, grid_col);
            gy += step;
        }

        // Parallax starfield
        for st in &self.stars {
            let x = st.x + (self.px - 960.0) * -0.04 * st.depth + sx * 0.2;
            let y = st.y + (self.py - 540.0) * -0.04 * st.depth + sy * 0.2;
            let sz = 1.8 + st.depth * 2.4;
            let a = 0.25 + 0.55 * st.depth;
            b.rect(x, y, sz, sz, [0.72, 0.90, 0.98, a]);
        }

        // Glowing arena frame
        let border_col = if self.emp_ring > 0.0 {
            [0.52, 0.98, 0.82, 0.95]
        } else if self.invuln > 1.2 {
            [1.0, 0.25, 0.38, 0.95]
        } else {
            [0.32, 0.85, 0.78, 0.85]
        };
        b.neon_line(ARENA_L + sx, ARENA_T + sy, ARENA_R + sx, ARENA_T + sy, 3.0, border_col);
        b.neon_line(ARENA_R + sx, ARENA_T + sy, ARENA_R + sx, ARENA_B + sy, 3.0, border_col);
        b.neon_line(ARENA_R + sx, ARENA_B + sy, ARENA_L + sx, ARENA_B + sy, 3.0, border_col);
        b.neon_line(ARENA_L + sx, ARENA_B + sy, ARENA_L + sx, ARENA_T + sy, 3.0, border_col);

        // Expanding EMP Shockwave Ring
        if self.emp_ring > 0.0 {
            let alpha = (1.0 - self.emp_ring / 760.0).clamp(0.0, 1.0);
            b.ring(self.emp_x + sx, self.emp_y + sy, self.emp_ring, 7.0, 48, [0.52, 0.98, 0.82, alpha]);
            b.ring(self.emp_x + sx, self.emp_y + sy, self.emp_ring * 0.86, 3.5, 40, [0.95, 1.0, 1.0, alpha * 0.8]);
        }

        // Particles
        for p in &self.particles {
            let a = (p.life / p.max_life).clamp(0.0, 1.0) * p.col[3];
            let sz = p.size * (0.4 + 0.6 * (p.life / p.max_life));
            b.rect(p.x - sz * 0.5 + sx, p.y - sz * 0.5 + sy, sz, sz, [p.col[0], p.col[1], p.col[2], a]);
        }

        // Bullets
        for blt in &self.bullets {
            let ang = blt.vy.atan2(blt.vx);
            let tx = blt.x - ang.cos() * 20.0;
            let ty = blt.y - ang.sin() * 20.0;
            b.neon_line(tx + sx, ty + sy, blt.x + sx, blt.y + sy, 4.0, [0.65, 1.0, 0.92, 1.0]);
        }

        // Enemies
        for e in &self.enemies {
            let base_col = match e.kind {
                EnemyKind::Drone => [1.0, 0.25, 0.76, 1.0],
                EnemyKind::Spinner => [1.0, 0.56, 0.16, 1.0],
                EnemyKind::Splitter => [1.0, 0.90, 0.22, 1.0],
                EnemyKind::Shard => [1.0, 0.96, 0.45, 1.0],
                EnemyKind::Goliath => [1.0, 0.20, 0.30, 1.0],
            };
            let col = if e.flash > 0.1 { [1.0, 1.0, 1.0, 1.0] } else { base_col };
            let sides = match e.kind {
                EnemyKind::Drone => 4,
                EnemyKind::Spinner => 6,
                EnemyKind::Splitter | EnemyKind::Shard => 3,
                EnemyKind::Goliath => 8,
            };
            b.polygon(e.x + sx, e.y + sy, e.radius, sides, e.angle, if e.kind == EnemyKind::Goliath { 5.0 } else { 3.5 }, col);
            if e.kind == EnemyKind::Goliath {
                let core_r = 9.0 + (self.time * 8.0).sin() * 3.0;
                b.disc(e.x + sx, e.y + sy, core_r, 12, [1.0, 0.5, 0.55, 0.85]);
            }
        }

        // Player ship
        let blink = self.invuln > 0.0 && ((self.time * 18.0) as i32 & 1) == 0;
        if !self.game_over && !blink {
            let ca = self.aim_angle.cos();
            let sa = self.aim_angle.sin();
            let rot = |lx: f32, ly: f32| -> [f32; 2] {
                [self.px + lx * ca - ly * sa + sx, self.py + lx * sa + ly * ca + sy]
            };
            let nose = rot(26.0, 0.0);
            let wing_l = rot(-18.0, -18.0);
            let tail = rot(-9.0, 0.0);
            let wing_r = rot(-18.0, 18.0);
            let ship_col = [0.52, 0.96, 0.84, 1.0];
            b.neon_line(nose[0], nose[1], wing_l[0], wing_l[1], 4.0, ship_col);
            b.neon_line(wing_l[0], wing_l[1], tail[0], tail[1], 4.0, ship_col);
            b.neon_line(tail[0], tail[1], wing_r[0], wing_r[1], 4.0, ship_col);
            b.neon_line(wing_r[0], wing_r[1], nose[0], nose[1], 4.0, ship_col);
            b.disc(self.px + sx, self.py + sy, 4.5, 10, [1.0, 1.0, 1.0, 0.95]);
        }

        // Top HUD bar
        let snow = [0.94, 0.97, 0.98, 1.0];
        let mint = [0.52, 0.90, 0.75, 1.0];
        let muted = [0.68, 0.78, 0.82, 0.95];
        let gold = [1.0, 0.88, 0.35, 1.0];

        b.text(88.0, 24.0, "NEON VOID XSX", 4.0, mint);
        let profile_line = format!("{}  |  {}", id.gamertag, id.console_label);
        b.text(88.0, 64.0, &profile_line, 2.5, muted);

        let score_txt = format!("SCORE {:07}   BEST {:07}   WAVE {}   X{}", self.score, self.save.best_score, self.wave, self.combo);
        b.text(560.0, 32.0, &score_txt, 3.5, snow);

        // Shield pips
        for h in 0..5 {
            let hx = 1490.0 + h as f32 * 34.0;
            let col = if h < self.hp { [0.52, 0.96, 0.80, 1.0] } else { [0.22, 0.30, 0.34, 0.8] };
            b.rect(hx, 26.0, 24.0, 20.0, col);
        }
        // EMP charge bar
        b.rect(1490.0, 58.0, 160.0, 14.0, [0.18, 0.25, 0.30, 0.9]);
        let emp_col = if self.emp_charge >= 1.0 { gold } else { mint };
        b.rect(1490.0, 58.0, 160.0 * self.emp_charge, 14.0, emp_col);
        b.text(1664.0, 56.0, if self.emp_charge >= 1.0 { "EMP READY" } else { "EMP" }, 2.2, emp_col);

        let perf_txt = if self.stress_level > 0 {
            format!("{fps:.0} FPS | STRESS +{}MB", self.stress_vram_mb)
        } else {
            format!("{fps:.0} FPS")
        };
        let pw = Batch::text_width(&perf_txt, 2.5);
        b.text(ARENA_R - pw, 26.0, &perf_txt, 2.5, if self.stress_level > 0 { gold } else { mint });

        // Wave banner
        if self.wave_banner > 0.0 && !self.game_over {
            let banner = format!("WAVE {}", self.wave);
            let bw = Batch::text_width(&banner, 9.0);
            let a = (self.wave_banner.min(1.0)).clamp(0.0, 1.0);
            b.text((SCREEN_W as f32 - bw) * 0.5, 220.0, &banner, 9.0, [0.52, 0.96, 0.82, a]);
        }

        // Game Over overlay
        if self.game_over {
            b.rect(520.0, 340.0, 880.0, 340.0, [0.06, 0.10, 0.13, 0.92]);
            b.rect(520.0, 340.0, 880.0, 6.0, [1.0, 0.26, 0.40, 1.0]);
            let t1 = "SYSTEM BREACHED - GAME OVER";
            b.text((SCREEN_W as f32 - Batch::text_width(t1, 4.5)) * 0.5, 395.0, t1, 4.5, [1.0, 0.35, 0.45, 1.0]);
            let t2 = format!("SCORE {}   BEST {}   WAVE {}", self.score, self.save.best_score, self.wave);
            b.text((SCREEN_W as f32 - Batch::text_width(&t2, 3.5)) * 0.5, 475.0, &t2, 3.5, snow);
            let t3 = "HIGH SCORE SAVED TO PLS";
            b.text((SCREEN_W as f32 - Batch::text_width(t3, 2.6)) * 0.5, 535.0, t3, 2.6, mint);
            let t4 = "PRESS X TO LAUNCH AGAIN   |   TOUCHPAD FOR TRINITYXSX MENU";
            b.text((SCREEN_W as f32 - Batch::text_width(t4, 2.6)) * 0.5, 610.0, t4, 2.6, gold);
        }

        // Bottom controls bar
        let hint = "L-STICK MOVE   R-STICK / X FIRE   L2 / SQUARE EMP NOVA   R1 VRAM STRESS   TOUCHPAD MENU";
        b.text(ARENA_L, SCREEN_H as f32 - 46.0, hint, 2.4, muted);
    }
}

fn main() {
    let identity = init_console_runtime();
    println!(
        "NEON VOID XSX: gamertag={} console={} pls={:?}",
        identity.gamertag, identity.console_label, identity.pls_dir
    );

    let mut renderer = match Renderer::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("NEON VOID XSX: D3D12.X init failed: {e}");
            std::process::exit(1);
        }
    };
    let mut pad = Pad::init();
    let mut audio = SynthAudio::init();
    let mut batch = Batch::new();
    let mut game = Game::new(&identity);

    if let Some(a) = audio.as_mut() {
        a.play(Sfx::Wave);
    }

    let mut last = Instant::now();
    let mut fps_timer = Instant::now();
    let mut fps_frames = 0u32;
    let mut fps = 60.0f32;

    loop {
        let now = Instant::now();
        let dt = now.duration_since(last).as_secs_f32().clamp(0.001, 0.05);
        last = now;

        pad.poll(dt);
        game.update(dt, &mut pad, &mut audio, &mut renderer, &identity);
        game.render(&mut batch, fps, &identity);

        let clear = [0.043, 0.070, 0.094, 1.0];
        if let Err(e) = renderer.present_frame(clear, &batch.verts) {
            eprintln!("NEON VOID XSX: PresentX error: {e}");
            break;
        }

        fps_frames += 1;
        let elapsed = fps_timer.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            fps = fps_frames as f32 / elapsed;
            fps_frames = 0;
            fps_timer = Instant::now();
        }
    }

    pad.stop_rumble();
}
