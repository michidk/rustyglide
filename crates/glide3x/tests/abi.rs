//! Drives the Glide 3 exports the way a game does: a custom vertex layout,
//! array draws, and a read-back through the LFB. Needs a GPU (lavapipe in CI).

use glide3x::*;

/// The game's vertex: position then color, 20 bytes.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    x: f32,
    y: f32,
    r: f32,
    g: f32,
    b: f32,
}

fn v(x: f32, y: f32, r: f32, g: f32, b: f32) -> Vertex {
    Vertex { x, y, r, g, b }
}

#[test]
fn layout_array_draws_and_read_back() {
    grGlideInit();
    if grSstWinOpen(0, 7, 0, 0, 0, 2, 1) == 0 {
        assert_ne!(std::env::var("RUSTYGLIDE_REQUIRE_GPU").as_deref(), Ok("1"), "no GPU although required");
        eprintln!("skipped: no GPU");
        grGlideShutdown();
        return;
    }
    grVertexLayout(0x01, 0, 1); // GR_PARAM_XY
    grVertexLayout(0x20, 8, 1); // GR_PARAM_RGB
    grColorCombine(1, 0, 0, 2, 0); // LOCAL, iterated
    grBufferClear(0, 0, 0);
    let red = [v(10.0, 10.0, 255.0, 0.0, 0.0), v(200.0, 10.0, 255.0, 0.0, 0.0), v(10.0, 200.0, 255.0, 0.0, 0.0)];
    // SAFETY: three vertices, 20 bytes apart.
    unsafe { grDrawVertexArrayContiguous(6, 3, red.as_ptr().cast(), 20) };
    // A two-triangle strip covering a square; the second triangle must not be culled away
    // by a flipped winding, so turn culling on.
    grCullMode(1);
    let g = |x, y| v(x, y, 0.0, 255.0, 0.0);
    let strip = [g(300.0, 300.0), g(400.0, 300.0), g(300.0, 400.0), g(400.0, 400.0)];
    let pointers: Vec<*const std::ffi::c_void> = strip.iter().map(|p| (p as *const Vertex).cast()).collect();
    // SAFETY: four vertex pointers.
    unsafe { grDrawVertexArray(4, 4, pointers.as_ptr()) };
    let mut pixels = vec![0u16; 640 * 480];
    // SAFETY: a 640x480 16-bit buffer with a 1280-byte stride.
    assert_eq!(unsafe { grLfbReadRegion(1, 0, 0, 640, 480, 1280, pixels.as_mut_ptr().cast()) }, 1);
    let at = |x: usize, y: usize| pixels[y * 640 + x];
    assert_eq!(at(50, 50), 0xf800, "triangle from the contiguous array");
    assert_eq!(at(320, 320), 0x07e0, "first strip triangle");
    assert_eq!(at(390, 390), 0x07e0, "second strip triangle, same winding");
    let mut layout_size = 0i32;
    // SAFETY: one i32.
    assert_eq!(unsafe { grGet(0x07, 4, &mut layout_size) }, 4);
    assert!(layout_size > 0);
    assert_eq!(grSstWinClose(1), 1);
    grGlideShutdown();
}
