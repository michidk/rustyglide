//! Feature scenes for rustyglide, each a short Glide program, plus helpers to
//! render them headless and compare them with golden images.
//!
//! The scenes are written from the descriptions of the 3dfx SDK test programs
//! (clear, gouraud, Z and W buffering, blending, fog, chroma key, culling, LFB,
//! textures, ...), not translated from their sources.

pub mod reference;
pub mod scenes;

use rustyglide::types::*;
use rustyglide::{Glide, GrVertex, Settings, Target};

/// Size scenes render at.
pub const SIZE: (u32, u32) = (160, 120);

/// A named Glide program.
pub struct Scene {
    pub name: &'static str,
    pub draw: fn(&mut Glide),
}

/// Whether GPU tests must run. Without it they are skipped on machines with
/// no GPU adapter; CI and `dev/scripts/gpu-test.sh` set it.
pub fn gpu_required() -> bool {
    std::env::var("RUSTYGLIDE_REQUIRE_GPU").is_ok_and(|v| v == "1")
}

/// Opens a headless Glide, or `None` (after printing why) when there is no
/// GPU and none is required.
pub fn open_or_skip() -> Option<Glide> {
    match open() {
        Ok(glide) => Some(glide),
        Err(e) if !gpu_required() => {
            eprintln!("skipped: no GPU ({e})");
            None
        }
        Err(e) => panic!("no GPU although RUSTYGLIDE_REQUIRE_GPU=1: {e}"),
    }
}

/// Opens a headless Glide of [`SIZE`].
pub fn open() -> Result<Glide, rustyglide::Error> {
    let mut glide = Glide::new(Settings::default());
    glide.sst_win_open(Target::Headless { size: SIZE }, Resolution::None, ColorFormat::Argb, Origin::UpperLeft)?;
    Ok(glide)
}

/// Renders a scene into `glide` and returns the back buffer as RGBA8.
pub fn render(glide: &mut Glide, scene: &Scene) -> Vec<u8> {
    (scene.draw)(glide);
    glide.read_pixels(Buffer::Back)
}

/// A vertex with position, color and 1/w = 1.
pub fn v(x: f32, y: f32, rgb: [f32; 3]) -> GrVertex {
    GrVertex { x, y, r: rgb[0], g: rgb[1], b: rgb[2], a: 255.0, ooz: 65535.0, oow: 1.0, ..Default::default() }
}

/// A vertex with texture coordinates (s, t in Glide's 0..256 space) at depth `w`.
pub fn vt(x: f32, y: f32, s: f32, t: f32, w: f32) -> GrVertex {
    let oow = 1.0 / w;
    let mut out = v(x, y, [255.0; 3]);
    out.oow = oow;
    out.ooz = 65535.0 * oow;
    out.tmuvtx[0] = rustyglide::GrTmuVertex { sow: s * oow, tow: t * oow, oow };
    out
}

/// Draws an axis-aligned rectangle as two triangles.
pub fn rect(glide: &mut Glide, x0: f32, y0: f32, x1: f32, y1: f32, color: [f32; 4]) {
    let p = |x, y| GrVertex { a: color[3], ..v(x, y, [color[0], color[1], color[2]]) };
    glide.draw_triangle(&p(x0, y0), &p(x1, y0), &p(x1, y1));
    glide.draw_triangle(&p(x0, y0), &p(x1, y1), &p(x0, y1));
}

/// Writes RGBA8 pixels as a PNG.
pub fn write_png(path: &std::path::Path, size: (u32, u32), rgba: &[u8]) -> std::io::Result<()> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)
}

/// Reads an RGBA8 PNG.
pub fn read_png(path: &std::path::Path) -> std::io::Result<((u32, u32), Vec<u8>)> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info().map_err(std::io::Error::other)?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf).map_err(std::io::Error::other)?;
    buf.truncate(info.buffer_size());
    if info.color_type != png::ColorType::Rgba {
        return Err(std::io::Error::other("golden images must be RGBA"));
    }
    Ok(((info.width, info.height), buf))
}

/// Pixels whose largest channel difference exceeds `tolerance`.
pub fn differing_pixels(a: &[u8], b: &[u8], tolerance: u8) -> usize {
    a.as_chunks::<4>()
        .0
        .iter()
        .zip(b.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tolerance))
        .count()
}
