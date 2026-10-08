// The plant's ink-wash shading: a flat colour lit in three bands, like a
// cartoon cel, plus whatever glows. Light comes from a fixed corner so every
// figure reads the same way wherever it stands; the outlines are a second
// mesh drawn by `app.rs`, not this shader.
#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> base: vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> emissive: vec4<f32>;

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
    let rgb = base.rgb * band + emissive.rgb;
    return vec4<f32>(rgb, base.a);
}
