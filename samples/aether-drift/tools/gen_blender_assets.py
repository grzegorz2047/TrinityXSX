# SPDX-License-Identifier: MIT
"""
Blender 5.2 LTS headless procedural 3D asset generator for AETHER DRIFT v2.0.
Run via:
  "C:\Program Files\Blender Foundation\Blender 5.2\blender.exe" --background --python gen_blender_assets.py

Generates smooth-shaded, beveled, UV-unwrapped binary meshes in ../assets/meshes/:
  - hero_craft.bin       : Sculpted twin-pod anti-gravity interceptor (matches concept art)
  - rival_craft.bin      : Sculpted angular wedge rival craft
  - skyscraper_spire.bin : Multi-tier Art-Deco/Cyberpunk megacity spire with antenna
"""

import math
import os
import struct
import bpy
import bmesh
from mathutils import Matrix, Vector

OUT_DIR = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "assets", "meshes"))
os.makedirs(OUT_DIR, exist_ok=True)


def clear_scene():
    bpy.ops.wm.read_factory_settings(use_empty=True)


def make_obj(name, bm, smooth=True, subsurf_levels=0, bevel_width=0.0):
    mesh = bpy.data.meshes.new(name)
    bm.normal_update()
    bm.to_mesh(mesh)
    bm.free()

    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)

    if bevel_width > 0.0:
        mod = obj.modifiers.new("Bevel", "BEVEL")
        mod.width = bevel_width
        mod.segments = 2
        mod.limit_method = "ANGLE"
        mod.angle_limit = math.radians(28.0)

    if subsurf_levels > 0:
        mod = obj.modifiers.new("Subsurf", "SUBSURF")
        mod.levels = subsurf_levels
        mod.render_levels = subsurf_levels

    if smooth:
        for p in mesh.polygons:
            p.use_smooth = True

    return obj


def export_parts(parts, filename):
    """
    parts: list of (bpy.types.Object, rgba_tuple, mat_code)
    Binary format:
      u32 vertex_count
      vertex_count * (pos[3f], normal[3f], uv[2f], col[4f], mat[2f]) = 56 bytes/vertex
    where:
      mat[0] = roughness/metallic encoded: wetness/metallic [0..1]
      mat[1] = emissive intensity [0..3], or negative tag for team-color/glow substitution:
               col.a encodes part role:
                 0.0 = primary body (tinted with racer body_col in game)
                 0.25 = secondary metallic / dark gunmetal / tinted canopy (fixed color)
                 0.50 = team accent / neon trim (tinted with racer glow_col, emissive)
                 0.75 = plasma core / brake bar (fixed bright emissive)
                 1.00 = textured architectural facade (fixed color + box UV)
    """
    depsgraph = bpy.context.evaluated_depsgraph_get()
    verts_blob = bytearray()
    total_verts = 0

    for obj, rgba, metal, emissive in parts:
        eval_obj = obj.evaluated_get(depsgraph)
        me = eval_obj.to_mesh()
        me.calc_loop_triangles()

        uv_layer = me.uv_layers.active.data if me.uv_layers.active else None
        mat_world = obj.matrix_world
        mat_norm = mat_world.to_3x3().inverted_safe().transposed()

        for tri in me.loop_triangles:
            # Blender is Z-up, Y-forward; AETHER DRIFT is Y-up, +Z-forward, +X-right!
            # Convert Blender (bx, by, bz) -> Engine (bx, bz, by) and flip winding to keep outward normals.
            idxs = [tri.loops[0], tri.loops[2], tri.loops[1]]
            for li in idxs:
                loop = me.loops[li]
                v = me.vertices[loop.vertex_index]
                wp = mat_world @ v.co
                # Use corner/polygon normal cleanly
                ln = loop.normal if hasattr(loop, "normal") and loop.normal.length > 0.01 else v.normal
                wn = (mat_norm @ ln).normalized()

                ex, ey, ez = wp.x, wp.z, wp.y
                nx, ny, nz = wn.x, wn.z, wn.y

                if uv_layer is not None:
                    uv = uv_layer[li].uv
                    u, vv = float(uv.x), float(uv.y)
                else:
                    # Clean planar/triplanar UV from engine position
                    u = (ex * 0.25 + ez * 0.15)
                    vv = (ey * 0.25 + ez * 0.15)

                verts_blob.extend(
                    struct.pack(
                        "<14f",
                        ex, ey, ez,
                        nx, ny, nz,
                        u, vv,
                        rgba[0], rgba[1], rgba[2], rgba[3],
                        metal, emissive,
                    )
                )
                total_verts += 1

        eval_obj.to_mesh_clear()

    out_path = os.path.join(OUT_DIR, filename)
    with open(out_path, "wb") as f:
        f.write(struct.pack("<I", total_verts))
        f.write(verts_blob)
    print(f"[BLENDER] Exported {filename}: {total_verts} verts ({total_verts // 3} tris), {len(verts_blob) + 4} bytes")


def add_lofted_profiles(bm, profiles, close_front=True, close_back=True):
    """Create a sleek lofted solid from a list of 3D rings (each ring has same vertex count)."""
    rings = []
    for ring in profiles:
        rings.append([bm.verts.new(Vector(p)) for p in ring])
    m = len(rings[0])
    for r in range(len(rings) - 1):
        a = rings[r]
        b = rings[r + 1]
        for i in range(m):
            j = (i + 1) % m
            bm.faces.new((a[i], a[j], b[j], b[i]))
    if close_front:
        bm.faces.new(rings[0][::-1])
    if close_back:
        bm.faces.new(rings[-1])


def build_hero_craft():
    """
    Sculpted Twin-Pod Interceptor modeled directly after the AETHER DRIFT concept art:
      - Sleek tapered central fuselage with beveled shoulders
      - Curved dark panoramic glass canopy
      - Massive twin chamfered side-nacelle wing pods with glowing cyan hex rim frames
      - Dual deep-recessed circular jet exhaust housings + blazing plasma cores
      - Warm amber central rear diffuser/brake vent bar
    In Blender coords: +Y = forward, +Z = up, +X = right.
    """
    clear_scene()
    parts = []

    # 1. Central Aerodynamic Fuselage (lofted cross-sections from nose +4.6 to tail -2.8)
    bm = bmesh.new()
    # Each ring: 8 points around cross-section (top-mid, top-right, mid-right, bot-right, bot-mid, bot-left, mid-left, top-left)
    sections = [
        # y_fwd, w_top, w_mid, w_bot, z_top, z_mid, z_bot
        ( 4.65, 0.14, 0.24, 0.16,  0.04, -0.02, -0.10),
        ( 3.60, 0.42, 0.72, 0.50,  0.22,  0.02, -0.18),
        ( 2.10, 0.72, 1.18, 0.84,  0.48,  0.12, -0.26),
        ( 0.50, 0.88, 1.32, 0.96,  0.76,  0.22, -0.32),
        (-1.10, 0.92, 1.24, 0.88,  0.72,  0.22, -0.30),
        (-2.55, 0.78, 1.02, 0.72,  0.50,  0.14, -0.24),
        (-3.05, 0.60, 0.78, 0.56,  0.34,  0.10, -0.18),
    ]
    profiles = []
    for y, wt, wm, wb, zt, zm, zb in sections:
        profiles.append([
            ( 0.0, y, zt),
            ( wt,  y, zt * 0.82),
            ( wm,  y, zm),
            ( wb,  y, zb * 0.9),
            ( 0.0, y, zb),
            (-wb,  y, zb * 0.9),
            (-wm,  y, zm),
            (-wt,  y, zt * 0.82),
        ])
    add_lofted_profiles(bm, profiles)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj_hull = make_obj("HeroFuselage", bm, smooth=True, subsurf_levels=1, bevel_width=0.05)
    parts.append((obj_hull, (1.0, 1.0, 1.0, 0.0), 0.92, 0.05))

    # 2. Curved Tinted Glass Canopy (flush teardrop bubble)
    bm = bmesh.new()
    canopy_sec = [
        ( 2.35, 0.12, 0.18, 0.34, 0.24),
        ( 1.50, 0.44, 0.62, 0.78, 0.36),
        ( 0.35, 0.54, 0.74, 1.02, 0.44),
        (-0.85, 0.46, 0.66, 0.92, 0.44),
        (-1.85, 0.24, 0.40, 0.66, 0.36),
    ]
    c_profs = []
    for y, wt, wb, zt, zb in canopy_sec:
        c_profs.append([
            ( 0.0, y, zt),
            ( wt,  y, (zt + zb) * 0.55),
            ( wb,  y, zb),
            ( 0.0, y, zb - 0.05),
            (-wb,  y, zb),
            (-wt,  y, (zt + zb) * 0.55),
        ])
    add_lofted_profiles(bm, c_profs)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj_canopy = make_obj("HeroCanopy", bm, smooth=True, subsurf_levels=1)
    parts.append((obj_canopy, (0.04, 0.09, 0.15, 0.25), 0.99, 0.06))

    # 3. Twin Swept Armored Side-Nacelle Thruster Pods (signature shape from concept art!)
    for side in (-1.0, 1.0):
        bm = bmesh.new()
        # Cross-section of wide chamfered pod from front intake (+1.55) to rear shroud (-3.25)
        pod_sec = [
            # y, x_in, x_out, z_top, z_bot
            ( 1.65, 1.08, 1.78,  0.22, -0.18),
            ( 0.80, 1.15, 2.55,  0.48, -0.26),
            (-0.45, 1.12, 3.05,  0.64, -0.32),
            (-2.05, 1.05, 3.12,  0.62, -0.32),
            (-3.20, 0.98, 2.86,  0.48, -0.26),
        ]
        p_profs = []
        for y, xi, xo, zt, zb in pod_sec:
            xm = (xi + xo) * 0.52
            zm = (zt + zb) * 0.5
            pts = [
                (side * xi, y, zt * 0.85),
                (side * xm, y, zt),
                (side * xo, y, zm + 0.12),
                (side * xo, y, zm - 0.12),
                (side * xm, y, zb),
                (side * xi, y, zb * 0.85),
            ]
            if side < 0:
                pts.reverse()
            p_profs.append(pts)
        add_lofted_profiles(bm, p_profs)
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        obj_pod = make_obj(f"HeroPod_{'R' if side > 0 else 'L'}", bm, smooth=True, bevel_width=0.07)
        parts.append((obj_pod, (0.94, 0.97, 1.0, 0.0), 0.90, 0.04))

        # 3b. Glowing Trapezoidal/Hex Cyan Rear Rim Trim Frame on Pod (concept art signature!)
        bm = bmesh.new()
        y0, y1 = -3.14, -3.28
        outer_ring = [
            (side * 1.16, 0.42),
            (side * 2.68, 0.30),
            (side * 2.68, -0.16),
            (side * 1.16, -0.20),
        ]
        inner_ring = [
            (side * 1.32, 0.30),
            (side * 2.48, 0.20),
            (side * 2.48, -0.08),
            (side * 1.32, -0.10),
        ]
        for i in range(4):
            j = (i + 1) % 4
            ox0, oz0 = outer_ring[i]
            ox1, oz1 = outer_ring[j]
            ix0, iz0 = inner_ring[i]
            ix1, iz1 = inner_ring[j]
            v0 = bm.verts.new((ox0, y1, oz0))
            v1 = bm.verts.new((ox1, y1, oz1))
            v2 = bm.verts.new((ix1, y1, iz1))
            v3 = bm.verts.new((ix0, y1, iz0))
            bm.faces.new((v0, v1, v2, v3) if side > 0 else (v0, v3, v2, v1))
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        obj_trim = make_obj(f"HeroTrim_{'R' if side > 0 else 'L'}", bm, smooth=False)
        parts.append((obj_trim, (1.0, 1.0, 1.0, 0.50), 0.2, 2.4))

        # 3c. Sleek Glowing Accent Stripe along Top Shoulder of Pod
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        for v in bm.verts:
            v.co.x = side * (1.72 + v.co.x * 0.14)
            v.co.y = -0.75 + v.co.y * 3.1
            v.co.z = 0.62 + v.co.z * 0.06
        bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
        obj_stripe = make_obj(f"HeroStripe_{'R' if side > 0 else 'L'}", bm, smooth=False)
        parts.append((obj_stripe, (1.0, 1.0, 1.0, 0.50), 0.15, 2.2))

        # 4. Circular Dark Gunmetal Jet Turbine Housing + Blazing Plasma Core
        bm = bmesh.new()
        bmesh.ops.create_cone(
            bm, cap_ends=True, cap_tris=False, segments=16,
            radius1=0.46, radius2=0.38, depth=0.65,
            matrix=Matrix.Translation((side * 1.48, -3.10, 0.10)) @ Matrix.Rotation(math.radians(90.0), 4, "X"),
        )
        obj_noz = make_obj(f"HeroNozzle_{'R' if side > 0 else 'L'}", bm, smooth=True)
        parts.append((obj_noz, (0.10, 0.13, 0.18, 0.25), 0.85, 0.05))

        bm = bmesh.new()
        bmesh.ops.create_cone(
            bm, cap_ends=True, cap_tris=False, segments=16,
            radius1=0.34, radius2=0.30, depth=0.14,
            matrix=Matrix.Translation((side * 1.48, -3.40, 0.10)) @ Matrix.Rotation(math.radians(90.0), 4, "X"),
        )
        obj_core = make_obj(f"HeroCore_{'R' if side > 0 else 'L'}", bm, smooth=True)
        parts.append((obj_core, (0.88, 0.99, 1.0, 0.75), 0.1, 3.0))

        # 5. Front Intake Louver / Dark Vent Grille
        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        for v in bm.verts:
            v.co.x = side * (1.42 + v.co.x * 0.56)
            v.co.y = 1.56 + v.co.y * 0.22
            v.co.z = 0.02 + v.co.z * 0.28
        obj_vent = make_obj(f"HeroVent_{'R' if side > 0 else 'L'}", bm, smooth=False)
        parts.append((obj_vent, (0.04, 0.06, 0.09, 0.25), 0.5, 0.0))

    # 6. Rear Central Warm Amber Diffuser / Brake Bar (signature detail in concept art!)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    for v in bm.verts:
        v.co.x = v.co.x * 1.35
        v.co.y = -3.08 + v.co.y * 0.14
        v.co.z = 0.08 + v.co.z * 0.14
    obj_bar = make_obj("HeroRearBar", bm, smooth=False)
    parts.append((obj_bar, (1.0, 0.68, 0.22, 0.75), 0.1, 2.6))

    export_parts(parts, "hero_craft.bin")


def build_rival_craft():
    """
    Sculpted sleek angular wide-delta Rival Craft (matching the orange & magenta rivals in concept art).
    """
    clear_scene()
    parts = []

    # 1. Wide Sculpted Wedge Hull
    bm = bmesh.new()
    sections = [
        ( 4.40, 0.35, 0.55, 0.38,  0.04, -0.02, -0.10),
        ( 2.80, 0.82, 1.45, 1.05,  0.26,  0.04, -0.20),
        ( 0.80, 1.15, 2.25, 1.75,  0.54,  0.14, -0.26),
        (-1.40, 1.25, 2.68, 2.15,  0.60,  0.18, -0.28),
        (-3.10, 1.08, 2.42, 1.90,  0.46,  0.14, -0.22),
    ]
    profiles = []
    for y, wt, wm, wb, zt, zm, zb in sections:
        profiles.append([
            ( 0.0, y, zt),
            ( wt,  y, zt * 0.85),
            ( wm,  y, zm),
            ( wb,  y, zb * 0.9),
            ( 0.0, y, zb),
            (-wb,  y, zb * 0.9),
            (-wm,  y, zm),
            (-wt,  y, zt * 0.85),
        ])
    add_lofted_profiles(bm, profiles)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj_hull = make_obj("RivalHull", bm, smooth=True, subsurf_levels=1, bevel_width=0.05)
    parts.append((obj_hull, (1.0, 1.0, 1.0, 0.0), 0.90, 0.05))

    # 2. Dark Tinted Low-Profile Canopy
    bm = bmesh.new()
    c_profs = []
    for y, wt, wb, zt, zb in [
        ( 2.00, 0.14, 0.24, 0.36, 0.24),
        ( 0.60, 0.52, 0.72, 0.88, 0.42),
        (-1.10, 0.48, 0.68, 0.82, 0.42),
        (-2.20, 0.26, 0.42, 0.58, 0.36),
    ]:
        c_profs.append([
            ( 0.0, y, zt), ( wt, y, (zt + zb) * 0.55), ( wb, y, zb),
            ( 0.0, y, zb - 0.04), (-wb, y, zb), (-wt, y, (zt + zb) * 0.55),
        ])
    add_lofted_profiles(bm, c_profs)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj_canopy = make_obj("RivalCanopy", bm, smooth=True, subsurf_levels=1)
    parts.append((obj_canopy, (0.05, 0.09, 0.14, 0.25), 0.98, 0.05))

    # 3. Twin Rear Exhaust Housings + Blazing Glow Cores + Side Trim
    for side in (-1.0, 1.0):
        bm = bmesh.new()
        bmesh.ops.create_cone(
            bm, cap_ends=True, cap_tris=False, segments=14,
            radius1=0.40, radius2=0.34, depth=0.55,
            matrix=Matrix.Translation((side * 1.15, -3.12, 0.12)) @ Matrix.Rotation(math.radians(90.0), 4, "X"),
        )
        parts.append((make_obj(f"RivalNoz_{side}", bm, smooth=True), (0.10, 0.12, 0.16, 0.25), 0.85, 0.0))

        bm = bmesh.new()
        bmesh.ops.create_cone(
            bm, cap_ends=True, cap_tris=False, segments=14,
            radius1=0.30, radius2=0.26, depth=0.14,
            matrix=Matrix.Translation((side * 1.15, -3.38, 0.12)) @ Matrix.Rotation(math.radians(90.0), 4, "X"),
        )
        parts.append((make_obj(f"RivalCore_{side}", bm, smooth=True), (1.0, 1.0, 1.0, 0.50), 0.1, 3.0))

        bm = bmesh.new()
        bmesh.ops.create_cube(bm, size=1.0)
        for v in bm.verts:
            v.co.x = side * (2.30 + v.co.x * 0.16)
            v.co.y = -1.65 + v.co.y * 2.2
            v.co.z = 0.26 + v.co.z * 0.08
        parts.append((make_obj(f"RivalWingLED_{side}", bm, smooth=False), (1.0, 1.0, 1.0, 0.50), 0.15, 2.3))

    export_parts(parts, "rival_craft.bin")


def build_skyscraper_spire():
    """
    Multi-tier gothic/Art-Deco Cyberpunk Skyscraper Spire (normalized unit width=1, height=1, base at z=0)
    with clean UV mapping for the illuminated window grid facade texture.
    """
    clear_scene()
    parts = []

    def add_box_uv(name, w, d, z0, z1, col, metal, emissive):
        bm = bmesh.new()
        uv_layer = bm.loops.layers.uv.new("UVMap")
        bmesh.ops.create_cube(bm, size=1.0)
        for v in bm.verts:
            v.co.x *= w
            v.co.y *= d
            v.co.z = z0 + (v.co.z + 0.5) * (z1 - z0)
        bm.normal_update()
        for f in bm.faces:
            n = f.normal
            for loop in f.loops:
                co = loop.vert.co
                if abs(n.z) > 0.7:
                    u, vv = co.x * 0.5 + 0.5, co.y * 0.5 + 0.5
                elif abs(n.x) > 0.7:
                    u, vv = (co.y / max(d, 0.01) + 0.5) * 1.5, co.z * 4.0
                else:
                    u, vv = (co.x / max(w, 0.01) + 0.5) * 1.5, co.z * 4.0
                loop[uv_layer].uv = (u, vv)
        obj = make_obj(name, bm, smooth=False)
        parts.append((obj, col, metal, emissive))

    # Tier 1: Main wide base shaft (0.0 .. 0.62)
    add_box_uv("TowerBase", 1.0, 1.0, 0.0, 0.62, (0.95, 0.97, 1.0, 1.0), 0.65, 0.0)
    # Tier 2: Mid setback (0.62 .. 0.82)
    add_box_uv("TowerMid", 0.76, 0.76, 0.62, 0.82, (0.92, 0.95, 1.0, 1.0), 0.70, 0.0)
    # Tier 3: Crown tower (0.82 .. 0.92)
    add_box_uv("TowerCrown", 0.50, 0.50, 0.82, 0.92, (0.88, 0.92, 1.0, 1.0), 0.75, 0.0)
    # Subtle glowing crown accent trim collar (sleek, won't blow out bloom!)
    add_box_uv("TowerCrownGlow", 0.52, 0.52, 0.910, 0.920, (0.85, 0.92, 1.0, 0.50), 0.1, 0.85)
    # Sleek tapered gothic/Art-Deco spire & needle antenna (0.92 .. 1.06)
    bm = bmesh.new()
    bmesh.ops.create_cone(
        bm, cap_ends=True, cap_tris=False, segments=8,
        radius1=0.09, radius2=0.008, depth=0.14,
        matrix=Matrix.Translation((0.0, 0.0, 0.99)),
    )
    parts.append((make_obj("TowerSpire", bm, smooth=True), (0.14, 0.18, 0.28, 0.25), 0.8, 0.05))
    # Subtle aviation beacon at spire tip
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=1, radius=0.014, matrix=Matrix.Translation((0.0, 0.0, 1.062)))
    parts.append((make_obj("TowerBeacon", bm, smooth=True), (1.0, 0.35, 0.55, 0.75), 0.1, 1.6))

    export_parts(parts, "skyscraper_spire.bin")


if __name__ == "__main__":
    build_hero_craft()
    build_rival_craft()
    build_skyscraper_spire()
    print("[BLENDER] All 3D meshes generated successfully!")
