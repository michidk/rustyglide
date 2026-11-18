//! Texture memory layout, sizes and format decoding.
//!
//! Texture memory is a byte array per TMU, laid out as on the hardware so that
//! addresses and sizes games compute from `grTexCalcMemRequired` and friends
//! stay valid. Level sizes follow the SST-1 rules: a level occupies at least 4
//! texels, and a whole texture is padded to 8 bytes.

use crate::types::{Aspect, Lod, TextureFormat, level_mask};

/// `GrTexInfo` with typed fields; `data` is supplied separately by callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TexInfo {
    pub small_lod: Lod,
    pub large_lod: Lod,
    pub aspect: Aspect,
    pub format: TextureFormat,
}

impl TexInfo {
    /// The texture's levels, largest first.
    pub fn lods(&self) -> impl Iterator<Item = Lod> + use<> {
        lods(self.large_lod, self.small_lod)
    }

    /// Bytes of every level in host memory, as `grTexDownloadMipMap` reads them.
    pub fn host_size(&self) -> usize {
        self.lods().map(|l| level_host_bytes(l, self.aspect, self.format)).sum()
    }
}

/// Width and height in texels of `lod` for a texture of `aspect`.
pub fn level_dims(lod: Lod, aspect: Aspect) -> (u32, u32) {
    let long = lod.size();
    let (wshift, hshift) = match aspect {
        Aspect::A8x1 => (0, 3),
        Aspect::A4x1 => (0, 2),
        Aspect::A2x1 => (0, 1),
        Aspect::A1x1 => (0, 0),
        Aspect::A1x2 => (1, 0),
        Aspect::A1x4 => (2, 0),
        Aspect::A1x8 => (3, 0),
    };
    ((long >> wshift).max(1), (long >> hshift).max(1))
}

/// Texels a level takes in texture memory: its area, at least 4.
fn level_mem_texels(lod: Lod, aspect: Aspect) -> u32 {
    let (w, h) = level_dims(lod, aspect);
    (w * h).max(4)
}

/// Texels a level takes in host memory (no minimum), as `grTexDownloadMipMap` steps.
pub fn level_host_texels(lod: Lod, aspect: Aspect) -> u32 {
    let (w, h) = level_dims(lod, aspect);
    w * h
}

/// Bytes a level takes in host memory, as `grTexDownloadMipMap` steps.
pub fn level_host_bytes(lod: Lod, aspect: Aspect, format: TextureFormat) -> usize {
    (level_host_texels(lod, aspect) * format.bytes_per_texel()) as usize
}

/// The levels from `large` down to `small`, largest first.
pub fn lods(large: Lod, small: Lod) -> impl Iterator<Item = Lod> {
    (large as u32..=small as u32).filter_map(Lod::from_index)
}

/// Whether `lod` is stored on a TMU holding the levels selected by `even_odd`.
pub fn level_in_mask(lod: Lod, even_odd: u32) -> bool {
    match even_odd & level_mask::BOTH {
        level_mask::BOTH => true,
        level_mask::EVEN => (lod as u32) & 1 == 0,
        level_mask::ODD => (lod as u32) & 1 == 1,
        _ => false,
    }
}

/// `grTexCalcMemRequired` / `grTexTextureMemRequired`.
pub fn mem_required(small: Lod, large: Lod, aspect: Aspect, format: TextureFormat, even_odd: u32) -> u32 {
    let texels: u32 =
        lods(large, small).filter(|&l| level_in_mask(l, even_odd)).map(|l| level_mem_texels(l, aspect)).sum();
    (texels * format.bytes_per_texel() + 7) & !7
}

/// Byte offset of `lod` from a texture's start address. With split levels
/// (`even_odd` not BOTH) only the levels on this TMU take space.
pub fn level_offset(lod: Lod, large: Lod, aspect: Aspect, format: TextureFormat, even_odd: u32) -> u32 {
    let texels: u32 = (large as u32..lod as u32)
        .filter_map(Lod::from_index)
        .filter(|&l| level_in_mask(l, even_odd))
        .map(|l| level_mem_texels(l, aspect))
        .sum();
    texels * format.bytes_per_texel()
}

/// A 256-entry palette of `0x00RRGGBB` values.
pub type Palette = [u32; 256];

/// An NCC (YIQ) decompression table, `GuNccTable` without its packed copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NccData {
    pub y: [u8; 16],
    pub i: [[i16; 3]; 4],
    pub q: [[i16; 3]; 4],
}

impl NccData {
    /// Reads a `GuNccTable` from its in-memory layout (16 + 24 + 24 + 48 bytes).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 64 {
            return None;
        }
        let mut t = Self::default();
        t.y.copy_from_slice(&bytes[..16]);
        let word = |i: usize| i16::from_le_bytes([bytes[i], bytes[i + 1]]);
        for k in 0..4 {
            for c in 0..3 {
                t.i[k][c] = word(16 + (k * 3 + c) * 2);
                t.q[k][c] = word(40 + (k * 3 + c) * 2);
            }
        }
        Some(t)
    }

    fn rgb(&self, texel: u8) -> [u8; 3] {
        let y = self.y[(texel >> 4) as usize] as i32;
        let i = self.i[((texel >> 2) & 3) as usize];
        let q = self.q[(texel & 3) as usize];
        // I and Q are 9-bit two's complement values.
        let s9 = |v: i16| ((v as i32) << 23) >> 23;
        let c = |k: usize| (y + s9(i[k]) + s9(q[k])).clamp(0, 255) as u8;
        [c(0), c(1), c(2)]
    }
}

/// Tables a texture may need to decode.
pub struct DecodeTables<'a> {
    pub palette: &'a Palette,
    pub ncc: &'a NccData,
}

pub(crate) fn expand(v: u32, bits: u32) -> u8 {
    // Replicate the high bits into the low ones, as the hardware does.
    let v = v << (8 - bits);
    let mut out = v;
    let mut shift = bits;
    while shift < 8 {
        out |= v >> shift;
        shift += bits;
    }
    out as u8
}

fn palette_rgb(entry: u32) -> [u8; 3] {
    [(entry >> 16) as u8, (entry >> 8) as u8, entry as u8]
}

/// Decodes `w*h` texels of `format` from `src` into RGBA8 (`out` gets `4*w*h` bytes).
pub fn decode(format: TextureFormat, src: &[u8], texels: usize, tables: &DecodeTables<'_>, out: &mut Vec<u8>) {
    out.reserve(texels * 4);
    let byte = |i: usize| src.get(i).copied().unwrap_or(0);
    let word = |i: usize| u16::from_le_bytes([byte(2 * i), byte(2 * i + 1)]) as u32;
    for t in 0..texels {
        let rgba: [u8; 4] = match format {
            TextureFormat::Rgb332 => {
                let v = byte(t) as u32;
                [expand(v >> 5, 3), expand((v >> 2) & 7, 3), expand(v & 3, 2), 255]
            }
            TextureFormat::Yiq422 => {
                let [r, g, b] = tables.ncc.rgb(byte(t));
                [r, g, b, 255]
            }
            // Alpha-only texels are full-intensity white with the given opacity.
            TextureFormat::Alpha8 => [255, 255, 255, byte(t)],
            TextureFormat::Intensity8 => {
                let i = byte(t);
                [i, i, i, 255]
            }
            TextureFormat::AlphaIntensity44 => {
                let v = byte(t) as u32;
                let i = expand(v & 0xf, 4);
                [i, i, i, expand(v >> 4, 4)]
            }
            TextureFormat::P8 => {
                let [r, g, b] = palette_rgb(tables.palette[byte(t) as usize]);
                [r, g, b, 255]
            }
            TextureFormat::Argb8332 => {
                let v = word(t);
                [expand((v >> 5) & 7, 3), expand((v >> 2) & 7, 3), expand(v & 3, 2), (v >> 8) as u8]
            }
            TextureFormat::Ayiq8422 => {
                let v = word(t);
                let [r, g, b] = tables.ncc.rgb(v as u8);
                [r, g, b, (v >> 8) as u8]
            }
            TextureFormat::Rgb565 => {
                let v = word(t);
                [expand(v >> 11, 5), expand((v >> 5) & 0x3f, 6), expand(v & 0x1f, 5), 255]
            }
            TextureFormat::Argb1555 => {
                let v = word(t);
                [
                    expand((v >> 10) & 0x1f, 5),
                    expand((v >> 5) & 0x1f, 5),
                    expand(v & 0x1f, 5),
                    if v & 0x8000 != 0 { 255 } else { 0 },
                ]
            }
            TextureFormat::Argb4444 => {
                let v = word(t);
                [expand((v >> 8) & 0xf, 4), expand((v >> 4) & 0xf, 4), expand(v & 0xf, 4), expand(v >> 12, 4)]
            }
            TextureFormat::AlphaIntensity88 => {
                let v = word(t);
                let i = v as u8;
                [i, i, i, (v >> 8) as u8]
            }
            TextureFormat::Ap88 => {
                let v = word(t);
                let [r, g, b] = palette_rgb(tables.palette[(v & 0xff) as usize]);
                [r, g, b, (v >> 8) as u8]
            }
        };
        out.extend_from_slice(&rgba);
    }
}

/// One TMU's texture memory.
#[derive(Clone)]
pub struct TmuMemory {
    bytes: Vec<u8>,
}

impl TmuMemory {
    pub fn new(size: usize) -> Self {
        Self { bytes: vec![0; size] }
    }

    pub fn len(&self) -> u32 {
        self.bytes.len() as u32
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Copies `data` to `address`, clipped to memory. Returns the written range.
    pub fn write(&mut self, address: u32, data: &[u8]) -> std::ops::Range<u32> {
        let start = (address as usize).min(self.bytes.len());
        let end = (start + data.len()).min(self.bytes.len());
        self.bytes[start..end].copy_from_slice(&data[..end - start]);
        start as u32..end as u32
    }

    pub fn read(&self, address: u32, len: usize) -> &[u8] {
        let start = (address as usize).min(self.bytes.len());
        let end = (start + len).min(self.bytes.len());
        &self.bytes[start..end]
    }
}

/// Decodes the mipmap chain of a texture into RGBA8 levels, largest first,
/// reading each level at `level_address`. Levels this TMU does not hold (split
/// even/odd textures) are filled by downsampling the previous level.
pub fn decode_chain(
    memory: &TmuMemory,
    even_odd: u32,
    info: &TexInfo,
    tables: &DecodeTables<'_>,
    level_address: impl Fn(Lod) -> u32,
) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    for lod in lods(info.large_lod, info.small_lod) {
        let (w, h) = level_dims(lod, info.aspect);
        let texels = (w * h) as usize;
        if level_in_mask(lod, even_odd) {
            let src = memory.read(level_address(lod), texels * info.format.bytes_per_texel() as usize);
            let mut rgba = Vec::new();
            decode(info.format, src, texels, tables, &mut rgba);
            levels.push((w, h, rgba));
        } else if let Some(prev) = levels.last() {
            levels.push((w, h, downsample(prev, w, h)));
        } else {
            levels.push((w, h, vec![0; texels * 4]));
        }
    }
    levels
}

fn downsample((pw, ph, src): &(u32, u32, Vec<u8>), w: u32, h: u32) -> Vec<u8> {
    let (sx, sy) = (pw / w, ph / h);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let mut sum = 0u32;
                for dy in 0..sy {
                    for dx in 0..sx {
                        sum += src[(((y * sy + dy) * pw + x * sx + dx) * 4 + c) as usize] as u32;
                    }
                }
                out.push((sum / (sx * sy)) as u8);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mem_required_matches_sst1_tables() {
        // 256x256 16-bit with every level: 0x15555 texels padded, times 2.
        let full = mem_required(Lod::L1, Lod::L256, Aspect::A1x1, TextureFormat::Rgb565, level_mask::BOTH);
        let texels = 0x10000 + 0x4000 + 0x1000 + 0x400 + 0x100 + 0x40 + 0x10 + 4 + 4;
        assert_eq!(full, (texels * 2 + 7) & !7);
        // 8-bit 1x1 still takes 4 texels, rounded to 8 bytes.
        assert_eq!(mem_required(Lod::L1, Lod::L1, Aspect::A1x1, TextureFormat::P8, level_mask::BOTH), 8);
        // Odd levels only of 256..64: just 128.
        assert_eq!(mem_required(Lod::L64, Lod::L256, Aspect::A1x1, TextureFormat::P8, level_mask::ODD), 0x4000);
    }

    #[test]
    fn aspect_dims() {
        assert_eq!(level_dims(Lod::L256, Aspect::A8x1), (256, 32));
        assert_eq!(level_dims(Lod::L256, Aspect::A1x4), (64, 256));
        assert_eq!(level_dims(Lod::L4, Aspect::A1x8), (1, 4));
        assert_eq!(level_dims(Lod::L1, Aspect::A8x1), (1, 1));
    }

    #[test]
    fn level_offsets_skip_previous_levels() {
        let o = level_offset(Lod::L64, Lod::L256, Aspect::A2x1, TextureFormat::Rgb565, level_mask::BOTH);
        assert_eq!(o, (256 * 128 + 128 * 64) * 2);
        let o = level_offset(Lod::L32, Lod::L256, Aspect::A1x1, TextureFormat::P8, level_mask::EVEN);
        assert_eq!(o, 256 * 256 + 64 * 64);
    }

    #[test]
    fn decodes_formats() {
        let palette = [0x00102030u32; 256];
        let ncc = NccData::default();
        let tables = DecodeTables { palette: &palette, ncc: &ncc };
        let mut out = Vec::new();
        decode(TextureFormat::Rgb565, &0xf800u16.to_le_bytes(), 1, &tables, &mut out);
        assert_eq!(out, [255, 0, 0, 255]);
        out.clear();
        decode(TextureFormat::Argb1555, &0x7c1fu16.to_le_bytes(), 1, &tables, &mut out);
        assert_eq!(out, [255, 0, 255, 0]);
        out.clear();
        decode(TextureFormat::Ap88, &0x8005u16.to_le_bytes(), 1, &tables, &mut out);
        assert_eq!(out, [0x10, 0x20, 0x30, 0x80]);
        out.clear();
        decode(TextureFormat::Rgb332, &[0b1110_0011], 1, &tables, &mut out);
        assert_eq!(out, [255, 0, 255, 255]);
        out.clear();
        decode(TextureFormat::Alpha8, &[0x40], 1, &tables, &mut out);
        assert_eq!(out, [255, 255, 255, 0x40]);
    }

    #[test]
    fn ncc_decodes_yiq() {
        let mut ncc = NccData::default();
        ncc.y[0xf] = 200;
        ncc.i[1] = [10, -20, 0];
        ncc.q[2] = [100, 0, -250];
        assert_eq!(ncc.rgb(0xf6), [255, 180, 0]);
    }
}
