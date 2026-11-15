//! Renders every scene headless and compares it with `golden/<name>.png`.
//!
//! `RUSTYGLIDE_BLESS=1` writes the current output as the new golden image.
//! A failing scene leaves `<name>.actual.png` next to its golden file.

use rustyglide_scenes::{SIZE, differing_pixels, open_or_skip, read_png, render, scenes::ALL, write_png};
use std::path::PathBuf;

/// Channel difference ignored, and the share of pixels allowed to exceed it
/// (rasterization edges differ slightly between drivers).
const TOLERANCE: u8 = 3;
const MAX_DIFFERING: f64 = 0.002;

#[test]
fn scenes_match_golden_images() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden");
    let bless = std::env::var("RUSTYGLIDE_BLESS").is_ok_and(|v| v == "1");
    let mut failures = Vec::new();
    for scene in ALL {
        let Some(mut glide) = open_or_skip() else { return };
        let pixels = render(&mut glide, scene);
        let golden = dir.join(format!("{}.png", scene.name));
        if bless {
            write_png(&golden, SIZE, &pixels).expect("write golden image");
            continue;
        }
        let Ok((size, expected)) = read_png(&golden) else {
            failures.push(format!("{}: no golden image (run with RUSTYGLIDE_BLESS=1)", scene.name));
            continue;
        };
        let differing = if size == SIZE { differing_pixels(&pixels, &expected, TOLERANCE) } else { usize::MAX };
        if differing as f64 > (SIZE.0 * SIZE.1) as f64 * MAX_DIFFERING {
            let _ = write_png(&dir.join(format!("{}.actual.png", scene.name)), SIZE, &pixels);
            failures.push(format!("{}: {differing} pixels differ", scene.name));
        }
    }
    assert!(failures.is_empty(), "scene mismatches:\n{}", failures.join("\n"));
}
