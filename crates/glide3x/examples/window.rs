//! Opens a window through the Glide 3 exports and draws a triangle for a few
//! seconds: a manual check of rustyglide's own window on Linux (Wayland or X11).
//!
//! `cargo run -p glide3x --example window [seconds]`

use glide3x::*;

fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    grGlideInit();
    // No window handle, 640x480 (GR_RESOLUTION_640x480 = 7), ARGB, upper-left origin.
    assert_eq!(grSstWinOpen(0, 7, 0, 0, 0, 2, 1), 1, "grSstWinOpen failed");
    grVertexLayout(0x01, 0, 1); // GR_PARAM_XY
    grVertexLayout(0x20, 8, 1); // GR_PARAM_RGB
    grColorCombine(1, 0, 0, 2, 0); // iterated colour
    let triangle: [[f32; 5]; 3] =
        [[320.0, 60.0, 255.0, 0.0, 0.0], [580.0, 420.0, 0.0, 255.0, 0.0], [60.0, 420.0, 0.0, 0.0, 255.0]];
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < seconds {
        grBufferClear(0x0020_2020, 0, 0);
        // SAFETY: three vertices of five f32s, 20 bytes apart.
        unsafe { grDrawVertexArrayContiguous(6, 3, triangle.as_ptr().cast(), 20) };
        grBufferSwap(1);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    grSstWinClose(1);
    grGlideShutdown();
}
