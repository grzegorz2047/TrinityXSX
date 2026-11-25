// SPDX-License-Identifier: MIT
//! 3D banked spline circuits, wet UV-mapped track deck, cantilevered cyberpunk billboards & turn
//! chevrons, Blender-sculpted megacity spires, and mirrored track-reflection geometry.

use std::f32::consts::TAU;

use crate::assets::{emit_mesh_world, MeshAsset};
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
        name: "NEO-KYOTO SKYWAY",
        subtitle: "TWILIGHT MEGACITY CIRCUIT  -  WET REFLECTIVE BANKED CURVES",
        difficulty: "CLASS A  -  INTERMEDIATE",
        gold_secs: 68.0,
        fog_rgb: [0.06, 0.10, 0.19],
        accent_a: [0.16, 0.96, 0.98], // cyan
        accent_b: [0.98, 0.24, 0.78], // magenta
    },
    TrackMeta {
        id: "orbital_ring",
        name: "ORBITAL RING",
        subtitle: "HIGH-ALTITUDE ROLLERCOASTER  -  CORKSCREW BANKING & CHICANES",
        difficulty: "CLASS S  -  TECHNICAL",
        gold_secs: 74.0,
        fog_rgb: [0.08, 0.06, 0.18],
        accent_a: [0.32, 0.98, 0.76], // mint
        accent_b: [0.68, 0.32, 1.00], // violet
    },
    TrackMeta {
        id: "canyon_storm",
        name: "CANYON STORM",
        subtitle: "ENDURANCE CIRCUIT  -  TWIN HYPER-STRAIGHTS & 90-DEG HAIRPINS",
        difficulty: "CLASS S+  -  EXPERT",
        gold_secs: 78.0,
        fog_rgb: [0.13, 0.08, 0.15],
        accent_a: [1.00, 0.62, 0.18], // amber
        accent_b: [0.20, 0.92, 1.00], // ice cyan
    },
];

#[derive(Clone, Copy)]
pub struct Sample {
    pub pos: Vec3,
    pub forward: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub s: f32,
    pub bank: f32,
    pub boost_x: Option<f32>,
}

pub struct StaticWorld {
    pub track_verts: Vec<Vertex3D>,     // wet textured road deck (writes Stencil = 1)
    pub rail_verts: Vec<Vertex3D>,      // metallic barriers, undersides, glowing rail LEDs, arches
    pub city_verts: Vec<Vertex3D>,      // Blender skyscraper spires with illuminated window atlas
    pub billboard_verts: Vec<Vertex3D>, // NEO-KYOTO / VORTEX / CHRONOS / AETHER-1 + Turn Chevrons
    pub refl_verts: Vec<Vertex3D>,      // mirrored rail LEDs + mirrored arches + mirrored billboards
    pub mist_verts: Vec<Vertex3D>,      // layered volumetric swirling megacity mist & cloud sheets
}

pub struct Circuit {
    pub meta: TrackMeta,
    pub samples: Vec<Sample>,
    pub total_len: f32,
    pub static_world: StaticWorld,
    pub minimap_pts: Vec<[f32; 2]>,
}

impl Circuit {
    pub fn build(index: usize, skyscraper_mesh: &MeshAsset) -> Self {
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

            let boost_x = if i > 10 && i % 32 == 0 {
                let lane = match (i / 32) % 3 {
                    0 => 0.0,
                    1 => -7.2,
                    _ => 7.2,
                };
                Some(lane)
            } else {
                None
            };

            samples.push(Sample {
                pos: p_cur,
                forward,
                right,
                up,
                s: total_len,
                bank,
                boost_x,
            });
        }
        total_len += raw_pos[0].sub(raw_pos[n - 1]).len();

        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
        for sm in &samples {
            min_x = min_x.min(sm.pos.x);
            max_x = max_x.max(sm.pos.x);
            min_z = min_z.min(sm.pos.z);
            max_z = max_z.max(sm.pos.z);
        }
        let span = (max_x - min_x).max(max_z - min_z).max(1.0);
        let ox = (min_x + max_x) * 0.5;
        let oz = (min_z + max_z) * 0.5;
        let minimap_pts: Vec<[f32; 2]> = samples
            .iter()
            .step_by(3)
            .map(|sm| [0.5 + (sm.pos.x - ox) / span, 0.5 - (sm.pos.z - oz) / span])
            .collect();

        let static_world = build_world_meshes(&meta, &samples, skyscraper_mesh);

        Self {
            meta,
            samples,
            total_len,
            static_world,
            minimap_pts,
        }
    }

    pub fn sample_at(&self, dist: f32) -> Sample {
        let mut s = dist % self.total_len;
        if s < 0.0 {
            s += self.total_len;
        }
        let n = self.samples.len();
        let idx = match self.samples.binary_search_by(|sm| sm.s.total_cmp(&s)) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let a = self.samples[idx % n];
        let b = self.samples[(idx + 1) % n];
        let seg_len = if idx + 1 < n {
            (b.s - a.s).max(0.001)
        } else {
            (self.total_len - a.s).max(0.001)
        };
        let t = ((s - a.s) / seg_len).clamp(0.0, 1.0);
        Sample {
            pos: a.pos.lerp(b.pos, t),
            forward: a.forward.lerp(b.forward, t).norm(),
            right: a.right.lerp(b.right, t).norm(),
            up: a.up.lerp(b.up, t).norm(),
            s,
            bank: a.bank + (b.bank - a.bank) * t,
            boost_x: a.boost_x,
        }
    }

    pub fn minimap_coord(&self, pos: Vec3) -> [f32; 2] {
        let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
        let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
        for sm in &self.samples {
            min_x = min_x.min(sm.pos.x);
            max_x = max_x.max(sm.pos.x);
            min_z = min_z.min(sm.pos.z);
            max_z = max_z.max(sm.pos.z);
        }
        let span = (max_x - min_x).max(max_z - min_z).max(1.0);
        let ox = (min_x + max_x) * 0.5;
        let oz = (min_z + max_z) * 0.5;
        [0.5 + (pos.x - ox) / span, 0.5 - (pos.z - oz) / span]
    }
}

fn control_points(idx: usize) -> Vec<(Vec3, f32)> {
    match idx {
        0 => vec![
            (Vec3::new(0.0, 26.0, 0.0), 0.0),
            (Vec3::new(0.0, 26.0, 240.0), 0.05),
            (Vec3::new(110.0, 44.0, 460.0), 0.36),
            (Vec3::new(340.0, 56.0, 520.0), 0.42),
            (Vec3::new(560.0, 38.0, 360.0), 0.32),
            (Vec3::new(620.0, 22.0, 90.0), 0.28),
            (Vec3::new(470.0, 32.0, -140.0), -0.34),
            (Vec3::new(530.0, 54.0, -370.0), 0.40),
            (Vec3::new(320.0, 62.0, -560.0), 0.42),
            (Vec3::new(80.0, 40.0, -490.0), 0.28),
            (Vec3::new(-40.0, 26.0, -280.0), 0.14),
        ],
        1 => vec![
            (Vec3::new(0.0, 34.0, 0.0), 0.0),
            (Vec3::new(15.0, 58.0, 260.0), -0.22),
            (Vec3::new(-160.0, 94.0, 480.0), -0.48),
            (Vec3::new(20.0, 118.0, 660.0), 0.52),
            (Vec3::new(290.0, 74.0, 600.0), 0.46),
            (Vec3::new(430.0, 26.0, 320.0), -0.36),
            (Vec3::new(580.0, 46.0, 20.0), 0.44),
            (Vec3::new(460.0, 88.0, -260.0), 0.50),
            (Vec3::new(190.0, 96.0, -420.0), 0.34),
            (Vec3::new(-90.0, 54.0, -340.0), 0.36),
            (Vec3::new(-50.0, 34.0, -170.0), 0.12),
        ],
        _ => vec![
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

fn push_quad_uv(
    out: &mut Vec<Vertex3D>,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    d: Vec3,
    n: Vec3,
    uva: [f32; 2],
    uvb: [f32; 2],
    uvc: [f32; 2],
    uvd: [f32; 2],
    col: [f32; 4],
    mat: [f32; 2],
) {
    let va = Vertex3D { pos: a.to_arr(), normal: n.to_arr(), uv: uva, col, mat };
    let vb = Vertex3D { pos: b.to_arr(), normal: n.to_arr(), uv: uvb, col, mat };
    let vc = Vertex3D { pos: c.to_arr(), normal: n.to_arr(), uv: uvc, col, mat };
    let vd = Vertex3D { pos: d.to_arr(), normal: n.to_arr(), uv: uvd, col, mat };
    out.extend_from_slice(&[va, vb, vc, va, vc, vd]);
}

fn push_quad(out: &mut Vec<Vertex3D>, a: Vec3, b: Vec3, c: Vec3, d: Vec3, n: Vec3, col: [f32; 4], mat: [f32; 2]) {
    push_quad_uv(out, a, b, c, d, n, [0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0], col, mat);
}

fn reflect_quad(
    out: &mut Vec<Vertex3D>,
    a: Vec3,
    b: Vec3,
    c: Vec3,
    d: Vec3,
    n: Vec3,
    plane_o: Vec3,
    plane_u: Vec3,
    col: [f32; 4],
    mat: [f32; 2],
) {
    let rp = |p: Vec3| p.sub(plane_u.mul(2.0 * p.sub(plane_o).dot(plane_u)));
    let rn = n.sub(plane_u.mul(2.0 * n.dot(plane_u)));
    push_quad(out, rp(a), rp(d), rp(c), rp(b), rn, col, mat);
}

fn build_world_meshes(meta: &TrackMeta, samples: &[Sample], skyscraper: &MeshAsset) -> StaticWorld {
    let mut track = Vec::with_capacity(8_000);
    let mut rails = Vec::with_capacity(48_000);
    let mut city = Vec::with_capacity(32_000);
    let mut billboards = Vec::with_capacity(6_000);
    let mut refl = Vec::with_capacity(18_000);

    let n = samples.len();
    let hw = TRACK_HALF_WIDTH;
    let ac_a = [meta.accent_a[0], meta.accent_a[1], meta.accent_a[2], 1.0];
    let ac_b = [meta.accent_b[0], meta.accent_b[1], meta.accent_b[2], 1.0];

    for i in 0..n {
        let a = samples[i];
        let b = samples[(i + 1) % n];

        let al = a.pos.sub(a.right.mul(hw));
        let ar = a.pos.add(a.right.mul(hw));
        let bl = b.pos.sub(b.right.mul(hw));
        let br = b.pos.add(b.right.mul(hw));

        // 1. Wet Textured Metallic Road Deck (UV tiled smoothly along s!)
        let v0 = a.s * 0.036;
        let v1 = if i + 1 < n { b.s * 0.036 } else { (a.s + b.pos.sub(a.pos).len()) * 0.036 };
        let deck_col = if i == 0 { [1.35, 1.45, 1.55, 1.0] } else { [1.0, 1.0, 1.0, 1.0] };
        let deck_emiss = if i == 0 { 0.85 } else { 0.0 };
        push_quad_uv(
            &mut track,
            al,
            bl,
            br,
            ar,
            a.up,
            [0.0, v0],
            [0.0, v1],
            [1.0, v1],
            [1.0, v0],
            deck_col,
            [0.95, deck_emiss],
        );

        // 2. Structural Metallic Underside Girder & Outer Ribbing (like concept art!)
        let drop_a = a.up.mul(-2.8);
        let drop_b = b.up.mul(-2.8);
        let dark_steel = [0.06, 0.09, 0.14, 1.0];
        push_quad(&mut rails, al, al.add(drop_a), bl.add(drop_b), bl, a.right.mul(-1.0), dark_steel, [0.75, 0.0]);
        push_quad(&mut rails, ar, br, br.add(drop_b), ar.add(drop_a), a.right, dark_steel, [0.75, 0.0]);
        // Bottom soffit plate
        push_quad(&mut rails, al.add(drop_a), ar.add(drop_a), br.add(drop_b), bl.add(drop_b), a.up.mul(-1.0), [0.04, 0.06, 0.10, 1.0], [0.5, 0.0]);

        // Structural under-rail glowing cyan/magenta ribs every 3 segments
        if i % 3 == 0 {
            let rib_col = if (i / 3) % 2 == 0 { ac_a } else { ac_b };
            push_quad(
                &mut rails,
                al.add(a.up.mul(-0.9)).sub(a.right.mul(0.25)),
                al.add(a.up.mul(-1.7)).sub(a.right.mul(0.25)),
                bl.add(b.up.mul(-1.7)).sub(b.right.mul(0.25)),
                bl.add(b.up.mul(-0.9)).sub(b.right.mul(0.25)),
                a.right.mul(-1.0),
                rib_col,
                [0.2, 1.8],
            );
            push_quad(
                &mut rails,
                ar.add(a.up.mul(-0.9)).add(a.right.mul(0.25)),
                br.add(b.up.mul(-0.9)).add(b.right.mul(0.25)),
                br.add(b.up.mul(-1.7)).add(b.right.mul(0.25)),
                ar.add(a.up.mul(-1.7)).add(a.right.mul(0.25)),
                a.right,
                rib_col,
                [0.2, 1.8],
            );
        }

        // 3. Sculpted Side Barrier Walls + Segmented Glowing Neon LED Bars (Cyan + Magenta!)
        let bh = 2.4;
        let la_top = al.add(a.up.mul(bh));
        let lb_top = bl.add(b.up.mul(bh));
        let ra_top = ar.add(a.up.mul(bh));
        let rb_top = br.add(b.up.mul(bh));
        // Outer metallic rail housing
        push_quad(&mut rails, al, la_top, lb_top, bl, a.right, [0.09, 0.13, 0.20, 1.0], [0.85, 0.04]);
        push_quad(&mut rails, ar, br, rb_top, ra_top, a.right.mul(-1.0), [0.09, 0.13, 0.20, 1.0], [0.85, 0.04]);
        // Top beveled rail cap
        push_quad(&mut rails, la_top, la_top.sub(a.right.mul(0.9)), lb_top.sub(b.right.mul(0.9)), lb_top, a.up, [0.14, 0.19, 0.28, 1.0], [0.9, 0.08]);
        push_quad(&mut rails, ra_top, rb_top, rb_top.add(b.right.mul(0.9)), ra_top.add(a.right.mul(0.9)), a.up, [0.14, 0.19, 0.28, 1.0], [0.9, 0.08]);

        // Segmented glowing neon LED tubes along inner barrier faces (mirrors onto wet track!)
        // Leave a crisp dark structural gap on every 3rd segment for authentic concept-art LED bars!
        let seg_on = i % 2 == 0;
        let l_led_col = if seg_on { ac_b } else { ac_a };
        let r_led_col = if seg_on { ac_a } else { ac_b };

        if i % 3 != 2 {
            let l_led0 = al.add(a.up.mul(0.55)).add(a.right.mul(0.14));
            let l_led1 = al.add(a.up.mul(1.45)).add(a.right.mul(0.14));
            let l_led2 = bl.add(b.up.mul(1.45)).add(b.right.mul(0.14));
            let l_led3 = bl.add(b.up.mul(0.55)).add(b.right.mul(0.14));
            push_quad(&mut rails, l_led0, l_led1, l_led2, l_led3, a.right, l_led_col, [0.15, 1.45]);
            reflect_quad(&mut refl, l_led0, l_led1, l_led2, l_led3, a.right, a.pos, a.up, [l_led_col[0], l_led_col[1], l_led_col[2], 0.65], [0.1, 1.25]);

            let r_led0 = ar.add(a.up.mul(0.55)).sub(a.right.mul(0.14));
            let r_led1 = br.add(b.up.mul(0.55)).sub(b.right.mul(0.14));
            let r_led2 = br.add(b.up.mul(1.45)).sub(b.right.mul(0.14));
            let r_led3 = ar.add(a.up.mul(1.45)).sub(a.right.mul(0.14));
            push_quad(&mut rails, r_led0, r_led1, r_led2, r_led3, a.right.mul(-1.0), r_led_col, [0.15, 1.45]);
            reflect_quad(&mut refl, r_led0, r_led1, r_led2, r_led3, a.right.mul(-1.0), a.pos, a.up, [r_led_col[0], r_led_col[1], r_led_col[2], 0.65], [0.1, 1.25]);
        }

        // 4. Glowing Boost Pad Chevrons embedded in road
        if let Some(bx) = a.boost_x {
            let s0 = a;
            let s1 = samples[(i + 2) % n];
            for k in 0..3 {
                let frac0 = k as f32 * 0.34;
                let frac1 = frac0 + 0.24;
                let c0 = s0.pos.lerp(s1.pos, frac0).add(s0.right.mul(bx)).add(s0.up.mul(0.14));
                let c1 = s0.pos.lerp(s1.pos, frac1).add(s0.right.mul(bx)).add(s0.up.mul(0.14));
                let pw = 2.4;
                let glow_col = if k == 1 { [0.42, 0.96, 1.0, 1.0] } else { [0.16, 0.88, 0.96, 1.0] };
                let l0 = c0.sub(s0.right.mul(pw));
                let r0 = c0.add(s0.right.mul(pw));
                let tip = c1.add(s0.forward.mul(2.0));
                let l1 = c1.sub(s0.right.mul(pw));
                let r1 = c1.add(s0.right.mul(pw));
                push_quad(&mut rails, l0, l1, tip, c0.add(s0.forward.mul(1.8)), s0.up, glow_col, [0.1, 1.45]);
                push_quad(&mut rails, c0.add(s0.forward.mul(1.8)), tip, r1, r0, s0.up, glow_col, [0.1, 1.45]);
            }
        }

        // 5. Concept-Art Cantilevered Cyberpunk Billboards & Animated Turn Chevrons along curves!
        if i % 6 == 0 {
            let outer_sign = if a.bank >= 0.0 { 1.0 } else { -1.0 };
            let b_end = samples[(i + 4) % n];
            let p0 = a.pos.add(a.right.mul(outer_sign * (hw + 1.6)));
            let p1 = b_end.pos.add(b_end.right.mul(outer_sign * (hw + 1.6)));
            let face_n = a.right.mul(-outer_sign);

            // Alternate between glowing sponsor billboards (NEO-KYOTO, VORTEX, CHRONOS, AETHER-1) and curved Turn Chevrons!
            let is_chevron = a.bank.abs() > 0.24 && (i / 6) % 2 == 0;
            let (h_bot, h_top, u0, u1, v_top, v_bot, scroll_flag, tint) = if is_chevron {
                let row_cyan = (i / 12) % 2 == 0;
                if row_cyan {
                    (3.2, 8.0, 0.0, 1.0, 0.755, 0.995, 1.0, [0.95, 1.0, 1.0, 1.0])
                } else {
                    (3.2, 8.0, 0.0, 1.0, 0.505, 0.745, 1.0, [1.0, 0.95, 1.0, 1.0])
                }
            } else {
                match (i / 6) % 4 {
                    0 => (3.8, 12.6, 0.004, 0.496, 0.004, 0.246, 0.0, [1.0, 1.0, 1.0, 1.0]), // NEO-KYOTO
                    1 => (3.8, 12.6, 0.504, 0.996, 0.004, 0.246, 0.0, [1.0, 1.0, 1.0, 1.0]), // VORTEX ENERGY
                    2 => (3.8, 12.6, 0.004, 0.496, 0.254, 0.496, 0.0, [1.0, 1.0, 1.0, 1.0]), // CHRONOS TECN
                    _ => (3.8, 12.6, 0.504, 0.996, 0.254, 0.496, 0.0, [1.0, 1.0, 1.0, 1.0]), // AETHER-1
                }
            };

            let q0 = p0.add(a.up.mul(h_bot));
            let q1 = p0.add(a.up.mul(h_top));
            let q2 = p1.add(b_end.up.mul(h_top));
            let q3 = p1.add(b_end.up.mul(h_bot));

            // Support steel struts holding the billboard above the rail
            for &(base_p, up_v) in &[(p0, a.up), (p1, b_end.up)] {
                let s_a = base_p.add(up_v.mul(1.8));
                let s_b = base_p.add(up_v.mul(h_bot + 0.4));
                let w = a.forward.mul(0.45);
                push_quad(&mut rails, s_a.sub(w), s_b.sub(w), s_b.add(w), s_a.add(w), face_n, [0.10, 0.14, 0.20, 1.0], [0.8, 0.0]);
            }

            // Front face of billboard (correct left-to-right orientation when viewed from approaching racer!)
            let (ua, ub, uc, ud) = if outer_sign > 0.0 {
                ([u1, v_bot], [u1, v_top], [u0, v_top], [u0, v_bot])
            } else {
                ([u0, v_bot], [u0, v_top], [u1, v_top], [u1, v_bot])
            };
            push_quad_uv(&mut billboards, q0, q1, q2, q3, face_n, ua, ub, uc, ud, tint, [scroll_flag, 1.15]);

            // Mirror-reflected glowing tint of the billboard on the wet track!
            let refl_tint = if is_chevron {
                if (i / 12) % 2 == 0 { [0.22, 0.96, 0.96, 0.52] } else { [0.96, 0.28, 0.88, 0.52] }
            } else {
                [0.45, 0.72, 1.0, 0.36]
            };
            reflect_quad(&mut refl, q0, q1, q2, q3, face_n, a.pos, a.up, refl_tint, [0.1, 1.4]);
        }

        // 6. Monumental Overhead Neon Arch Gantries every 18 segments
        if i % 18 == 0 {
            let arch_h = 18.5;
            let arch_col = if (i / 18) % 2 == 0 { ac_a } else { ac_b };
            let dark_beam = [0.08, 0.12, 0.18, 1.0];
            let fwd_t = a.forward.mul(1.6);

            let l_base = al.sub(a.right.mul(0.8));
            let r_base = ar.add(a.right.mul(0.8));
            let l_top = l_base.add(a.up.mul(arch_h));
            let r_top = r_base.add(a.up.mul(arch_h));

            push_quad(&mut rails, l_base, l_top, l_top.add(fwd_t), l_base.add(fwd_t), a.right, dark_beam, [0.7, 0.0]);
            push_quad(&mut rails, r_base, r_base.add(fwd_t), r_top.add(fwd_t), r_top, a.right.mul(-1.0), dark_beam, [0.7, 0.0]);
            // Glowing overhead crossbar light strip
            push_quad(&mut rails, l_top, r_top, r_top.add(fwd_t), l_top.add(fwd_t), a.up.mul(-1.0), arch_col, [0.15, 2.4]);
            push_quad(&mut rails, l_top, l_top.add(a.up.mul(1.3)), r_top.add(a.up.mul(1.3)), r_top, a.forward.mul(-1.0), arch_col, [0.15, 2.4]);
            // Mirror reflection of overhead neon arch on wet track!
            reflect_quad(&mut refl, l_top, r_top, r_top.add(fwd_t), l_top.add(fwd_t), a.up.mul(-1.0), a.pos, a.up, [arch_col[0], arch_col[1], arch_col[2], 0.55], [0.1, 1.6]);

            // Elevated support viaduct pier plunging into the megacity mist below
            let p_top = a.pos.add(a.up.mul(-2.8));
            let p_bot = Vec3::new(p_top.x, -260.0, p_top.z);
            let pw = a.right.mul(3.0);
            push_quad(&mut rails, p_bot.sub(pw), p_top.sub(pw), p_top.add(pw), p_bot.add(pw), a.forward.mul(-1.0), [0.05, 0.08, 0.13, 1.0], [0.6, 0.0]);
            push_quad(&mut rails, p_bot.add(pw), p_top.add(pw), p_top.sub(pw), p_bot.sub(pw), a.forward, [0.05, 0.08, 0.13, 1.0], [0.6, 0.0]);
        }
    }

    // 7. Dense Cyberpunk Megacity Skyline Below & Around the Track using Blender-Sculpted Skyscraper Spires!
    let mut rng = 0xA37E_2026_5511u64;
    let mut next_f = || -> f32 {
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((rng >> 40) as f32) / 16777216.0
    };

    let mut placed: Vec<(f32, f32)> = Vec::with_capacity(200);
    for t_idx in 0..220 {
        let sm = samples[(t_idx * 5) % n];
        let side = if t_idx % 2 == 0 { 1.0 } else { -1.0 };
        let dist_out = 52.0 + next_f() * 290.0;
        let along_jit = (next_f() - 0.5) * 70.0;
        let cx = sm.pos.x + sm.right.x * side * dist_out + sm.forward.x * along_jit;
        let cz = sm.pos.z + sm.right.z * side * dist_out + sm.forward.z * along_jit;

        // Guarantee clearance from every track segment
        let mut clear = true;
        for s_chk in samples.iter().step_by(4) {
            let dx = cx - s_chk.pos.x;
            let dz = cz - s_chk.pos.z;
            if dx * dx + dz * dz < 44.0 * 44.0 {
                clear = false;
                break;
            }
        }
        if !clear {
            continue;
        }
        // Avoid overlapping neighboring towers
        if placed.iter().any(|&(px, pz)| (px - cx) * (px - cx) + (pz - cz) * (pz - cz) < 34.0 * 34.0) {
            continue;
        }
        placed.push((cx, cz));

        let bw = 18.0 + next_f() * 24.0;
        let bd = 18.0 + next_f() * 24.0;
        let base_y = -270.0;
        let top_y = sm.pos.y - 10.0 + (next_f() - 0.42) * 145.0;
        let height = (top_y - base_y).max(110.0);

        let yaw = next_f() * TAU;
        let (sy, cy) = yaw.sin_cos();
        let tr = Vec3::new(cy, 0.0, sy);
        let tf = Vec3::new(-sy, 0.0, cy);
        let glow_col = if t_idx % 3 == 0 {
            ac_a
        } else if t_idx % 3 == 1 {
            ac_b
        } else {
            [1.0, 0.72, 0.25, 1.0]
        };

        emit_mesh_world(
            skyscraper,
            &mut city,
            Vec3::new(cx, base_y, cz),
            tr,
            Vec3::UP,
            tf,
            Vec3::new(bw, height, bd),
            [0.85, 0.90, 1.0, 1.0],
            glow_col,
            None,
        );
    }

    // 8. Layered Volumetric Swirling Megacity Mist Banks weaving between the skyscrapers below the track!
    let mut mist = Vec::with_capacity(256);
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_z, mut max_z) = (f32::MAX, f32::MIN);
    for sm in samples {
        min_x = min_x.min(sm.pos.x);
        max_x = max_x.max(sm.pos.x);
        min_z = min_z.min(sm.pos.z);
        max_z = max_z.max(sm.pos.z);
    }
    let cx = (min_x + max_x) * 0.5;
    let cz = (min_z + max_z) * 0.5;
    let half_span = 1150.0;
    // 6 stacked volumetric mist cloud decks wrapping right around the skyscraper mid-sections & below the track!
    let layers: [(f32, [f32; 4]); 6] = [
        ( 34.0, [0.85, 1.00, 1.15, 0.32]),
        ( 18.0, [0.80, 0.98, 1.15, 0.44]),
        ( -4.0, [0.74, 0.94, 1.12, 0.54]),
        (-30.0, [0.66, 0.88, 1.08, 0.62]),
        (-66.0, [0.58, 0.80, 1.04, 0.72]),
        (-110.0,[0.50, 0.74, 0.98, 0.82]),
    ];
    for (ly, col) in layers {
        let a = Vec3::new(cx - half_span, ly, cz - half_span);
        let b = Vec3::new(cx - half_span, ly, cz + half_span);
        let c = Vec3::new(cx + half_span, ly, cz + half_span);
        let d = Vec3::new(cx + half_span, ly, cz - half_span);
        push_quad(&mut mist, a, b, c, d, Vec3::UP, col, [0.0, 0.0]);
    }

    StaticWorld {
        track_verts: track,
        rail_verts: rails,
        city_verts: city,
        billboard_verts: billboards,
        refl_verts: refl,
        mist_verts: mist,
    }
}

/// Build a unit sphere mesh for the equirectangular twilight storm-cloud sky dome.
pub fn build_sky_dome_mesh() -> Vec<Vertex3D> {
    let slices = 32usize;
    let stacks = 16usize;
    let mut out = Vec::with_capacity(slices * stacks * 6);
    for st in 0..stacks {
        let phi0 = -std::f32::consts::FRAC_PI_2 + (st as f32 / stacks as f32) * std::f32::consts::PI;
        let phi1 = -std::f32::consts::FRAC_PI_2 + ((st + 1) as f32 / stacks as f32) * std::f32::consts::PI;
        for sl in 0..slices {
            let th0 = (sl as f32 / slices as f32) * TAU;
            let th1 = ((sl + 1) as f32 / slices as f32) * TAU;
            let pt = |phi: f32, th: f32| {
                Vec3::new(phi.cos() * th.cos(), phi.sin(), phi.cos() * th.sin())
            };
            let p00 = pt(phi0, th0);
            let p10 = pt(phi0, th1);
            let p11 = pt(phi1, th1);
            let p01 = pt(phi1, th0);
            push_quad(&mut out, p00, p01, p11, p10, p00.mul(-1.0), [1.0, 1.0, 1.0, 1.0], [0.0, 1.0]);
        }
    }
    out
}
