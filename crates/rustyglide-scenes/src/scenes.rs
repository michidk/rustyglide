//! The scene list. Each scene exercises one area of the API.

use crate::{Scene, rect, v, vt};
use rustyglide::combine::{Combine, TexCombine};
use rustyglide::texture::TexInfo;
use rustyglide::types::*;
use rustyglide::{Blend, Glide, GrVertex};

pub const ALL: &[Scene] = &[
    Scene { name: "clear", draw: clear },
    Scene { name: "gouraud", draw: gouraud },
    Scene { name: "zbuffer", draw: zbuffer },
    Scene { name: "wbuffer", draw: wbuffer },
    Scene { name: "blend", draw: blend },
    Scene { name: "fog_table", draw: fog_table },
    Scene { name: "fog_iterated_alpha", draw: fog_iterated_alpha },
    Scene { name: "chroma_key", draw: chroma_key },
    Scene { name: "cull", draw: cull },
    Scene { name: "clip_window", draw: clip_window },
    Scene { name: "alpha_test", draw: alpha_test },
    Scene { name: "lines_points", draw: lines_points },
    Scene { name: "origin_lower_left", draw: origin_lower_left },
    Scene { name: "texture_decal", draw: texture_decal },
    Scene { name: "texture_modulate", draw: texture_modulate },
    Scene { name: "texture_formats", draw: texture_formats },
    Scene { name: "texture_clamp_wrap", draw: texture_clamp_wrap },
    Scene { name: "mipmap", draw: mipmap },
    Scene { name: "lfb_write", draw: lfb_write },
    Scene { name: "lfb_write_region", draw: lfb_write_region },
    Scene { name: "clip_triangle", draw: clip_triangle },
];

const RED: [f32; 3] = [255.0, 0.0, 0.0];
const GREEN: [f32; 3] = [0.0, 255.0, 0.0];
const BLUE: [f32; 3] = [0.0, 0.0, 255.0];

fn iterated(glide: &mut Glide) {
    glide.gu_color_combine_function(ColorCombineFnc::Itrgb);
    glide.gu_alpha_source(AlphaSource::IteratedAlpha);
}

fn clear(glide: &mut Glide) {
    glide.buffer_clear(0x00336699, 0, 0);
}

fn gouraud(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.draw_triangle(&v(80.0, 10.0, RED), &v(150.0, 110.0, GREEN), &v(10.0, 110.0, BLUE));
}

fn depth_tri(x: [f32; 3], y: [f32; 3], ooz: f32, oow: f32, rgb: [f32; 3]) -> [GrVertex; 3] {
    let p = |i: usize| GrVertex { ooz, oow, ..v(x[i], y[i], rgb) };
    [p(0), p(1), p(2)]
}

fn zbuffer(glide: &mut Glide) {
    iterated(glide);
    glide.depth_buffer_mode(DepthBufferMode::ZBuffer);
    glide.depth_buffer_function(CmpFnc::Greater);
    glide.depth_mask(true);
    glide.buffer_clear(0, 0, 0);
    // Near (large ooz) red first, then a far green triangle that must stay behind.
    let near = depth_tri([20.0, 120.0, 20.0], [10.0, 60.0, 110.0], 40000.0, 1.0, RED);
    let far = depth_tri([140.0, 40.0, 140.0], [10.0, 60.0, 110.0], 10000.0, 1.0, GREEN);
    glide.draw_triangle(&near[0], &near[1], &near[2]);
    glide.draw_triangle(&far[0], &far[1], &far[2]);
}

fn wbuffer(glide: &mut Glide) {
    iterated(glide);
    glide.depth_buffer_mode(DepthBufferMode::WBuffer);
    glide.depth_buffer_function(CmpFnc::Less);
    glide.depth_mask(true);
    glide.buffer_clear(0, 0, 0xffff);
    // A tilted triangle (w 1 to 100) crossing a flat one at w = 10.
    let flat = depth_tri([10.0, 150.0, 10.0], [10.0, 60.0, 110.0], 0.0, 0.1, GREEN);
    glide.draw_triangle(&flat[0], &flat[1], &flat[2]);
    let p = |x, y, w: f32| GrVertex { oow: 1.0 / w, ..v(x, y, RED) };
    glide.draw_triangle(&p(150.0, 10.0, 1.0), &p(150.0, 110.0, 1.0), &p(20.0, 60.0, 100.0));
}

fn blend(glide: &mut Glide) {
    glide.buffer_clear(0x000000ff, 0, 0);
    iterated(glide);
    rect(glide, 10.0, 10.0, 100.0, 100.0, [255.0, 0.0, 0.0, 255.0]);
    glide.alpha_blend_function(Blend {
        rgb_src: BlendFactor::SrcAlpha,
        rgb_dst: BlendFactor::OneMinusSrcAlpha,
        alpha_src: BlendFactor::One,
        alpha_dst: BlendFactor::Zero,
    });
    rect(glide, 50.0, 30.0, 150.0, 110.0, [0.0, 255.0, 0.0, 128.0]);
    glide.alpha_blend_function(Blend {
        rgb_src: BlendFactor::One,
        rgb_dst: BlendFactor::One,
        alpha_src: BlendFactor::One,
        alpha_dst: BlendFactor::Zero,
    });
    rect(glide, 120.0, 5.0, 155.0, 60.0, [128.0, 128.0, 0.0, 255.0]);
}

fn fog_bands(glide: &mut Glide) {
    // Bands at constant w = 1, 2, 4 ... 2048, nearest at the bottom.
    for i in 0..12 {
        let w = (1u32 << i) as f32;
        let p = |x, y| GrVertex { oow: 1.0 / w, ..v(x, y, [255.0, 255.0, 255.0]) };
        let (y1, y0) = (120.0 - i as f32 * 10.0, 110.0 - i as f32 * 10.0);
        glide.draw_triangle(&p(0.0, y0), &p(160.0, y0), &p(160.0, y1));
        glide.draw_triangle(&p(0.0, y0), &p(160.0, y1), &p(0.0, y1));
    }
}

fn fog_table(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    let mut table = [0u8; FOG_TABLE_SIZE];
    rustyglide::fog::generate_exp(&mut table, 0.002);
    glide.fog_table(&table);
    glide.fog_color(0x00ff4000);
    glide.fog_mode(FogMode(FogMode::WITH_TABLE));
    fog_bands(glide);
}

fn fog_iterated_alpha(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    glide.gu_color_combine_function(ColorCombineFnc::Itrgb);
    glide.fog_color(0x000000ff);
    glide.fog_mode(FogMode(FogMode::WITH_ITERATED_ALPHA));
    let p = |x, a| GrVertex { a, ..v(x, 60.0, [255.0, 255.0, 255.0]) };
    let q = |x, y, a| GrVertex { y, ..p(x, a) };
    glide.draw_triangle(&q(10.0, 10.0, 0.0), &q(150.0, 10.0, 255.0), &q(150.0, 110.0, 255.0));
    glide.draw_triangle(&q(10.0, 10.0, 0.0), &q(150.0, 110.0, 255.0), &q(10.0, 110.0, 0.0));
}

fn chroma_key(glide: &mut Glide) {
    glide.buffer_clear(0x00404040, 0, 0);
    // The key is compared with the color combine's "other" input: iterated
    // color with Glide's default combine, the texel with a decal texture.
    glide.color_combine(rustyglide::combine::DEFAULT_COLOR);
    glide.chromakey_value(0x00ff00ff);
    glide.chromakey_mode(ChromakeyMode::Enable);
    rect(glide, 10.0, 10.0, 50.0, 50.0, [255.0, 0.0, 255.0, 255.0]);
    rect(glide, 50.0, 10.0, 90.0, 50.0, [255.0, 0.0, 254.0, 255.0]);
    load_texture(glide, info(Lod::L16, TextureFormat::Rgb565), &checker565(0xf81f, 0xffe0));
    glide.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    textured_quad(glide, 10.0, 60.0, 150.0, 110.0, 256.0);
}

fn cull(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.cull_mode(CullMode::Negative);
    // Left: positive area (drawn); right: negative area (culled).
    glide.draw_triangle(&v(10.0, 10.0, RED), &v(70.0, 10.0, RED), &v(10.0, 100.0, RED));
    glide.draw_triangle(&v(90.0, 10.0, GREEN), &v(90.0, 100.0, GREEN), &v(150.0, 10.0, GREEN));
}

fn clip_window(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.clip_window(40, 30, 120, 90);
    glide.buffer_clear(0x00202080, 0, 0);
    glide.draw_triangle(&v(0.0, 0.0, RED), &v(160.0, 0.0, GREEN), &v(80.0, 120.0, BLUE));
}

fn alpha_test(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.alpha_test_function(CmpFnc::GEqual);
    glide.alpha_test_reference(128);
    // Alpha ramps 0..255 left to right; the left half fails the test.
    let p = |x, y, a| GrVertex { a, ..v(x, y, [255.0, 255.0, 255.0]) };
    glide.draw_triangle(&p(0.0, 20.0, 0.0), &p(160.0, 20.0, 255.0), &p(160.0, 100.0, 255.0));
    glide.draw_triangle(&p(0.0, 20.0, 0.0), &p(160.0, 100.0, 255.0), &p(0.0, 100.0, 0.0));
}

fn lines_points(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    for i in 0..10 {
        let x = 10.0 + i as f32 * 14.0;
        glide.draw_line(&v(x, 10.0, RED), &v(x + 10.0, 60.0, GREEN));
        glide.draw_point(&v(x, 80.0, [255.0, 255.0, 255.0]));
    }
    glide.draw_line(&v(5.0, 110.0, BLUE), &v(155.0, 100.0, BLUE));
}

fn origin_lower_left(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.sst_origin(Origin::LowerLeft);
    // Red sits at the bottom of the screen with a lower-left origin.
    glide.draw_triangle(&v(10.0, 10.0, RED), &v(150.0, 10.0, RED), &v(80.0, 50.0, GREEN));
}

/// Downloads a texture into TMU0 at address 0 and makes it the source.
pub fn load_texture(glide: &mut Glide, info: TexInfo, data: &[u8]) {
    glide.tex_download_mipmap(Tmu::Tmu0, 0, level_mask::BOTH, info, data);
    glide.tex_source(Tmu::Tmu0, 0, level_mask::BOTH, info);
    glide.gu_tex_combine_function(Tmu::Tmu0, TextureCombineFnc::Decal);
}

/// A 16x16 565 checkerboard of `a` and `b`.
fn checker565(a: u16, b: u16) -> Vec<u8> {
    (0..256).flat_map(|i| if ((i % 16) / 4 + (i / 16) / 4) % 2 == 0 { a } else { b }.to_le_bytes()).collect()
}

fn info(lod: Lod, format: TextureFormat) -> TexInfo {
    TexInfo { small_lod: lod, large_lod: lod, aspect: Aspect::A1x1, format }
}

fn textured_quad(glide: &mut Glide, x0: f32, y0: f32, x1: f32, y1: f32, s1: f32) {
    let a = vt(x0, y0, 0.0, 0.0, 1.0);
    let b = vt(x1, y0, s1, 0.0, 1.0);
    let c = vt(x1, y1, s1, s1, 1.0);
    let d = vt(x0, y1, 0.0, s1, 1.0);
    glide.draw_triangle(&a, &b, &c);
    glide.draw_triangle(&a, &c, &d);
}

fn texture_decal(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    load_texture(glide, info(Lod::L16, TextureFormat::Rgb565), &checker565(0xf800, 0x07ff));
    glide.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    // A rectangle in perspective: the far edge (top) is at w = 4, a quarter as wide.
    let a = vt(10.0, 110.0, 0.0, 256.0, 1.0);
    let b = vt(150.0, 110.0, 256.0, 256.0, 1.0);
    let c = vt(97.5, 10.0, 256.0, 0.0, 4.0);
    let d = vt(62.5, 10.0, 0.0, 0.0, 4.0);
    glide.draw_triangle(&a, &b, &c);
    glide.draw_triangle(&a, &c, &d);
}

fn texture_modulate(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    load_texture(glide, info(Lod::L16, TextureFormat::Rgb565), &checker565(0xffff, 0x8410));
    glide.gu_color_combine_function(ColorCombineFnc::TextureTimesItrgb);
    glide.tex_filter_mode(Tmu::Tmu0, FilterMode::Bilinear, FilterMode::Bilinear);
    let mut a = vt(10.0, 10.0, 0.0, 0.0, 1.0);
    let mut b = vt(150.0, 10.0, 256.0, 0.0, 1.0);
    let mut c = vt(150.0, 110.0, 256.0, 256.0, 1.0);
    let mut d = vt(10.0, 110.0, 0.0, 256.0, 1.0);
    (a.r, a.g, a.b) = (255.0, 0.0, 0.0);
    (b.r, b.g, b.b) = (0.0, 255.0, 0.0);
    (c.r, c.g, c.b) = (0.0, 0.0, 255.0);
    (d.r, d.g, d.b) = (255.0, 255.0, 0.0);
    glide.draw_triangle(&a, &b, &c);
    glide.draw_triangle(&a, &c, &d);
}

fn texture_formats(glide: &mut Glide) {
    glide.buffer_clear(0x00202020, 0, 0);
    glide.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    glide.gu_alpha_source(AlphaSource::TextureAlpha);
    glide.alpha_blend_function(Blend {
        rgb_src: BlendFactor::SrcAlpha,
        rgb_dst: BlendFactor::OneMinusSrcAlpha,
        alpha_src: BlendFactor::One,
        alpha_dst: BlendFactor::Zero,
    });
    let mut palette = [0u32; 256];
    for (i, p) in palette.iter_mut().enumerate() {
        *p = (i as u32) << 16 | ((255 - i) as u32) << 8 | ((i * 7) & 0xff) as u32;
    }
    glide.tex_download_palette(Tmu::Tmu0, &palette, 0);
    let mut ncc = rustyglide::texture::NccData::default();
    for (i, y) in ncc.y.iter_mut().enumerate() {
        *y = (i * 17) as u8;
    }
    ncc.i = [[-40, 10, 30], [0, 0, 0], [40, -10, -30], [80, -20, -60]];
    ncc.q = [[0, 0, 0], [30, -30, 0], [-30, 30, 0], [0, 40, -40]];
    glide.tex_download_ncc(Tmu::Tmu0, NccTable::Ncc0, &ncc);
    let formats = [
        TextureFormat::Rgb332,
        TextureFormat::Yiq422,
        TextureFormat::Alpha8,
        TextureFormat::Intensity8,
        TextureFormat::AlphaIntensity44,
        TextureFormat::P8,
        TextureFormat::Argb8332,
        TextureFormat::Ayiq8422,
        TextureFormat::Rgb565,
        TextureFormat::Argb1555,
        TextureFormat::Argb4444,
        TextureFormat::AlphaIntensity88,
        TextureFormat::Ap88,
    ];
    for (n, &format) in formats.iter().enumerate() {
        // A 16x16 gradient: every byte value appears.
        let data: Vec<u8> = if format.bytes_per_texel() == 1 {
            (0..256u32).map(|i| i as u8).collect()
        } else {
            (0..256u32).flat_map(|i| ((i * 257) as u16 ^ 0x5a5a).to_le_bytes()).collect()
        };
        let address = n as u32 * 1024;
        let info = info(Lod::L16, format);
        glide.tex_download_mipmap(Tmu::Tmu0, address, level_mask::BOTH, info, &data);
        glide.tex_source(Tmu::Tmu0, address, level_mask::BOTH, info);
        glide.gu_tex_combine_function(Tmu::Tmu0, TextureCombineFnc::Decal);
        let (x, y) = ((n % 5) as f32 * 32.0, (n / 5) as f32 * 40.0);
        textured_quad(glide, x + 1.0, y + 1.0, x + 31.0, y + 31.0, 256.0);
    }
}

fn texture_clamp_wrap(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    load_texture(glide, info(Lod::L16, TextureFormat::Rgb565), &checker565(0xffe0, 0x001f));
    glide.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    glide.tex_clamp_mode(Tmu::Tmu0, ClampMode::Wrap, ClampMode::Wrap);
    textured_quad(glide, 5.0, 5.0, 75.0, 115.0, 768.0);
    glide.tex_clamp_mode(Tmu::Tmu0, ClampMode::Clamp, ClampMode::Clamp);
    textured_quad(glide, 85.0, 5.0, 155.0, 115.0, 768.0);
}

fn mipmap(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    // A 64x64 chain whose levels have distinct colors.
    let colors = [0xf800u16, 0x07e0, 0x001f, 0xffe0, 0xf81f, 0x07ff, 0xffff];
    let mut data = Vec::new();
    for (level, &c) in colors.iter().enumerate() {
        let side = 64usize >> level;
        data.extend((0..side * side).flat_map(|_| c.to_le_bytes()));
    }
    let info = TexInfo { small_lod: Lod::L1, large_lod: Lod::L64, aspect: Aspect::A1x1, format: TextureFormat::Rgb565 };
    load_texture(glide, info, &data);
    glide.gu_color_combine_function(ColorCombineFnc::DecalTexture);
    glide.tex_mipmap_mode(Tmu::Tmu0, MipMapMode::Nearest, false);
    glide.tex_clamp_mode(Tmu::Tmu0, ClampMode::Wrap, ClampMode::Wrap);
    // A floor receding into the distance (w 1 to 8), repeating the texture.
    let a = vt(0.0, 120.0, 0.0, 0.0, 1.0);
    let b = vt(160.0, 120.0, 2048.0, 0.0, 1.0);
    let c = vt(90.0, 10.0, 2048.0, 2048.0, 8.0);
    let d = vt(70.0, 10.0, 0.0, 2048.0, 8.0);
    glide.draw_triangle(&a, &b, &c);
    glide.draw_triangle(&a, &c, &d);
}

fn lfb_write(glide: &mut Glide) {
    glide.buffer_clear(0x00000040, 0, 0);
    let Some(lock) = glide.lfb_lock(lock::WRITE_ONLY, Buffer::Back, LfbWriteMode::W565, Origin::LowerLeft, false) else {
        return;
    };
    for y in 0..40usize {
        for x in 0..120usize {
            let color: u16 = ((x as u16 / 4) << 11) | ((y as u16) << 5) | 0x10;
            // SAFETY: the lock buffer holds stride * height bytes and (x, y) is inside it.
            unsafe {
                let p = lock.data.add(y * lock.stride as usize + (x + 20) * 2) as *mut u16;
                p.write_unaligned(color);
            }
        }
    }
    glide.lfb_unlock(lock::WRITE_ONLY, Buffer::Back);
}

fn lfb_write_region(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    let pixels: Vec<u8> = (0..64 * 32u32)
        .flat_map(|i| {
            let (x, y) = (i % 64, i / 64);
            (0x8000u16 | ((x as u16 / 2) << 10) | ((y as u16) << 5) | 0x1f).to_le_bytes()
        })
        .collect();
    glide.lfb_write_region(Buffer::Back, 48, 44, LfbWriteMode::W1555 as u32, 64, 32, 128, &pixels);
}

fn clip_triangle(glide: &mut Glide) {
    glide.buffer_clear(0, 0, 0);
    iterated(glide);
    glide.clip_window(20, 20, 140, 100);
    glide.draw_triangle_clipped(&v(-40.0, -20.0, RED), &v(200.0, 30.0, GREEN), &v(60.0, 160.0, BLUE));
}

/// Applies combine settings used by the combine sweep test.
pub fn set_combine(glide: &mut Glide, color: Combine, alpha: Combine) {
    glide.color_combine(color);
    glide.alpha_combine(alpha);
}

/// TMU combine passthrough of TMU0's texel.
pub const DECAL: TexCombine = TexCombine::new(
    CombineFunction::Local,
    CombineFactor::Zero,
    CombineFunction::Local,
    CombineFactor::Zero,
    false,
    false,
);
