// Scales the Glide frame into the window and applies gamma.

struct PresentOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

struct PresentParams {
    // x: 1/gamma, y: 1 when the target encodes sRGB itself.
    params: vec4<f32>,
}

@group(0) @binding(0) var present_tex: texture_2d<f32>;
@group(0) @binding(1) var present_samp: sampler;
@group(0) @binding(2) var<uniform> present: PresentParams;

@vertex
fn vs_present(@builtin(vertex_index) i: u32) -> PresentOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: PresentOut;
    out.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_present(in: PresentOut) -> @location(0) vec4<f32> {
    var c = textureSample(present_tex, present_samp, in.uv).rgb;
    c = pow(c, vec3<f32>(present.params.x));
    if present.params.y > 0.5 {
        // The target re-encodes to sRGB; hand it linear values so the
        // displayed pixels equal the Glide frame buffer.
        c = pow(c, vec3<f32>(2.2));
    }
    return vec4<f32>(c, 1.0);
}
