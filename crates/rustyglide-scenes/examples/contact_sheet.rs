//! Writes all golden images side by side, scaled 2x, for review:
//! `cargo run -p rustyglide-scenes --example contact_sheet -- out.png`.

use rustyglide_scenes::{SIZE, read_png, scenes::ALL, write_png};
use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let out = std::env::args().nth(1).unwrap_or_else(|| "contact_sheet.png".into());
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("golden");
    let (cols, scale, gap) = (4u32, 2u32, 4u32);
    let rows = (ALL.len() as u32).div_ceil(cols);
    let (cw, ch) = (SIZE.0 * scale + gap, SIZE.1 * scale + gap);
    let (w, h) = (cols * cw, rows * ch);
    let mut sheet = vec![255u8; (w * h * 4) as usize];
    for (n, scene) in ALL.iter().enumerate() {
        let (_, px) = read_png(&dir.join(format!("{}.png", scene.name)))?;
        let (ox, oy) = (n as u32 % cols * cw, n as u32 / cols * ch);
        for y in 0..SIZE.1 * scale {
            for x in 0..SIZE.0 * scale {
                let src = (((y / scale) * SIZE.0 + x / scale) * 4) as usize;
                let dst = (((oy + y) * w + ox + x) * 4) as usize;
                sheet[dst..dst + 4].copy_from_slice(&px[src..src + 4]);
                sheet[dst + 3] = 255;
            }
        }
    }
    write_png(std::path::Path::new(&out), (w, h), &sheet)?;
    println!("{}", ALL.iter().map(|s| s.name).collect::<Vec<_>>().join(", "));
    Ok(())
}
