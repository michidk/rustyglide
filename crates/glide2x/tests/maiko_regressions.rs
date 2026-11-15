//! The renderer checks behind Maiko's OpenGLide patches (Maiko's
//! `tests/backend_smoke.cpp`), driven through the same C exports with the same
//! expected values. They guard the behaviour Redguard needs:
//! fog interpolated between table entries, fog looked up per pixel, alpha-only
//! textures at full brightness, and LFB writes that bypass fog.

use glide2x::*;
use rustyglide::GrTmuVertex;
use rustyglide::GrVertex;

const GR_FOG_WITH_TABLE: i32 = 2;
const GR_FOG_DISABLE: i32 = 0;
const GR_COLORCOMBINE_CCRGB: i32 = 1;
const GR_COLORCOMBINE_TEXTURE_TIMES_CCRGB: i32 = 5;

fn tri(oow: f32) -> [GrVertex; 3] {
    let p = |x, y| GrVertex { x, y, oow, ..Default::default() };
    [p(40.0, 40.0), p(400.0, 40.0), p(40.0, 400.0)]
}

/// Locks a buffer of 16-bit pixels; returns the pointer and the stride in bytes.
fn lock(write: bool) -> (*mut u8, usize) {
    let mut info = GrLfbInfo::default();
    // SAFETY: a valid GrLfbInfo.
    assert_eq!(unsafe { grLfbLock(write as u32, 1, 0, 0, 0, &mut info) }, 1);
    (info.lfb_ptr.cast(), info.stride_in_bytes as usize)
}

/// Red channel (0..=255) at (x, y) of the back buffer, read through an LFB lock.
fn red_at(x: usize, y: usize) -> i32 {
    let (ptr, stride) = lock(false);
    // SAFETY: the lock buffer holds stride * height bytes.
    let pixel = unsafe { (ptr.add(y * stride + x * 2) as *const u16).read_unaligned() };
    assert_eq!(grLfbUnlock(0, 1), 1);
    ((pixel >> 11) as i32) * 255 / 31
}

fn fogged_white(depth: f32) -> i32 {
    grBufferClear(0, 0, 0xffff);
    let [a, b, c] = tri(1.0 / depth);
    // SAFETY: valid vertices.
    unsafe { grDrawTriangle(&a, &b, &c) };
    red_at(100, 100)
}

fn check_fog_interpolation() {
    guColorCombineFunction(GR_COLORCOMBINE_CCRGB);
    grConstantColorValue(0xffff_ffff);
    grFogColorValue(0);
    grFogMode(GR_FOG_WITH_TABLE);
    let mut fog = [0u8; 64];
    for (i, entry) in fog.iter_mut().enumerate().skip(44) {
        *entry = match i {
            44 => 38,
            45 => 76,
            46 => 136,
            47 => 196,
            _ => 255,
        };
    }
    // SAFETY: 64 entries.
    unsafe { grFogTable(fog.as_ptr()) };
    // The old lookup jumped a whole entry as a vertex moved across these depths.
    for (depth, expected) in
        [(0.5, 255), (2730.65, 119), (2730.68, 119), (3000.0, 87), (3276.79, 59), (3276.81, 59), (65535.0, 0)]
    {
        let red = fogged_white(depth);
        assert!((red - expected).abs() <= 8, "fog at depth {depth}: red {red}, expected {expected}");
    }
    // Falling tables: halfway in reciprocal depth between 255 and 0 gives 127.5.
    fog[20] = 255;
    fog[21] = 0;
    // SAFETY: 64 entries.
    unsafe { grFogTable(fog.as_ptr()) };
    let midpoint = 2.0 / (1.0 / guFogTableIndexToW(20) + 1.0 / guFogTableIndexToW(21));
    let red = fogged_white(midpoint);
    assert!((120..=136).contains(&red), "fog at the reciprocal midpoint: red {red}, expected 128");
    grFogMode(GR_FOG_DISABLE);
}

fn table_fog(fog: &[u8; 64], w: f32) -> f32 {
    let i = (0..64).find(|&i| guFogTableIndexToW(i) >= w).unwrap_or(64) as usize;
    if i == 0 {
        return fog[0] as f32 / 255.0;
    }
    if i == 64 {
        return fog[63] as f32 / 255.0;
    }
    let lo = 1.0 / guFogTableIndexToW(i as i32 - 1);
    let hi = 1.0 / guFogTableIndexToW(i as i32);
    let t = (lo - 1.0 / w) / (lo - hi);
    (fog[i - 1] as f32 + t * (fog[i] as f32 - fog[i - 1] as f32)) / 255.0
}

fn check_per_pixel_fog() {
    guColorCombineFunction(GR_COLORCOMBINE_CCRGB);
    grConstantColorValue(0xffff_ffff);
    grFogColorValue(0);
    grFogMode(GR_FOG_WITH_TABLE);
    let fog: [u8; 64] = std::array::from_fn(|i| {
        if i < 30 {
            0
        } else if i > 50 {
            255
        } else {
            ((i - 30) * 12) as u8
        }
    });
    // SAFETY: 64 entries.
    unsafe { grFogTable(fog.as_ptr()) };
    grBufferClear(0, 0, 0xffff);
    let [mut a, mut b, mut c] = tri(1.0);
    (a.oow, b.oow, c.oow) = (1.0 / 200.0, 1.0 / 30000.0, 1.0 / 2000.0);
    // SAFETY: valid vertices.
    unsafe { grDrawTriangle(&a, &b, &c) };
    for (x, y) in [(80, 60), (120, 100), (200, 60), (60, 200), (150, 150), (100, 250)] {
        let lb = (x as f32 - 40.0) / 360.0;
        let lc = (y as f32 - 40.0) / 360.0;
        let la = 1.0 - lb - lc;
        let oow = la * a.oow + lb * b.oow + lc * c.oow;
        let expected = (255.0 * (1.0 - table_fog(&fog, 1.0 / oow))) as i32;
        let red = red_at(x, y);
        assert!((red - expected).abs() <= 10, "pixel fog at ({x}, {y}): red {red}, expected {expected}");
    }
    grFogMode(GR_FOG_DISABLE);
}

fn check_alpha_texture() {
    let mut pixels = [0u8; 256];
    let info = |data: *mut u8| GrTexInfo { small_lod: 4, large_lod: 4, aspect_ratio: 3, format: 2, data: data.cast() };
    let address = grTexMinAddress(0);
    grTexFilterMode(0, 1, 1);
    guTexCombineFunction(0, 1); // decal
    guColorCombineFunction(GR_COLORCOMBINE_TEXTURE_TIMES_CCRGB);
    grConstantColorValue(0xffff_ffff);
    grAlphaCombine(3, 8, 1, 1, 0); // SCALE_OTHER, ONE, constant, texture
    grAlphaBlendFunction(1, 5, 4, 0); // SRC_ALPHA, ONE_MINUS_SRC_ALPHA, ONE, ZERO
    // Alpha-only textures supply white RGB at every opacity: over white nothing
    // darkens, over black the result equals the opacity.
    for background in [0u32, 255] {
        for alpha in [0i32, 64, 128, 192, 255, -1] {
            for (i, p) in pixels.iter_mut().enumerate() {
                *p = if alpha < 0 { if i % 16 < 8 { 0 } else { 255 } } else { alpha as u8 };
            }
            let tex = info(pixels.as_mut_ptr());
            // SAFETY: a 16x16 A8 texture.
            unsafe {
                grTexDownloadMipMap(0, address, 3, &tex);
                grTexSource(0, address, 3, &tex);
            }
            grBufferClear(if background != 0 { 0xffff_ffff } else { 0 }, 255, 0xffff);
            let t = GrTmuVertex { sow: 128.0, tow: 128.0, oow: 1.0 };
            let p = |x, y| GrVertex {
                x,
                y,
                r: 255.0,
                g: 255.0,
                b: 255.0,
                a: 255.0,
                oow: 1.0,
                tmuvtx: [t; 2],
                ..Default::default()
            };
            // SAFETY: valid vertices.
            unsafe { grDrawTriangle(&p(40.0, 40.0), &p(400.0, 40.0), &p(40.0, 400.0)) };
            let red = red_at(100, 100);
            let expected = if background != 0 {
                255
            } else if alpha < 0 {
                128
            } else {
                alpha
            };
            assert!(
                (red - expected).abs() <= 8,
                "alpha-only texture over {background} at opacity {alpha}: red {red}, expected {expected}"
            );
        }
    }
    grAlphaBlendFunction(4, 0, 4, 0);
}

fn check_lfb_write_unfogged() {
    guColorCombineFunction(GR_COLORCOMBINE_CCRGB);
    grConstantColorValue(0xffff_ffff);
    grFogColorValue(0);
    grFogMode(GR_FOG_WITH_TABLE);
    let fog = [255u8; 64];
    // SAFETY: 64 entries.
    unsafe { grFogTable(fog.as_ptr()) };
    let fogged = fogged_white(200.0);
    let (ptr, stride) = lock(true);
    for y in 80..120 {
        for x in 80..120 {
            // SAFETY: inside the locked buffer.
            unsafe { (ptr.add(y * stride + x * 2) as *mut u16).write_unaligned(0xffff) };
        }
    }
    assert_eq!(grLfbUnlock(1, 1), 1);
    let red = red_at(100, 100);
    grFogMode(GR_FOG_DISABLE);
    assert!(fogged < 16 && red == 255, "LFB write over fog: triangle red {fogged}, written red {red}");
}

#[test]
fn maiko_renderer_regressions() {
    grGlideInit();
    if grSstWinOpen(0, 7, 0, 0, 0, 2, 1) == 0 {
        assert_ne!(std::env::var("RUSTYGLIDE_REQUIRE_GPU").as_deref(), Ok("1"), "no GPU although required");
        eprintln!("skipped: no GPU");
        grGlideShutdown();
        return;
    }
    check_fog_interpolation();
    check_per_pixel_fog();
    check_alpha_texture();
    check_lfb_write_unfogged();
    grSstWinClose();
    grGlideShutdown();
}
