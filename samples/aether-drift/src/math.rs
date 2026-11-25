// SPDX-License-Identifier: MIT
//! Compact 3D vector & matrix math for AETHER DRIFT (left-handed D3D12 clip space).

#[derive(Clone, Copy, Default, Debug)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const UP: Self = Self::new(0.0, 1.0, 0.0);

    #[inline]
    pub fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    #[inline]
    pub fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    #[inline]
    pub fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
    #[inline]
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    #[inline]
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    #[inline]
    pub fn norm(self) -> Self {
        let l = self.len();
        if l > 1e-6 { self.mul(1.0 / l) } else { Self::new(0.0, 1.0, 0.0) }
    }
    #[inline]
    pub fn lerp(self, o: Self, t: f32) -> Self {
        self.add(o.sub(self).mul(t))
    }
    #[inline]
    pub fn to_arr(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

/// Column-major 4x4 matrix stored for HLSL `mul(viewProj, float4(pos, 1))`.
/// (HLSL `cbuffer` defaults to column-major, so `m[col][row]` matches standard math.)
#[derive(Clone, Copy)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub fn look_at_lh(eye: Vec3, target: Vec3, up: Vec3) -> Self {
        let z = target.sub(eye).norm();
        let x = up.cross(z).norm();
        let y = z.cross(x);
        // Row-vector view matrix transposed into column-major for HLSL mul(M, v)
        let r0 = [x.x, x.y, x.z, -x.dot(eye)];
        let r1 = [y.x, y.y, y.z, -y.dot(eye)];
        let r2 = [z.x, z.y, z.z, -z.dot(eye)];
        let r3 = [0.0, 0.0, 0.0, 1.0];
        Self::from_rows(r0, r1, r2, r3)
    }

    pub fn perspective_lh(fov_y_rad: f32, aspect: f32, zn: f32, zf: f32) -> Self {
        let ys = 1.0 / (fov_y_rad * 0.5).tan();
        let xs = ys / aspect;
        let q = zf / (zf - zn);
        Self::from_rows(
            [xs, 0.0, 0.0, 0.0],
            [0.0, ys, 0.0, 0.0],
            [0.0, 0.0, q, -zn * q],
            [0.0, 0.0, 1.0, 0.0],
        )
    }

    /// Build HLSL column-major storage from 4 logical rows.
    pub fn from_rows(r0: [f32; 4], r1: [f32; 4], r2: [f32; 4], r3: [f32; 4]) -> Self {
        Self([
            [r0[0], r1[0], r2[0], r3[0]],
            [r0[1], r1[1], r2[1], r3[1]],
            [r0[2], r1[2], r2[2], r3[2]],
            [r0[3], r1[3], r2[3], r3[3]],
        ])
    }

    pub fn mul(self, rhs: Self) -> Self {
        let mut out = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                out[col][row] = self.0[0][row] * rhs.0[col][0]
                    + self.0[1][row] * rhs.0[col][1]
                    + self.0[2][row] * rhs.0[col][2]
                    + self.0[3][row] * rhs.0[col][3];
            }
        }
        Self(out)
    }
}

/// Closed Catmull-Rom spline interpolation on 3D points.
pub fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    let c = |a: f32, b: f32, c: f32, d: f32| -> f32 {
        0.5 * ((2.0 * b)
            + (-a + c) * t
            + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
            + (-a + 3.0 * b - 3.0 * c + d) * t3)
    };
    Vec3::new(
        c(p0.x, p1.x, p2.x, p3.x),
        c(p0.y, p1.y, p2.y, p3.y),
        c(p0.z, p1.z, p2.z, p3.z),
    )
}
