//! `grGlideGetState` / `grGlideSetState`: the rendering state as an opaque blob.
//!
//! The blob must fit Glide's `GrState` (272 bytes in 2.1x, 312 in 2.4x), so
//! it holds packed words, not texture memory or tables.

use crate::combine::{Combine, TexCombine};
use crate::texture::TexInfo;
use crate::types::*;
use crate::{Blend, Context, TexSource};

const MAGIC: u32 = 0x5247_5331; // "RGS1"
pub const SIZE: usize = 45 * 4;
const _: () = assert!(SIZE <= 272);

fn get<T: TryFrom<u32> + Default>(v: u32) -> T {
    T::try_from(v).unwrap_or_default()
}

fn unpack_combine(v: u32) -> Combine {
    Combine::new(get(v & 0x1f), get((v >> 5) & 0xf), get((v >> 9) & 3), get((v >> 11) & 3), v >> 13 & 1 != 0)
}

fn unpack_tex_combine(v: u32) -> TexCombine {
    TexCombine::new(
        get(v & 0x1f),
        get((v >> 5) & 0xf),
        get((v >> 9) & 0x1f),
        get((v >> 14) & 0xf),
        v >> 18 & 1 != 0,
        v >> 19 & 1 != 0,
    )
}

pub fn save(c: &Context, out: &mut [u8]) {
    let mut w: Vec<u32> = vec![
        MAGIC,
        c.color.pack(),
        c.alpha.pack(),
        c.delta0 as u32 | (c.alpha_controls_itrgb as u32) << 1,
        c.constant,
        c.blend.rgb_src as u32
            | (c.blend.rgb_dst as u32) << 8
            | (c.blend.alpha_src as u32) << 16
            | (c.blend.alpha_dst as u32) << 24,
        c.alpha_test as u32 | (c.alpha_ref as u32) << 8 | (c.chroma as u32) << 16,
        c.chroma_value,
        c.clip.0 | c.clip.1 << 16,
        c.clip.2 | c.clip.3 << 16,
        c.write_rgb as u32
            | (c.write_alpha as u32) << 1
            | (c.cull as u32) << 2
            | (c.depth_mode as u32) << 4
            | (c.depth_func as u32) << 8
            | (c.depth_mask as u32) << 12
            | (c.dither as u32) << 13,
        c.depth_bias as u16 as u32,
        c.fog_mode.0,
        c.fog_color,
    ];
    w.extend(c.fog_table.chunks(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])));
    w.push(c.origin as u32 | (c.render_buffer as u32) << 8 | c.stw_hint << 16);
    for i in 0..2 {
        let Some(t) = c.tmus.get(i) else {
            w.extend([0; 7]);
            continue;
        };
        w.push(t.combine.pack());
        w.push(
            t.min_filter as u32
                | (t.mag_filter as u32) << 1
                | (t.mipmap as u32) << 2
                | (t.lod_blend as u32) << 4
                | (t.clamp_s as u32) << 5
                | (t.clamp_t as u32) << 6
                | (t.ncc_select as u32) << 7,
        );
        w.push(t.lod_bias.to_bits());
        w.push(
            (t.detail.0 as i8 as u8 as u32)
                | (t.detail.1 as u32) << 8
                | ((t.detail.2.clamp(0.0, 1.0) * 255.0) as u32) << 16,
        );
        match t.source {
            Some(s) => {
                w.push(s.address);
                w.push(
                    1 | s.even_odd << 1
                        | (s.info.small_lod as u32) << 4
                        | (s.info.large_lod as u32) << 8
                        | (s.info.aspect as u32) << 12
                        | (s.info.format as u32) << 16,
                );
            }
            None => w.extend([0, 0]),
        }
        w.push(0);
    }
    debug_assert_eq!(w.len() * 4, SIZE);
    for (chunk, word) in out.as_chunks_mut::<4>().0.iter_mut().zip(w) {
        *chunk = word.to_le_bytes();
    }
}

pub fn restore(c: &mut Context, data: &[u8]) {
    let w: Vec<u32> = data.as_chunks::<4>().0.iter().map(|&b| u32::from_le_bytes(b)).collect();
    if w.len() * 4 < SIZE || w[0] != MAGIC {
        return;
    }
    c.color = unpack_combine(w[1]);
    c.alpha = unpack_combine(w[2]);
    c.delta0 = w[3] & 1 != 0;
    c.alpha_controls_itrgb = w[3] & 2 != 0;
    c.constant = w[4];
    c.blend = Blend {
        rgb_src: get(w[5] & 0xff),
        rgb_dst: get((w[5] >> 8) & 0xff),
        alpha_src: get((w[5] >> 16) & 0xff),
        alpha_dst: get(w[5] >> 24),
    };
    c.alpha_test = get(w[6] & 0xff);
    c.alpha_ref = (w[6] >> 8) as u8;
    c.chroma = get((w[6] >> 16) & 0xff);
    c.chroma_value = w[7];
    c.clip = (w[8] & 0xffff, w[8] >> 16, w[9] & 0xffff, w[9] >> 16);
    c.write_rgb = w[10] & 1 != 0;
    c.write_alpha = w[10] & 2 != 0;
    c.cull = get((w[10] >> 2) & 3);
    c.depth_mode = get((w[10] >> 4) & 0xf);
    c.depth_func = get((w[10] >> 8) & 0xf);
    c.depth_mask = w[10] >> 12 & 1 != 0;
    c.dither = get((w[10] >> 13) & 3);
    c.depth_bias = w[11] as u16 as i16;
    c.fog_mode = FogMode(w[12]);
    c.fog_color = w[13];
    for (i, word) in w[14..30].iter().enumerate() {
        c.fog_table[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    c.origin = get(w[30] & 0xff);
    c.render_buffer = get((w[30] >> 8) & 0xff);
    c.stw_hint = w[30] >> 16;
    for i in 0..c.tmus.len().min(2) {
        let base = 31 + i * 7;
        let t = &mut c.tmus[i];
        t.combine = unpack_tex_combine(w[base]);
        let m = w[base + 1];
        t.min_filter = get(m & 1);
        t.mag_filter = get((m >> 1) & 1);
        t.mipmap = get((m >> 2) & 3);
        t.lod_blend = m >> 4 & 1 != 0;
        t.clamp_s = get((m >> 5) & 1);
        t.clamp_t = get((m >> 6) & 1);
        t.ncc_select = get((m >> 7) & 1);
        t.lod_bias = f32::from_bits(w[base + 2]);
        let d = w[base + 3];
        t.detail = (d as u8 as i8 as i32, (d >> 8) as u8, ((d >> 16) & 0xff) as f32 / 255.0);
        let s = w[base + 5];
        t.source = (s & 1 != 0).then(|| TexSource {
            address: w[base + 4],
            even_odd: (s >> 1) & 3,
            info: TexInfo {
                small_lod: get((s >> 4) & 0xf),
                large_lod: get((s >> 8) & 0xf),
                aspect: get((s >> 12) & 0xf),
                format: get((s >> 16) & 0xff),
            },
        });
    }
}
