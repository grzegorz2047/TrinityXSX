// SPDX-License-Identifier: MIT
//! Immediate-mode GPU neon vector & 5x7 HUD text batcher (1920x1080 -> NDC).

use std::f32::consts::TAU;

use crate::scarlett::{Vertex, SCREEN_H, SCREEN_W};

pub struct Batch {
    pub verts: Vec<Vertex>,
}

impl Batch {
    pub fn new() -> Self {
        Self { verts: Vec::with_capacity(32_768) }
    }

    pub fn clear(&mut self) {
        self.verts.clear();
    }

    #[inline]
    fn to_ndc(x: f32, y: f32) -> [f32; 2] {
        [(x / (SCREEN_W as f32)) * 2.0 - 1.0, 1.0 - (y / (SCREEN_H as f32)) * 2.0]
    }

    #[inline]
    pub fn tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], col: [f32; 4]) {
        self.verts.push(Vertex { pos: Self::to_ndc(a[0], a[1]), col });
        self.verts.push(Vertex { pos: Self::to_ndc(b[0], b[1]), col });
        self.verts.push(Vertex { pos: Self::to_ndc(c[0], c[1]), col });
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, col: [f32; 4]) {
        let a = [x, y];
        let b = [x + w, y];
        let c = [x + w, y + h];
        let d = [x, y + h];
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    pub fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, col: [f32; 4]) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();
        if len < 0.001 {
            return;
        }
        let nx = -dy / len * (thickness * 0.5);
        let ny = dx / len * (thickness * 0.5);
        let a = [x1 + nx, y1 + ny];
        let b = [x2 + nx, y2 + ny];
        let c = [x2 - nx, y2 - ny];
        let d = [x1 - nx, y1 - ny];
        self.tri(a, b, c, col);
        self.tri(a, c, d, col);
    }

    /// Glowing neon segment: soft wide outer halo + crisp bright core.
    pub fn neon_line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, thickness: f32, col: [f32; 4]) {
        let glow = [col[0], col[1], col[2], col[3] * 0.24];
        self.line(x1, y1, x2, y2, thickness * 2.8, glow);
        self.line(x1, y1, x2, y2, thickness, col);
    }

    pub fn ring(&mut self, cx: f32, cy: f32, radius: f32, thickness: f32, segs: usize, col: [f32; 4]) {
        let mut px = cx + radius;
        let mut py = cy;
        for i in 1..=segs {
            let ang = (i as f32 / segs as f32) * TAU;
            let nx = cx + ang.cos() * radius;
            let ny = cy + ang.sin() * radius;
            self.neon_line(px, py, nx, ny, thickness, col);
            px = nx;
            py = ny;
        }
    }

    pub fn polygon(&mut self, cx: f32, cy: f32, radius: f32, sides: usize, rot: f32, thickness: f32, col: [f32; 4]) {
        let mut px = cx + rot.cos() * radius;
        let mut py = cy + rot.sin() * radius;
        for i in 1..=sides {
            let ang = rot + (i as f32 / sides as f32) * TAU;
            let nx = cx + ang.cos() * radius;
            let ny = cy + ang.sin() * radius;
            self.neon_line(px, py, nx, ny, thickness, col);
            px = nx;
            py = ny;
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, radius: f32, segs: usize, col: [f32; 4]) {
        let center = [cx, cy];
        let mut px = cx + radius;
        let mut py = cy;
        for i in 1..=segs {
            let ang = (i as f32 / segs as f32) * TAU;
            let nx = cx + ang.cos() * radius;
            let ny = cy + ang.sin() * radius;
            self.tri(center, [px, py], [nx, ny], col);
            px = nx;
            py = ny;
        }
    }

    pub fn text(&mut self, x: f32, y: f32, s: &str, scale: f32, col: [f32; 4]) {
        let mut cx = x;
        for ch in s.bytes() {
            let rows = glyph(ch.to_ascii_uppercase());
            for (r, &bits) in rows.iter().enumerate() {
                for c in 0..5 {
                    if (bits & (1 << (4 - c))) != 0 {
                        self.rect(cx + c as f32 * scale, y + r as f32 * scale, scale, scale, col);
                    }
                }
            }
            cx += 6.0 * scale;
        }
    }

    pub fn text_width(s: &str, scale: f32) -> f32 {
        if s.is_empty() {
            0.0
        } else {
            (s.len() as f32 * 6.0 - 1.0) * scale
        }
    }
}

fn glyph(ch: u8) -> [u8; 7] {
    match ch {
        b'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        b'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        b'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        b'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        b'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        b'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        b'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        b'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'J' => [0x07, 0x02, 0x02, 0x02, 0x12, 0x12, 0x0C],
        b'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        b'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        b'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        b'N' => [0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11],
        b'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        b'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        b'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        b'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        b'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        b'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        b'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        b'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        b'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        b'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        b'0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        b'1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        b'2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        b'3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        b'4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        b'5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        b'6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        b'7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        b'8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        b'9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        b':' => [0x00, 0x04, 0x04, 0x00, 0x04, 0x04, 0x00],
        b'-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        b'+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        b'/' => [0x01, 0x02, 0x02, 0x04, 0x08, 0x08, 0x10],
        b'.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        b'!' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x00, 0x04],
        b'|' => [0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        b'(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        b')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        _ => [0; 7],
    }
}
