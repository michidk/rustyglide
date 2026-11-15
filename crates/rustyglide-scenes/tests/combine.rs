//! Sweeps the FBI color and alpha combine settings and compares the GPU result
//! of each with the CPU model in `reference.rs`.

use rustyglide::combine::Combine;
use rustyglide::texture::TexInfo;
use rustyglide::types::*;
use rustyglide::{GrVertex, combine};
use rustyglide_scenes::reference::{Inputs, shade};
use rustyglide_scenes::{SIZE, gpu_lock, open_or_skip, scenes};

const FUNCTIONS: [CombineFunction; 11] = [
    CombineFunction::Zero,
    CombineFunction::Local,
    CombineFunction::LocalAlpha,
    CombineFunction::ScaleOther,
    CombineFunction::ScaleOtherAddLocal,
    CombineFunction::ScaleOtherAddLocalAlpha,
    CombineFunction::ScaleOtherMinusLocal,
    CombineFunction::ScaleOtherMinusLocalAddLocal,
    CombineFunction::ScaleOtherMinusLocalAddLocalAlpha,
    CombineFunction::ScaleMinusLocalAddLocal,
    CombineFunction::ScaleMinusLocalAddLocalAlpha,
];
const FACTORS: [CombineFactor; 12] = [
    CombineFactor::Zero,
    CombineFactor::Local,
    CombineFactor::OtherAlpha,
    CombineFactor::LocalAlpha,
    CombineFactor::TextureAlpha,
    CombineFactor::TextureRgb,
    CombineFactor::One,
    CombineFactor::OneMinusLocal,
    CombineFactor::OneMinusOtherAlpha,
    CombineFactor::OneMinusLocalAlpha,
    CombineFactor::OneMinusTextureAlpha,
    CombineFactor::OneMinusLodFraction,
];
const LOCALS: [CombineLocal; 2] = [CombineLocal::Iterated, CombineLocal::Constant];
const OTHERS: [CombineOther; 3] = [CombineOther::Iterated, CombineOther::Texture, CombineOther::Constant];

const ITERATED: [u8; 4] = [200, 100, 40, 160];
const CONSTANT: [u8; 4] = [30, 220, 120, 90];
const TEXEL: [u8; 4] = [70, 140, 250, 200];
/// Each combination fills a CELL x CELL square.
const CELL: u32 = 4;

fn all(alpha_sweep: bool) -> Vec<Combine> {
    let mut out = Vec::new();
    for function in FUNCTIONS {
        for factor in FACTORS {
            if alpha_sweep && factor == CombineFactor::TextureRgb {
                continue; // Not an alpha factor.
            }
            for local in LOCALS {
                for other in OTHERS {
                    for invert in [false, true] {
                        out.push(Combine::new(function, factor, local, other, invert));
                    }
                }
            }
        }
    }
    out
}

fn sweep(alpha_sweep: bool) {
    let _gpu = gpu_lock();
    let combos = all(alpha_sweep);
    let per_frame = ((SIZE.0 / CELL) * (SIZE.1 / CELL)) as usize;
    let inputs = Inputs {
        iterated: ITERATED.map(|c| c as f32 / 255.0),
        constant: CONSTANT.map(|c| c as f32 / 255.0),
        texel: TEXEL.map(|c| c as f32 / 255.0),
    };
    let mut failures = Vec::new();
    for chunk in combos.chunks(per_frame) {
        let Some(mut glide) = open_or_skip() else { return };
        // An AP88 texture gives an exact 8-bit texel: RGB from the palette, alpha from the high byte.
        let info =
            TexInfo { small_lod: Lod::L8, large_lod: Lod::L8, aspect: Aspect::A1x1, format: TextureFormat::Ap88 };
        let palette = [(TEXEL[0] as u32) << 16 | (TEXEL[1] as u32) << 8 | TEXEL[2] as u32; 256];
        glide.tex_download_palette(Tmu::Tmu0, &palette, 0);
        let texel = ((TEXEL[3] as u16) << 8).to_le_bytes();
        let data: Vec<u8> = (0..64).flat_map(|_| texel).collect();
        scenes::load_texture(&mut glide, info, &data);
        glide.color_mask(true, true);
        glide.constant_color(u32::from_be_bytes([CONSTANT[3], CONSTANT[0], CONSTANT[1], CONSTANT[2]]));
        let fixed = combine::DEFAULT_COLOR;
        let fixed_alpha = Combine::new(
            CombineFunction::Local,
            CombineFactor::Zero,
            CombineLocal::Constant,
            CombineOther::Constant,
            false,
        );
        for (n, c) in chunk.iter().enumerate() {
            let (color, alpha) = if alpha_sweep { (fixed, *c) } else { (*c, fixed_alpha) };
            scenes::set_combine(&mut glide, color, alpha);
            let cols = SIZE.0 / CELL;
            let (x, y) = ((n as u32 % cols * CELL) as f32, (n as u32 / cols * CELL) as f32);
            let p = |x, y| GrVertex {
                x,
                y,
                r: ITERATED[0] as f32,
                g: ITERATED[1] as f32,
                b: ITERATED[2] as f32,
                a: ITERATED[3] as f32,
                oow: 1.0,
                ooz: 0.0,
                tmuvtx: [rustyglide::GrTmuVertex { sow: 128.0, tow: 128.0, oow: 1.0 }; 2],
                ..Default::default()
            };
            let s = CELL as f32;
            glide.draw_triangle(&p(x, y), &p(x + s, y), &p(x + s, y + s));
            glide.draw_triangle(&p(x, y), &p(x + s, y + s), &p(x, y + s));
        }
        let pixels = glide.read_pixels(Buffer::Back);
        for (n, c) in chunk.iter().enumerate() {
            let (color, alpha) = if alpha_sweep { (fixed, *c) } else { (*c, fixed_alpha) };
            let expected = shade(&color, &alpha, &inputs).map(|v| (v * 255.0).round() as i32);
            let cols = SIZE.0 / CELL;
            let (x, y) = (n as u32 % cols * CELL + CELL / 2, n as u32 / cols * CELL + CELL / 2);
            let at = ((y * SIZE.0 + x) * 4) as usize;
            let got = [0, 1, 2, 3].map(|k| pixels[at + k] as i32);
            let channels = if alpha_sweep { 3..4 } else { 0..3 };
            if channels.clone().any(|k| (got[k] - expected[k]).abs() > 2) {
                failures.push(format!("{c:?}: got {got:?}, expected {expected:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} combinations differ:\n{}",
        failures.len(),
        combos.len(),
        failures[..failures.len().min(20)].join("\n")
    );
}

#[test]
fn color_combine_matches_reference() {
    sweep(false);
}

#[test]
fn alpha_combine_matches_reference() {
    sweep(true);
}
