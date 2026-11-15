//! Drives the exported C functions the way a game does: open, draw, read the
//! frame back through the LFB, close. Needs a GPU (lavapipe in CI).

use glide2x::*;
use rustyglide::GrVertex;

fn v(x: f32, y: f32) -> GrVertex {
    GrVertex { x, y, r: 255.0, g: 0.0, b: 0.0, a: 255.0, oow: 1.0, ..Default::default() }
}

#[test]
fn draw_and_read_back() {
    grGlideInit();
    let mut version = [0i8; 80];
    // SAFETY: an 80-byte buffer, as Glide requires.
    unsafe { grGlideGetVersion(version.as_mut_ptr()) };
    assert!(version.starts_with(&b"Glide 2.".map(|b| b as i8)));
    if grSstWinOpen(0, 7, 0, 0, 0, 2, 1) == 0 {
        assert_ne!(
            std::env::var("RUSTYGLIDE_REQUIRE_GPU").as_deref(),
            Ok("1"),
            "no GPU although RUSTYGLIDE_REQUIRE_GPU=1"
        );
        eprintln!("skipped: no GPU");
        grGlideShutdown();
        return;
    }
    assert_eq!((grSstScreenWidth(), grSstScreenHeight()), (640, 480));
    grBufferClear(0x000000ff, 0, 0);
    guColorCombineFunction(2); // ITRGB
    // SAFETY: valid vertices.
    unsafe { grDrawTriangle(&v(100.0, 100.0), &v(300.0, 100.0), &v(100.0, 300.0)) };
    let mut pixels = vec![0u16; 640 * 480];
    // SAFETY: a 640x480 16-bit buffer with a 1280-byte stride.
    let ok = unsafe { grLfbReadRegion(1, 0, 0, 640, 480, 1280, pixels.as_mut_ptr().cast()) };
    assert_eq!(ok, 1);
    assert_eq!(pixels[150 * 640 + 150], 0xf800, "inside the red triangle");
    assert_eq!(pixels[400 * 640 + 400], 0x001f, "the blue clear color");
    let mut stats = (0u32, 0u32);
    // SAFETY: two valid out pointers.
    unsafe { grTriStats(&mut stats.0, &mut stats.1) };
    assert_eq!(stats, (1, 1));
    grSstWinClose();
    grGlideShutdown();
}
