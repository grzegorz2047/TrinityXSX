// SPDX-License-Identifier: MIT
//! Embedded Blender 5.2 sculpted binary meshes (.bin) and PBR textures (.tex) for AETHER DRIFT v2.0.

use crate::math::Vec3;
use crate::scarlett::Vertex3D;

static HERO_CRAFT_BIN: &[u8] = include_bytes!("../assets/meshes/hero_craft.bin");
static RIVAL_CRAFT_BIN: &[u8] = include_bytes!("../assets/meshes/rival_craft.bin");
static SKYSCRAPER_BIN: &[u8] = include_bytes!("../assets/meshes/skyscraper_spire.bin");

static TEX_TRACK_ALBEDO: &[u8] = include_bytes!("../assets/textures/track_albedo.tex");
static TEX_TRACK_PUDDLE: &[u8] = include_bytes!("../assets/textures/track_puddle.tex");
static TEX_BILLBOARD_ATLAS: &[u8] = include_bytes!("../assets/textures/billboard_atlas.tex");
static TEX_CITY_FACADE: &[u8] = include_bytes!("../assets/textures/city_facade.tex");
static TEX_SKY_TWILIGHT: &[u8] = include_bytes!("../assets/textures/sky_twilight.tex");

pub struct RawTexture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct MeshAsset {
    pub verts: Vec<Vertex3D>,
}

pub struct GameAssets {
    pub hero_craft: MeshAsset,
    pub rival_craft: MeshAsset,
    pub skyscraper: MeshAsset,
}

impl GameAssets {
    pub fn load_meshes() -> Self {
        Self {
            hero_craft: parse_mesh(HERO_CRAFT_BIN),
            rival_craft: parse_mesh(RIVAL_CRAFT_BIN),
            skyscraper: parse_mesh(SKYSCRAPER_BIN),
        }
    }

    pub fn decode_textures() -> [RawTexture; 5] {
        [
            decode_tex(TEX_TRACK_ALBEDO),
            decode_tex(TEX_TRACK_PUDDLE),
            decode_tex(TEX_BILLBOARD_ATLAS),
            decode_tex(TEX_CITY_FACADE),
            decode_tex(TEX_SKY_TWILIGHT),
        ]
    }
}

fn decode_tex(bytes: &[u8]) -> RawTexture {
    assert!(bytes.len() >= 8);
    let width = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    let height = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let rgba = miniz_oxide::inflate::decompress_to_vec_zlib(&bytes[8..])
        .expect("embedded .tex zlib decompress failed");
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    RawTexture { width, height, rgba }
}

fn parse_mesh(bytes: &[u8]) -> MeshAsset {
    assert!(bytes.len() >= 4);
    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let mut verts = Vec::with_capacity(count);
    let mut off = 4usize;
    let f = |b: &[u8], i: usize| f32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    for _ in 0..count {
        verts.push(Vertex3D {
            pos: [f(bytes, off), f(bytes, off + 4), f(bytes, off + 8)],
            normal: [f(bytes, off + 12), f(bytes, off + 16), f(bytes, off + 20)],
            uv: [f(bytes, off + 24), f(bytes, off + 28)],
            col: [
                f(bytes, off + 32),
                f(bytes, off + 36),
                f(bytes, off + 40),
                f(bytes, off + 44),
            ],
            mat: [f(bytes, off + 48), f(bytes, off + 52)],
        });
        off += 56;
    }
    MeshAsset { verts }
}

/// Emit an instanced Blender mesh into `out`, transforming local (right, up, fwd) into world space
/// and tinting part roles encoded in `v.col[3]` (0.0 = body, 0.25 = metal/canopy, 0.5 = team glow,
/// 0.75 = plasma core, 1.0 = skyscraper facade).
#[inline]
pub fn emit_mesh_world(
    mesh: &MeshAsset,
    out: &mut Vec<Vertex3D>,
    origin: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
    scale: Vec3,
    body_col: [f32; 4],
    glow_col: [f32; 4],
    alpha_override: Option<f32>,
) {
    out.reserve(mesh.verts.len());
    for v in &mesh.verts {
        let lx = v.pos[0] * scale.x;
        let ly = v.pos[1] * scale.y;
        let lz = v.pos[2] * scale.z;
        let wp = Vec3::new(
            origin.x + right.x * lx + up.x * ly + fwd.x * lz,
            origin.y + right.y * lx + up.y * ly + fwd.y * lz,
            origin.z + right.z * lx + up.z * ly + fwd.z * lz,
        );
        let wn = Vec3::new(
            right.x * v.normal[0] + up.x * v.normal[1] + fwd.x * v.normal[2],
            right.y * v.normal[0] + up.y * v.normal[1] + fwd.y * v.normal[2],
            right.z * v.normal[0] + up.z * v.normal[1] + fwd.z * v.normal[2],
        )
        .norm();

        let role = v.col[3];
        let (col, mat) = if role < 0.12 {
            (
                [
                    body_col[0] * v.col[0],
                    body_col[1] * v.col[1],
                    body_col[2] * v.col[2],
                    alpha_override.unwrap_or(body_col[3]),
                ],
                v.mat,
            )
        } else if role < 0.38 {
            ([v.col[0], v.col[1], v.col[2], alpha_override.unwrap_or(1.0)], v.mat)
        } else if role < 0.62 {
            (
                [
                    glow_col[0] * v.col[0],
                    glow_col[1] * v.col[1],
                    glow_col[2] * v.col[2],
                    alpha_override.unwrap_or(1.0),
                ],
                v.mat,
            )
        } else if role < 0.88 {
            ([v.col[0], v.col[1], v.col[2], alpha_override.unwrap_or(1.0)], v.mat)
        } else {
            (
                [
                    body_col[0] * v.col[0],
                    body_col[1] * v.col[1],
                    body_col[2] * v.col[2],
                    alpha_override.unwrap_or(1.0),
                ],
                v.mat,
            )
        };

        out.push(Vertex3D {
            pos: wp.to_arr(),
            normal: wn.to_arr(),
            uv: v.uv,
            col,
            mat,
        });
    }
}

/// Emit a planar mirror-reflected copy of `mesh` reflected across `(plane_origin, plane_up)`
/// for true wet-track reflections on banked curves!
#[inline]
pub fn emit_mesh_reflected(
    mesh: &MeshAsset,
    out: &mut Vec<Vertex3D>,
    origin: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
    scale: Vec3,
    plane_origin: Vec3,
    plane_up: Vec3,
    body_col: [f32; 4],
    glow_col: [f32; 4],
) {
    let reflect_pt = |p: Vec3| -> Vec3 {
        let d = p.sub(plane_origin).dot(plane_up);
        p.sub(plane_up.mul(2.0 * d))
    };
    let reflect_dir = |n: Vec3| -> Vec3 {
        let d = n.dot(plane_up);
        n.sub(plane_up.mul(2.0 * d))
    };

    let n_verts = mesh.verts.len();
    out.reserve(n_verts);
    let mut i = 0;
    while i + 2 < n_verts {
        let src = [mesh.verts[i], mesh.verts[i + 2], mesh.verts[i + 1]]; // flip winding for mirror!
        for v in src {
            let lx = v.pos[0] * scale.x;
            let ly = v.pos[1] * scale.y;
            let lz = v.pos[2] * scale.z;
            let wp = Vec3::new(
                origin.x + right.x * lx + up.x * ly + fwd.x * lz,
                origin.y + right.y * lx + up.y * ly + fwd.y * lz,
                origin.z + right.z * lx + up.z * ly + fwd.z * lz,
            );
            let wn = Vec3::new(
                right.x * v.normal[0] + up.x * v.normal[1] + fwd.x * v.normal[2],
                right.y * v.normal[0] + up.y * v.normal[1] + fwd.y * v.normal[2],
                right.z * v.normal[0] + up.z * v.normal[1] + fwd.z * v.normal[2],
            )
            .norm();

            let role = v.col[3];
            let (col, mat) = if role < 0.12 {
                ([body_col[0] * v.col[0], body_col[1] * v.col[1], body_col[2] * v.col[2], 0.85], v.mat)
            } else if role < 0.38 {
                ([v.col[0], v.col[1], v.col[2], 0.65], v.mat)
            } else if role < 0.62 {
                ([glow_col[0] * v.col[0], glow_col[1] * v.col[1], glow_col[2] * v.col[2], 1.0], v.mat)
            } else {
                ([v.col[0], v.col[1], v.col[2], 1.0], v.mat)
            };

            out.push(Vertex3D {
                pos: reflect_pt(wp).to_arr(),
                normal: reflect_dir(wn).to_arr(),
                uv: v.uv,
                col,
                mat,
            });
        }
        i += 3;
    }
}
