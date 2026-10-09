// The plant's ink-wash shading: a flat colour lit in three bands, like a
// cartoon cel, plus whatever glows. Light comes from a fixed corner so every
// figure reads the same way wherever it stands; the outlines are a second
// mesh drawn by `app.rs`, not this shader.
//
// A surface may also wear a pattern painted in world space — bricks, pavers,
// planks, a roof's seams, moving water — so a wall is a wall without a
// texture file: `pattern.x` names it (see `scene::Pattern::id`), `pattern.y`
// is its tile size in world units.
#import bevy_pbr::forward_io::VertexOutput
#import bevy_pbr::mesh_view_bindings::globals

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> base: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> emissive: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> pattern: vec4<f32>;

// A stable pseudo-random value in [0, 1) per tile.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.547);
}

// The surface's own two coordinates: the plane its normal faces most.
fn face_uv(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    let a = abs(n);
    if (a.y >= a.x && a.y >= a.z) {
        return p.xz;
    }
    if (a.x >= a.z) {
        return vec2<f32>(p.z, p.y);
    }
    return p.xy;
}

// How much a joint darkens: 1 inside a tile, `dark` on its edge.
fn joint(f: vec2<f32>, width: vec2<f32>, dark: f32) -> f32 {
    let edge = step(f, width) + step(vec2<f32>(1.0) - width, f);
    return select(1.0, dark, max(edge.x, edge.y) > 0.0);
}

// The pattern's shade, multiplied into the colour.
fn shade(p: vec3<f32>, n: vec3<f32>) -> f32 {
    let kind = i32(pattern.x + 0.5);
    let size = max(pattern.y, 0.01);
    let uv = face_uv(p, n);
    switch kind {
        // Bricks: courses under half as tall as a brick is long, every other
        // one shifted by half a brick, mortar a shade darker.
        case 1: {
            var g = vec2<f32>(uv.x / size, uv.y / (size * 0.45));
            let row = floor(g.y);
            g.x += 0.5 * (row - 2.0 * floor(row / 2.0));
            let cell = floor(g);
            return joint(fract(g), vec2<f32>(0.04, 0.09), 0.72) * (0.93 + 0.12 * hash(cell));
        }
        // Pavers: square slabs, thin grout, each a touch lighter or darker.
        case 2: {
            let g = uv / size;
            return joint(fract(g), vec2<f32>(0.03, 0.03), 0.84) * (0.95 + 0.08 * hash(floor(g)));
        }
        // Planks: boards along the surface's first axis.
        case 3: {
            let g = vec2<f32>(uv.x / (size * 6.0), uv.y / size);
            return joint(fract(g), vec2<f32>(0.0, 0.08), 0.7) * (0.92 + 0.12 * hash(vec2<f32>(0.0, floor(g.y))));
        }
        // Seams: a roof of metal sheets, one seam every tile along x.
        case 4: {
            return select(1.0, 0.78, fract(p.x / size) < 0.07);
        }
        // Water: slow crossing waves, their crests caught as pale streaks.
        case 5: {
            let t = globals.time;
            let w = sin(p.x * 2.1 + t * 1.1 + sin(p.z * 1.3 + t * 0.6) * 1.4)
                + 0.6 * sin(p.z * 3.4 - t * 0.9 + p.x * 0.7);
            if (w > 1.25) {
                return 1.32;
            }
            if (w < -1.1) {
                return 0.88;
            }
            return 1.0;
        }
        default: {
            return 1.0;
        }
    }
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let l = normalize(vec3<f32>(0.55, 0.85, 0.3));
    let d = dot(n, l);
    // Three bands, none of them dark: the palette is bright and should
    // stay so in the shade.
    var band = 0.68;
    if (d > 0.5) {
        band = 1.0;
    } else if (d > 0.05) {
        band = 0.86;
    }
    let rgb = base.rgb * band * shade(in.world_position.xyz, n) + emissive.rgb;
    return vec4<f32>(rgb, base.a);
}
