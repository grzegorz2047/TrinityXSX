// SPDX-License-Identifier: MIT
//! 3D banked spline circuits, boost pads, neon tunnel arches, and cyberpunk megacity skyline.

use std::f32::consts::TAU;

use crate::math::{catmull_rom, Vec3};
use crate::scarlett::Vertex3D;

pub const TRACK_COUNT: usize = 3;
pub const TRACK_HALF_WIDTH: f32 = 16.5;

#[derive(Clone, Copy)]
pub struct TrackMeta {
    pub id: &'static str,
    pub name: &'static str,
    pub subtitle: &'static str,
    pub difficulty: &'static str,
    pub gold_secs: f32,
    pub fog_rgb: [f32; 3],
    pub accent_a: [f32; 3],
    pub accent_b: [f32; 3],
}

pub const TRACKS: [TrackMeta; TRACK_COUNT] = [
    TrackMeta {
        id: "neon_skyway",
        name: "NEON SKYWAY",
        subtitle: "TWILIGHT MEGACITY CIRCUIT  -  FAST SWEEPING BANKED TURNS",
        difficulty: "CLASS A  -  INTERMEDIATE",
        gold_secs: 68.0,
        fog_rgb: [0.06, 0.11, 0.19],
        accent_a: [0.18, 0.94, 0.96], // cyan
        accent_b: [0.98, 0.24, 0.72], // magenta
    },
    TrackMeta {
        id: "orbital_ring",
        name: "ORBITAL RING",
        subtitle: "HIGH-ALTITUDE ROLLERCOASTER  -  CORKSCREW BANKING & CHICANES",
        difficulty: "CLASS S  -  TECHNICAL",
        gold_secs: 74.0,
        fog_rgb: [0.08, 0.06, 0.18],
        accent_a: [0.42, 0.98, 0.72], // mint
        accent_b: [0.55, 0.36, 1.00], // violet
    },
    TrackMeta {
        id: "canyon_storm",
        name: "CANYON STORM",
        subtitle: "ENDURANCE CIRCUIT  -  TWIN HYPER-STRAIGHTS & 90-DEG HAIRPINS",
        difficulty: "CLASS S+  -  EXPERT",
        gold_secs: 78.0,
        fog_rgb: [0.14, 0.08, 0.14],
        accent_a: [1.00, 0.62, 0.18], // amber
        accent_b: [0.22, 0.88, 1.00], // ice cyan
    },
];

#[derive(Clone, Copy)]
pub struct Sample {
    pub pos: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub s: f32,       // cumulative distance along circuit
    pub bank: f32,    // radians
    pub boost_x: Option<f32>, // lateral position of boost pad on this segment, if any
}

pub struct Circuit {
    pub meta: TrackMeta,
    pub samples: Vec<Sample>,
    pub total_len: f32,
    pub static_verts: Vec<Vertex3D>,
    pub minimap_pts: Vec<[f32; 2]>, // normalized [0..1, 0..1]
}

impl Circuit {
    pub fn build(index: usize) -> Self {
        let meta = TRACKS[index % TRACK_COUNT];
        let ctrl = control_points(index % TRACK_COUNT);
        let segs_per_ctrl = 24usize;
        let n = ctrl.len() * segs_per_ctrl;

        let mut raw_pos = Vec::with_capacity(n);
        let mut raw_bank = Vec::with_capacity(n);
        for i in 0..ctrl.len() {
            let p0 = ctrl[(i + ctrl.len() - 1) % ctrl.len()];
            let p1 = ctrl[i];
            let p2 = ctrl[(i + 1) % ctrl.len()];
            let p3 = ctrl[(i + 2) % ctrl.len()];
            for k in 0..segs_per_ctrl {
                let t = k as f32 / segs_per_ctrl as f32;
                let pos = catmull_rom(p0.0, p1.0, p2.0, p3.0, t);
                let bank = p1.1 + (p2.1 - p1.1) * (t * t * (3.0 - 2.0 * t));
                raw_pos.push(pos);
                raw_bank.push(bank);
            }
        }

        let mut samples = Vec::with_capacity(n);
        let mut total_len = 0.0f32;
        for i in 0..n {
            let p_prev = raw_pos[(i + n - 1) % n];
            let p_cur = raw_pos[i];
            let p_next = raw_pos[(i + 1) % n];
            if i > 0 {
                total_len += p_cur.sub(p_prev).len();
            }
            let forward = p_next.sub(p_prev).norm();
            let flat_right = Vec3::UP.cross(forward).norm();
            let flat_up = forward.cross(flat_right).norm();
            let bank = raw_bank[i];
            let (sb, cb) = bank.sin_cos();
            let right = flat_right.mul(cb).add(flat_up.mul(sb)).norm();
            let up = forward.cross(right).norm();

            // Place chevron boost pads every ~32 segments, alternating lateral lanes
            let boost_x = if i > 10 && i % 32 == 0 {
                let lane = match (i / 32) % 3 {
                    0 => 0.0,
                    1 => -7.5,
                    _ => 7.5,
                };
                Some(lane)
            } else {
                None
            };

            samples.push(Sample { pos: p_cur, forward, right, up, s: total_len, bank, boost_x });
        }
        total_len += raw_pos[0].sub(raw_pos[n - 1]).len();

        // Build normalized minimap points
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
        for s in &samples {
            min_x = min_x.min(s.pos.x);
            max_x = max_x.max(s.pos.x);
            min_z = min_z.min(s.pos.z);
            max_z = max_z.max(s.pos.z);
        }
        let span = (max_x - min_x).max(max_z - min_z).max(1.0);
        let cx = (min_x + max_x) * 0.5;
        let cz = (min_z + max_z) * 0.5;
        let minimap_pts = samples
            .iter()
            .step_by(3)
            .map(|s| [(s.pos.x - cx) / span + 0.5, (s.pos.z - cz) / span + 0.5])
            .collect();

        let static_verts = build_world_mesh(&meta, &samples);
        Self { meta, samples, total_len, static_verts, minimap_pts }
    }

    pub fn sample_at(&self, mut s: f32) -> Sample {
        s = s.rem_euclid(self.total_len);
        let n = self.samples.len();
        // Binary search on cumulative distance
        let idx = match self.samples.binary_search_by(|sm| sm.s.total_cmp(&s)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let a = self.samples[idx % n];
        let b = self.samples[(idx + 1) % n];
        let seg_len = if idx + 1 < n { b.s - a.s } else { self.total_len - a.s }.max(0.001);
        let t = ((s - a.s) / seg_len).clamp(0.0, 1.0);
        let pos = a.pos.lerp(b.pos, t);
        let forward = a.forward.lerp(b.forward, t).norm();
        let right = a.right.lerp(b.right, t).norm();
        let up = forward.cross(right).norm();
        Sample {
            pos,
            forward,
            right,
            up,
            s,
            bank: a.bank + (b.bank - a.bank) * t,
            boost_x: a.boost_x,
        }
    }

    pub fn minimap_coord(&self, world_pos: Vec3) -> [f32; 2] {
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
        for s in self.samples.iter().step_by(8) {
            min_x = min_x.min(s.pos.x);
            max_x = max_x.max(s.pos.x);
            min_z = min_z.min(s.pos.z);
            max_z = max_z.max(s.pos.z);
        }
        let span = (max_x - min_x).max(max_z - min_z).max(1.0);
        let cx = (min_x + max_x) * 0.5;
        let cz = (min_z + max_z) * 0.5;
        [(world_pos.x - cx) / span + 0.5, (world_pos.z - cz) / span + 0.5]
    }
}

fn control_points(track: usize) -> Vec<(Vec3, f32)> {
    match track {
        0 => vec![
            // NEON SKYWAY: wide twilight megacity circuit with sweeping banked turns
            (Vec3::new(0.0, 26.0, 0.0), 0.0),
            (Vec3::new(0.0, 28.0, 260.0), 0.0),
            (Vec3::new(90.0, 46.0, 480.0), 0.36),
            (Vec3::new(310.0, 56.0, 540.0), 0.42),
            (Vec3::new(500.0, 38.0, 390.0), 0.34),
            (Vec3::new(470.0, 22.0, 140.0), -0.26),
            (Vec3::new(620.0, 42.0, -90.0), 0.38),
            (Vec3::new(560.0, 54.0, -340.0), 0.44),
            (Vec3::new(310.0, 36.0, -450.0), 0.30),
            (Vec3::new(110.0, 18.0, -300.0), -0.28),
            (Vec3::new(-40.0, 24.0, -180.0), 0.20),
        ],
        1 => vec![
            // ORBITAL RING: dramatic elevation rollercoaster & S-curves
            (Vec3::new(0.0, 34.0, 0.0), 0.0),
            (Vec3::new(0.0, 55.0, 250.0), 0.12),
            (Vec3::new(140.0, 88.0, 450.0), 0.48),
            (Vec3::new(390.0, 62.0, 490.0), 0.44),
            (Vec3::new(450.0, 24.0, 270.0), -0.42),
            (Vec3::new(260.0, 32.0, 110.0), -0.35),
            (Vec3::new(430.0, 68.0, -110.0), 0.46),
            (Vec3::new(520.0, 80.0, -350.0), 0.50),
            (Vec3::new(240.0, 44.0, -470.0), 0.32),
            (Vec3::new(40.0, 22.0, -320.0), 0.25),
            (Vec3::new(-30.0, 28.0, -160.0), 0.10),
        ],
        _ => vec![
            // CANYON STORM: twin hyper-straights + technical canyon hairpins
            (Vec3::new(0.0, 24.0, 0.0), 0.0),
            (Vec3::new(0.0, 24.0, 360.0), 0.0),
            (Vec3::new(70.0, 42.0, 590.0), 0.52),
            (Vec3::new(250.0, 52.0, 610.0), 0.52),
            (Vec3::new(340.0, 34.0, 400.0), 0.36),
            (Vec3::new(540.0, 20.0, 220.0), -0.44),
            (Vec3::new(660.0, 30.0, -20.0), 0.48),
            (Vec3::new(560.0, 54.0, -290.0), 0.50),
            (Vec3::new(340.0, 46.0, -440.0), 0.34),
            (Vec3::new(100.0, 28.0, -460.0), 0.40),
            (Vec3::new(-30.0, 22.0, -260.0), 0.18),
        ],
    }
}

fn push_quad(out: &mut Vec<Vertex3D>, a: Vec3, b: Vec3, c: Vec3, d: Vec3, n: Vec3, col: [f32; 4], mat: [f32; 2]) {
    let va = Vertex3D { pos: a.to_arr(), normal: n.to_arr(), col, mat };
    let vb = Vertex3D { pos: b.to_arr(), normal: n.to_arr(), col, mat };
    let vc = Vertex3D { pos: c.to_arr(), normal: n.to_arr(), col, mat };
    let vd = Vertex3D { pos: d.to_arr(), normal: n.to_arr(), col, mat };
    out.extend_from_slice(&[va, vb, vc, va, vc, vd]);
}

fn push_box(out: &mut Vec<Vertex3D>, center: Vec3, half: Vec3, col: [f32; 4], mat: [f32; 2]) {
    let (cx, cy, cz) = (center.x, center.y, center.z);
    let (hx, hy, hz) = (half.x, half.y, half.z);
    let p = |sx: f32, sy: f32, sz: f32| Vec3::new(cx + sx * hx, cy + sy * hy, cz + sz * hz);
    // Top
    push_quad(out, p(-1.0, 1.0, -1.0), p(-1.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, -1.0), Vec3::UP, col, mat);
    // Four vertical walls
    push_quad(out, p(-1.0, -1.0, 1.0), p(-1.0, 1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, -1.0, 1.0), Vec3::new(0.0, 0.0, 1.0), col, mat);
    push_quad(out, p(1.0, -1.0, -1.0), p(1.0, 1.0, -1.0), p(-1.0, 1.0, -1.0), p(-1.0, -1.0, -1.0), Vec3::new(0.0, 0.0, -1.0), col, mat);
    push_quad(out, p(1.0, -1.0, 1.0), p(1.0, 1.0, 1.0), p(1.0, 1.0, -1.0), p(1.0, -1.0, -1.0), Vec3::new(1.0, 0.0, 0.0), col, mat);
    push_quad(out, p(-1.0, -1.0, -1.0), p(-1.0, 1.0, -1.0), p(-1.0, 1.0, 1.0), p(-1.0, -1.0, 1.0), Vec3::new(-1.0, 0.0, 0.0), col, mat);
}

fn build_world_mesh(meta: &TrackMeta, samples: &[Sample]) -> Vec<Vertex3D> {
    let mut v = Vec::with_capacity(90_000);
    let n = samples.len();
    let hw = TRACK_HALF_WIDTH;
    let ac_a = [meta.accent_a[0], meta.accent_a[1], meta.accent_a[2], 1.0];
    let ac_b = [meta.accent_b[0], meta.accent_b[1], meta.accent_b[2], 1.0];

    for i in 0..n {
        let a = samples[i];
        let b = samples[(i + 1) % n];

        // 1. Wet dark-metallic racing deck (with alternating subtle sheen bands)
        let dark = if (i / 2) % 2 == 0 {
            [0.09, 0.12, 0.17, 1.0]
        } else {
            [0.075, 0.105, 0.15, 1.0]
        };
        let al = a.pos.sub(a.right.mul(hw));
        let ar = a.pos.add(a.right.mul(hw));
        let bl = b.pos.sub(b.right.mul(hw));
        let br = b.pos.add(b.right.mul(hw));
        push_quad(&mut v, al, bl, br, ar, a.up, dark, [0.95, 0.0]);

        // Underside structural hull so elevated track looks solid from below/curves
        let drop_a = a.up.mul(-2.4);
        let drop_b = b.up.mul(-2.4);
        push_quad(&mut v, al, al.add(drop_a), bl.add(drop_b), bl, a.right.mul(-1.0), [0.05, 0.07, 0.10, 1.0], [0.4, 0.0]);
        push_quad(&mut v, ar, br, br.add(drop_b), ar.add(drop_a), a.right, [0.05, 0.07, 0.10, 1.0], [0.4, 0.0]);

        // 2. Crisp reflective center lane dash & start/finish checkered banner
        if i == 0 || i == 1 {
            let strip_col = if i == 0 { [0.98, 0.99, 1.0, 1.0] } else { ac_a };
            push_quad(
                &mut v,
                al.add(a.up.mul(0.08)),
                bl.add(b.up.mul(0.08)),
                br.add(b.up.mul(0.08)),
                ar.add(a.up.mul(0.08)),
                a.up,
                strip_col,
                [0.4, 1.3],
            );
        } else if i % 4 < 2 {
            let cw = 0.35;
            push_quad(
                &mut v,
                a.pos.sub(a.right.mul(cw)).add(a.up.mul(0.06)),
                b.pos.sub(b.right.mul(cw)).add(b.up.mul(0.06)),
                b.pos.add(b.right.mul(cw)).add(b.up.mul(0.06)),
                a.pos.add(a.right.mul(cw)).add(a.up.mul(0.06)),
                a.up,
                [0.55, 0.85, 0.95, 1.0],
                [0.3, 0.65],
            );
        }

        // 3. Glowing neon side curb + elevated energy guardrails
        let rail_h = 2.1;
        let curb_w = 1.1;
        let rail_col_l = if (i / 6) % 2 == 0 { ac_a } else { [0.95, 0.98, 1.0, 1.0] };
        let rail_col_r = if (i / 6) % 2 == 0 { ac_b } else { [0.95, 0.98, 1.0, 1.0] };

        // Left & right glowing edge strips on deck
        push_quad(
            &mut v,
            al.add(a.up.mul(0.06)),
            bl.add(b.up.mul(0.06)),
            bl.add(b.right.mul(curb_w)).add(b.up.mul(0.06)),
            al.add(a.right.mul(curb_w)).add(a.up.mul(0.06)),
            a.up,
            rail_col_l,
            [0.2, 1.5],
        );
        push_quad(
            &mut v,
            ar.sub(a.right.mul(curb_w)).add(a.up.mul(0.06)),
            br.sub(b.right.mul(curb_w)).add(b.up.mul(0.06)),
            br.add(b.up.mul(0.06)),
            ar.add(a.up.mul(0.06)),
            a.up,
            rail_col_r,
            [0.2, 1.5],
        );

        // Raised barrier walls with glowing top neon tube
        let la_top = al.add(a.up.mul(rail_h));
        let lb_top = bl.add(b.up.mul(rail_h));
        let ra_top = ar.add(a.up.mul(rail_h));
        let rb_top = br.add(b.up.mul(rail_h));
        push_quad(&mut v, al, la_top, lb_top, bl, a.right, [0.10, 0.14, 0.20, 1.0], [0.7, 0.05]);
        push_quad(&mut v, ar, br, rb_top, ra_top, a.right.mul(-1.0), [0.10, 0.14, 0.20, 1.0], [0.7, 0.05]);
        // Bright neon rail cap
        push_quad(
            &mut v,
            la_top.sub(a.up.mul(0.32)),
            la_top,
            lb_top,
            lb_top.sub(b.up.mul(0.32)),
            a.right,
            ac_a,
            [0.1, 1.75],
        );
        push_quad(
            &mut v,
            ra_top.sub(a.up.mul(0.32)),
            rb_top.sub(b.up.mul(0.32)),
            rb_top,
            ra_top,
            a.right.mul(-1.0),
            ac_b,
            [0.1, 1.75],
        );

        // 4. Glowing Chevron Boost Pads
        if let Some(bx) = a.boost_x {
            let pad_w = 3.2;
            for chev in 0..3 {
                let s0 = samples[(i + chev) % n];
                let s1 = samples[(i + chev + 1) % n];
                let c0 = s0.pos.add(s0.right.mul(bx)).add(s0.up.mul(0.11));
                let c1 = s1.pos.add(s1.right.mul(bx)).add(s1.up.mul(0.11));
                let tip = c1.add(s1.forward.mul(2.8));
                let l0 = c0.sub(s0.right.mul(pad_w));
                let r0 = c0.add(s0.right.mul(pad_w));
                let l1 = c1.sub(s1.right.mul(pad_w * 0.55));
                let r1 = c1.add(s1.right.mul(pad_w * 0.55));
                let glow_col = if chev == 2 { [0.95, 1.0, 0.65, 1.0] } else { [0.18, 1.0, 0.88, 1.0] };
                push_quad(&mut v, l0, l1, tip, c0.add(s0.forward.mul(1.8)), s0.up, glow_col, [0.1, 2.0]);
                push_quad(&mut v, c0.add(s0.forward.mul(1.8)), tip, r1, r0, s0.up, glow_col, [0.1, 2.0]);
            }
        }

        // 5. Monumental Neon Overhead Arches & Support Pylons every 14 segments
        if i % 14 == 0 {
            let arch_h = if i == 0 { 16.5 } else { 13.5 };
            let thick = if i == 0 { 2.2 } else { 1.2 };
            let arch_col = if i == 0 { [0.96, 0.99, 1.0, 1.0] } else if (i / 14) % 2 == 0 { ac_a } else { ac_b };
            let dark_beam = [0.08, 0.11, 0.16, 1.0];

            // Left & right arch pillars + glowing inner strip
            let l_base = al.sub(a.right.mul(0.6));
            let r_base = ar.add(a.right.mul(0.6));
            let l_top = l_base.add(a.up.mul(arch_h));
            let r_top = r_base.add(a.up.mul(arch_h));
            let fwd_t = a.forward.mul(thick);

            push_quad(&mut v, l_base, l_top, l_top.add(fwd_t), l_base.add(fwd_t), a.right, dark_beam, [0.5, 0.0]);
            push_quad(&mut v, r_base, r_base.add(fwd_t), r_top.add(fwd_t), r_top, a.right.mul(-1.0), dark_beam, [0.5, 0.0]);
            // Top crossbar
            push_quad(&mut v, l_top, r_top, r_top.add(fwd_t), l_top.add(fwd_t), a.up.mul(-1.0), arch_col, [0.2, 1.8]);
            push_quad(
                &mut v,
                l_top,
                l_top.add(a.up.mul(1.2)),
                r_top.add(a.up.mul(1.2)),
                r_top,
                a.forward.mul(-1.0),
                arch_col,
                [0.2, 1.5],
            );

            // Tall support pillar plunging down into the twilight megacity mist
            let p_bot = a.pos.sub(Vec3::new(0.0, 180.0, 0.0));
            let pw = 2.4;
            push_quad(
                &mut v,
                a.pos.sub(a.right.mul(pw)).sub(a.up.mul(2.4)),
                p_bot.sub(a.right.mul(pw)),
                p_bot.add(a.right.mul(pw)),
                a.pos.add(a.right.mul(pw)).sub(a.up.mul(2.4)),
                a.forward.mul(-1.0),
                [0.05, 0.07, 0.11, 1.0],
                [0.3, 0.0],
            );
        }
    }

    // 6. Procedural Cyberpunk Megacity Skyline rising out of the clouds around & beneath the circuit
    let mut rng = 0xAE74_E8D1u64;
    let mut next_f = || -> f32 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 32) as f32) / (u32::MAX as f32)
    };

    for i in (0..n).step_by(3) {
        let s = samples[i];
        for side in [-1.0f32, 1.0f32] {
            let lateral = TRACK_HALF_WIDTH + 30.0 + next_f() * 145.0;
            let bx = s.pos.x + s.right.x * side * lateral + (next_f() - 0.5) * 28.0;
            let bz = s.pos.z + s.right.z * side * lateral + (next_f() - 0.5) * 28.0;

            // Keep skyscrapers safely clear of any track segment
            let mut too_close = false;
            for chk in samples.iter().step_by(6) {
                let dx = bx - chk.pos.x;
                let dz = bz - chk.pos.z;
                if dx * dx + dz * dz < 42.0 * 42.0 {
                    too_close = true;
                    break;
                }
            }
            if too_close {
                continue;
            }

            let w = 11.0 + next_f() * 20.0;
            let d = 11.0 + next_f() * 20.0;
            let top_y = s.pos.y - 35.0 + next_f() * 135.0;
            let base_y = -170.0;
            let h = (top_y - base_y).max(40.0);
            let center = Vec3::new(bx, base_y + h * 0.5, bz);
            let tower_col = [0.05 + next_f() * 0.04, 0.07 + next_f() * 0.05, 0.12 + next_f() * 0.06, 1.0];
            push_box(&mut v, center, Vec3::new(w * 0.5, h * 0.5, d * 0.5), tower_col, [0.55, 0.0]);

            // Glowing neon rooftop trim & window accent band
            let neon = match (next_f() * 3.0) as u32 {
                0 => ac_a,
                1 => ac_b,
                _ => [1.0, 0.72, 0.25, 1.0],
            };
            push_box(
                &mut v,
                Vec3::new(bx, top_y - 1.5, bz),
                Vec3::new(w * 0.52, 1.3, d * 0.52),
                neon,
                [0.1, 1.65],
            );
            // Mid-tower glowing billboard band
            if next_f() > 0.35 {
                let mid_y = top_y - 18.0 - next_f() * 32.0;
                push_box(
                    &mut v,
                    Vec3::new(bx, mid_y, bz),
                    Vec3::new(w * 0.515, 2.2, d * 0.515),
                    neon,
                    [0.1, 1.25],
                );
            }
        }
    }

    // 7. Distant twilight horizon ring (gives deep sunset/moonlight sky framing)
    let sky_r = 1650.0;
    let sky_segs = 36;
    for i in 0..sky_segs {
        let a0 = (i as f32 / sky_segs as f32) * TAU;
        let a1 = ((i + 1) as f32 / sky_segs as f32) * TAU;
        let cx = 250.0;
        let cz = 40.0;
        let p00 = Vec3::new(cx + a0.cos() * sky_r, -140.0, cz + a0.sin() * sky_r);
        let p01 = Vec3::new(cx + a1.cos() * sky_r, -140.0, cz + a1.sin() * sky_r);
        let p10 = Vec3::new(cx + a0.cos() * sky_r, 240.0, cz + a0.sin() * sky_r);
        let p11 = Vec3::new(cx + a1.cos() * sky_r, 240.0, cz + a1.sin() * sky_r);
        let n_in = Vec3::new(-a0.cos(), 0.0, -a0.sin());
        let glow = (a0.sin() * 0.5 + 0.5).powf(2.0);
        let hor = [
            meta.fog_rgb[0] + glow * 0.22,
            meta.fog_rgb[1] + glow * 0.14,
            meta.fog_rgb[2] + glow * 0.24,
            1.0,
        ];
        push_quad(&mut v, p00, p10, p11, p01, n_in, hor, [0.0, 0.9]);
    }

    v
}
