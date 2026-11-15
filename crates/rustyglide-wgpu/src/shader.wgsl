// The Voodoo pixel pipeline: TMU combine, FBI color and alpha combine,
// chroma key, alpha test, fog and depth, driven by `ShaderState`.
//
// Colors and depth are interpolated linearly in screen space and s/w, t/w are
// divided by 1/w per pixel, exactly as the hardware iterates them.

struct State {
    combine: vec4<u32>,
    flags: vec4<u32>,
    constant: vec4<f32>,
    fog_color: vec4<f32>,
    tex_scale: vec4<f32>,
    bias: vec4<f32>,
    detail0: vec4<f32>,
    detail1: vec4<f32>,
    screen: vec4<f32>,
    fog_table: array<vec4<u32>, 4>,
}

const FLAG_TMU0: u32 = 1u;
const FLAG_TMU1: u32 = 2u;
const FLAG_CHROMA: u32 = 4u;
const FLAG_ALPHA_CONTROLS_ITRGB: u32 = 8u;
const FLAG_LFB_PIPELINE: u32 = 32u;
const FLAG_LFB_DEPTH: u32 = 64u;

@group(0) @binding(0) var<uniform> st: State;
@group(1) @binding(0) var tex0: texture_2d<f32>;
@group(1) @binding(1) var samp0: sampler;
@group(1) @binding(2) var tex1: texture_2d<f32>;
@group(1) @binding(3) var samp1: sampler;

struct VertexIn {
    @location(0) pos: vec2<f32>,
    @location(1) ooz: f32,
    @location(2) oow: f32,
    @location(3) color: vec4<f32>,
    @location(4) tex0: vec3<f32>,
    @location(5) tex1: vec3<f32>,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(linear) color: vec4<f32>,
    @location(1) @interpolate(linear) ooz: f32,
    @location(2) @interpolate(linear) oow: f32,
    @location(3) @interpolate(linear) tex0: vec3<f32>,
    @location(4) @interpolate(linear) tex1: vec3<f32>,
}

struct FragmentOut {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@vertex
fn vs_main(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = vec2<f32>(v.pos.x / st.screen.x * 2.0 - 1.0, 1.0 - v.pos.y / st.screen.y * 2.0);
    out.position = vec4<f32>(ndc, 0.5, 1.0);
    out.color = v.color / 255.0;
    out.ooz = v.ooz;
    out.oow = v.oow;
    out.tex0 = v.tex0;
    out.tex1 = v.tex1;
    return out;
}

// ---- Combine units ----------------------------------------------------------

fn zero_other(f: u32) -> bool {
    return f == 0u || f == 1u || f == 2u || f == 9u || f == 16u;
}

fn sub_local(f: u32) -> bool {
    return f == 6u || f == 7u || f == 8u || f == 9u || f == 16u;
}

fn add_local(f: u32) -> bool {
    return f == 1u || f == 4u || f == 7u || f == 9u;
}

fn add_local_alpha(f: u32) -> bool {
    return f == 2u || f == 5u || f == 8u || f == 16u;
}

fn combine_rgb(f: u32, factor: vec3<f32>, local: vec3<f32>, other: vec3<f32>, local_alpha: f32, invert: bool) -> vec3<f32> {
    var c = select(other, vec3<f32>(0.0), zero_other(f));
    if sub_local(f) {
        c = c - local;
    }
    c = c * factor;
    if add_local(f) {
        c = c + local;
    } else if add_local_alpha(f) {
        c = c + vec3<f32>(local_alpha);
    }
    c = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(c, vec3<f32>(1.0) - c, invert);
}

fn combine_a(f: u32, factor: f32, local: f32, other: f32, invert: bool) -> f32 {
    var c = select(other, 0.0, zero_other(f));
    if sub_local(f) {
        c = c - local;
    }
    c = c * factor;
    if add_local(f) || add_local_alpha(f) {
        c = c + local;
    }
    c = clamp(c, 0.0, 1.0);
    return select(c, 1.0 - c, invert);
}

fn invert_factor(code: u32, f: f32) -> f32 {
    return select(f, 1.0 - f, (code & 8u) != 0u);
}

fn invert_factor3(code: u32, f: vec3<f32>) -> vec3<f32> {
    return select(f, vec3<f32>(1.0) - f, (code & 8u) != 0u);
}

// A TMU: `local` is its texel, `other` the upstream TMU's output.
fn tmu_combine(word: u32, local: vec4<f32>, other: vec4<f32>, detail: f32, lod_fraction: f32) -> vec4<f32> {
    let rgb_fn = word & 31u;
    let rgb_factor = (word >> 5u) & 15u;
    let a_fn = (word >> 9u) & 31u;
    let a_factor = (word >> 14u) & 15u;
    var fr: vec3<f32>;
    switch rgb_factor & 7u {
        case 1u: { fr = local.rgb; }
        case 2u: { fr = vec3<f32>(other.a); }
        case 3u: { fr = vec3<f32>(local.a); }
        case 4u: { fr = vec3<f32>(detail); }
        case 5u: { fr = vec3<f32>(lod_fraction); }
        default: { fr = vec3<f32>(0.0); }
    }
    var fa: f32;
    switch a_factor & 7u {
        case 1u, 3u: { fa = local.a; }
        case 2u: { fa = other.a; }
        case 4u: { fa = detail; }
        case 5u: { fa = lod_fraction; }
        default: { fa = 0.0; }
    }
    let rgb = combine_rgb(rgb_fn, invert_factor3(rgb_factor, fr), local.rgb, other.rgb, local.a, ((word >> 18u) & 1u) != 0u);
    let a = combine_a(a_fn, invert_factor(a_factor, fa), local.a, other.a, ((word >> 19u) & 1u) != 0u);
    return vec4<f32>(rgb, a);
}

// ---- Texturing ---------------------------------------------------------------

fn tex_coord(t: vec3<f32>, scale: vec2<f32>) -> vec2<f32> {
    let w = select(t.z, 1.0, abs(t.z) < 1e-30);
    return t.xy / w * scale;
}

fn lod_of(uv: vec2<f32>, size: vec2<f32>) -> f32 {
    let dx = dpdx(uv * size);
    let dy = dpdy(uv * size);
    let rho = max(dot(dx, dx), dot(dy, dy));
    return 0.5 * log2(max(rho, 1e-12));
}

fn detail_factor(d: vec4<f32>, lod: f32) -> f32 {
    // Detail bias, scale and max as grTexDetailControl sets them; LOD in quarter steps.
    let f = (d.x - lod * 4.0) * exp2(d.y) / 255.0;
    return clamp(f, 0.0, d.z);
}

// ---- Fog and depth -----------------------------------------------------------

// The hardware's 16-bit floating W: 0 nearest, 65535 farthest.
fn wfloat(oow: f32) -> f32 {
    if oow >= 1.0 {
        return 0.0;
    }
    if oow < 1.0 / 65536.0 {
        return 65535.0;
    }
    let bits = bitcast<u32>(oow);
    let e = 126u - ((bits >> 23u) & 255u);
    let inverted = (~(bits >> 11u)) & 4095u;
    return f32(min(((e << 12u) | inverted) + 1u, 65535u));
}

fn fog_entry(i: u32) -> f32 {
    let word = st.fog_table[i / 16u][(i / 4u) % 4u];
    return f32((word >> ((i % 4u) * 8u)) & 255u);
}

fn apply_fog(color: vec3<f32>, iter_alpha: f32, ooz: f32, oow: f32) -> vec3<f32> {
    let mode = st.flags.w & 1023u;
    let base = mode & 255u;
    if base == 0u {
        return color;
    }
    var blend: f32;
    if base == 1u {
        blend = iter_alpha * 255.0;
    } else if base == 2u {
        let w = u32(wfloat(oow));
        let index = w >> 10u;
        let frac = f32((w >> 2u) & 255u);
        let here = fog_entry(index);
        let next = fog_entry(min(index + 1u, 63u));
        blend = here + (next - here) * frac / 256.0;
    } else {
        blend = clamp(ooz, 0.0, 65535.0) / 256.0;
    }
    var f = select(st.fog_color.rgb, vec3<f32>(0.0), (mode & 512u) != 0u);
    let mult = (mode & 256u) != 0u;
    if !mult {
        f = f - color;
    }
    f = f * (blend / 255.0);
    return clamp(select(color + f, f, mult), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn depth_value(ooz: f32, oow: f32) -> f32 {
    let mode = st.flags.w >> 12u;
    var d: f32;
    switch mode {
        case 1u: { d = clamp(ooz, 0.0, 65535.0) + st.bias.z; }
        case 2u: { d = wfloat(oow) + st.bias.z; }
        case 3u, 4u: { d = st.bias.z; }
        default: { d = 0.0; }
    }
    return clamp(d, 0.0, 65535.0) / 65535.0;
}

fn alpha_test(a: f32) -> bool {
    let a8 = u32(round(clamp(a, 0.0, 1.0) * 255.0));
    let r = (st.flags.y >> 8u) & 255u;
    switch st.flags.y & 7u {
        case 0u: { return false; }
        case 1u: { return a8 < r; }
        case 2u: { return a8 == r; }
        case 3u: { return a8 <= r; }
        case 4u: { return a8 > r; }
        case 5u: { return a8 != r; }
        case 6u: { return a8 >= r; }
        default: { return true; }
    }
}

fn chroma_match(c: vec3<f32>) -> bool {
    let q = vec3<u32>(round(clamp(c, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0));
    let k = vec3<u32>((st.flags.z >> 16u) & 255u, (st.flags.z >> 8u) & 255u, st.flags.z & 255u);
    return all(q == k);
}

// ---- The pixel pipeline ------------------------------------------------------

@fragment
fn fs_main(in: VertexOut) -> FragmentOut {
    let uv0 = tex_coord(in.tex0, st.tex_scale.xy);
    let uv1 = tex_coord(in.tex1, st.tex_scale.zw);
    let size0 = vec2<f32>(textureDimensions(tex0));
    let size1 = vec2<f32>(textureDimensions(tex1));
    let lod0 = lod_of(uv0, size0) + st.bias.x;
    let lod1 = lod_of(uv1, size1) + st.bias.y;
    let s0 = textureSampleBias(tex0, samp0, uv0, st.bias.x);
    let s1 = textureSampleBias(tex1, samp1, uv1, st.bias.y);

    let flags = st.flags.x;
    var t1 = vec4<f32>(0.0);
    if (flags & FLAG_TMU1) != 0u {
        t1 = tmu_combine(st.combine.w, s1, vec4<f32>(0.0), detail_factor(st.detail1, lod1), fract(lod1));
    }
    let local0 = select(vec4<f32>(0.0), s0, (flags & FLAG_TMU0) != 0u);
    let texel = tmu_combine(st.combine.z, local0, t1, detail_factor(st.detail0, lod0), fract(lod0));

    let iter = clamp(in.color, vec4<f32>(0.0), vec4<f32>(1.0));
    let cc = st.combine.x;
    let ac = st.combine.y;

    // Alpha combine inputs.
    var a_local: f32;
    switch (ac >> 9u) & 3u {
        case 0u: { a_local = iter.a; }
        case 1u: { a_local = st.constant.a; }
        default: { a_local = floor(clamp(in.ooz, 0.0, 65535.0) / 256.0) / 255.0; }
    }
    var a_other: f32;
    switch (ac >> 11u) & 3u {
        case 0u: { a_other = iter.a; }
        case 1u: { a_other = texel.a; }
        default: { a_other = st.constant.a; }
    }

    // Color combine inputs.
    var c_local = select(iter.rgb, st.constant.rgb, ((cc >> 9u) & 3u) == 1u);
    if (flags & FLAG_ALPHA_CONTROLS_ITRGB) != 0u {
        c_local = select(iter.rgb, st.constant.rgb, texel.a >= 0.5);
    }
    var c_other: vec3<f32>;
    switch (cc >> 11u) & 3u {
        case 0u: { c_other = iter.rgb; }
        case 1u: { c_other = texel.rgb; }
        default: { c_other = st.constant.rgb; }
    }

    if (flags & FLAG_CHROMA) != 0u && chroma_match(c_other) {
        discard;
    }

    let cf_code = (cc >> 5u) & 15u;
    var cf: vec3<f32>;
    switch cf_code & 7u {
        case 1u: { cf = c_local; }
        case 2u: { cf = vec3<f32>(a_other); }
        case 3u: { cf = vec3<f32>(a_local); }
        case 4u: { cf = vec3<f32>(texel.a); }
        case 5u: { cf = texel.rgb; }
        default: { cf = vec3<f32>(0.0); }
    }
    let rgb = combine_rgb(cc & 31u, invert_factor3(cf_code, cf), c_local, c_other, a_local, ((cc >> 13u) & 1u) != 0u);

    let af_code = (ac >> 5u) & 15u;
    var af: f32;
    switch af_code & 7u {
        case 1u, 3u: { af = a_local; }
        case 2u: { af = a_other; }
        case 4u: { af = texel.a; }
        default: { af = 0.0; }
    }
    let alpha = combine_a(ac & 31u, invert_factor(af_code, af), a_local, a_other, ((ac >> 13u) & 1u) != 0u);

    if !alpha_test(alpha) {
        discard;
    }

    var out: FragmentOut;
    out.color = vec4<f32>(apply_fog(rgb, iter.a, in.ooz, in.oow), alpha);
    out.depth = depth_value(in.ooz, in.oow);
    return out;
}

// LFB writes: tex0 holds the pixels, tex1 the written mask (r) and depth (g, b).
// tex0 coordinates carry the pixel position inside the written rectangle.
@fragment
fn fs_lfb(in: VertexOut) -> FragmentOut {
    let p = vec2<i32>(floor(in.tex0.xy));
    let color = textureLoad(tex0, p, 0);
    let extra = textureLoad(tex1, p, 0);
    if extra.r < 0.5 {
        discard;
    }
    let flags = st.flags.x;
    if (flags & FLAG_LFB_PIPELINE) != 0u {
        if (flags & FLAG_CHROMA) != 0u && chroma_match(color.rgb) {
            discard;
        }
        if !alpha_test(color.a) {
            discard;
        }
    }
    var out: FragmentOut;
    out.color = color;
    let lfb_depth = (round(extra.g * 255.0) * 256.0 + round(extra.b * 255.0)) / 65535.0;
    out.depth = select(st.bias.w, lfb_depth, (flags & FLAG_LFB_DEPTH) != 0u);
    return out;
}

// Clears: color from `constant`, depth from `bias.w`.
@fragment
fn fs_clear(in: VertexOut) -> FragmentOut {
    var out: FragmentOut;
    out.color = st.constant;
    out.depth = st.bias.w;
    return out;
}
