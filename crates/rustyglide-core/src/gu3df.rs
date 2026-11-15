//! The `.3df` texture file format read by `gu3dfGetInfo` and `gu3dfLoad`.
//!
//! A text header of four lines (`3df v1.0`, format, `lod range: S L`,
//! `aspect ratio: W H`), then for NCC formats a big-endian table of 16 Y and
//! 2x12 I/Q words, for palette formats 256 big-endian words, then the mipmap
//! levels largest first with 16-bit texels big-endian.

use crate::texture::{NccData, level_host_texels};
use crate::types::{Aspect, Lod, TextureFormat};

#[derive(Debug, Clone, PartialEq)]
pub enum Table {
    None,
    Ncc(NccData),
    Palette(Box<[u32; 256]>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    pub small_lod: Lod,
    pub large_lod: Lod,
    pub aspect: Aspect,
    pub format: TextureFormat,
    /// Bytes of texel data, every level.
    pub mem_required: u32,
}

/// Parses the header. Returns it and the offset where binary data starts.
pub fn parse_header(file: &[u8]) -> Option<(Header, usize)> {
    let mut lines = Vec::new();
    let mut at = 0;
    while lines.len() < 4 {
        let end = at + file.get(at..)?.iter().position(|&b| b == b'\n')?;
        lines.push(std::str::from_utf8(&file[at..end]).ok()?.trim().to_owned());
        at = end + 1;
    }
    let text = lines.join(" ");
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.first().is_none_or(|w| !w.starts_with("3df")) {
        return None;
    }
    let number = |key: &str, offset: usize| -> Option<u32> {
        let i = words.iter().position(|w| w.eq_ignore_ascii_case(key))?;
        words.get(i + offset)?.parse().ok()
    };
    let small = number("range:", 1)?;
    let large = number("range:", 2)?;
    let aw = number("ratio:", 1)?;
    let ah = number("ratio:", 2)?;
    let format = match words.get(2)?.to_ascii_uppercase().as_str() {
        "I8" => TextureFormat::Intensity8,
        "A8" => TextureFormat::Alpha8,
        "AI44" => TextureFormat::AlphaIntensity44,
        "YIQ" => TextureFormat::Yiq422,
        "RGB332" => TextureFormat::Rgb332,
        "RGB565" => TextureFormat::Rgb565,
        "ARGB8332" => TextureFormat::Argb8332,
        "ARGB1555" => TextureFormat::Argb1555,
        "AYIQ8422" => TextureFormat::Ayiq8422,
        "ARGB4444" => TextureFormat::Argb4444,
        "AI88" => TextureFormat::AlphaIntensity88,
        "P8" => TextureFormat::P8,
        "AP88" => TextureFormat::Ap88,
        _ => return None,
    };
    let aspect = match (aw, ah) {
        (1, 1) => Aspect::A1x1,
        (1, 2) => Aspect::A1x2,
        (1, 4) => Aspect::A1x4,
        (1, 8) => Aspect::A1x8,
        (2, 1) => Aspect::A2x1,
        (4, 1) => Aspect::A4x1,
        (8, 1) => Aspect::A8x1,
        _ => return None,
    };
    let lod = |size: u32| Lod::from_index(8u32.checked_sub(size.checked_ilog2()?)?).filter(|l| l.size() == size);
    let (small_lod, large_lod) = (lod(small)?, lod(large)?);
    let (width, height) = if aw >= ah { (large, large / aw) } else { (large / ah, large) };
    let mem_required = (large_lod as u32..=small_lod as u32)
        .filter_map(Lod::from_index)
        .map(|l| level_host_texels(l, aspect) * format.bytes_per_texel())
        .sum();
    Some((Header { width, height, small_lod, large_lod, aspect, format, mem_required }, at))
}

/// Parses a whole file: header, decompression table and texel data in host
/// (little-endian) order.
pub fn load(file: &[u8]) -> Option<(Header, Table, Vec<u8>)> {
    let (header, mut at) = parse_header(file)?;
    let mut word = || {
        let v = u16::from_be_bytes([*file.get(at)?, *file.get(at + 1)?]);
        at += 2;
        Some(v)
    };
    let table = if header.format.uses_ncc() {
        let mut t = NccData::default();
        for y in &mut t.y {
            *y = word()? as u8;
        }
        for row in &mut t.i {
            for v in row {
                *v = (word()? & 0x1ff) as i16;
            }
        }
        for row in &mut t.q {
            for v in row {
                *v = (word()? & 0x1ff) as i16;
            }
        }
        Table::Ncc(t)
    } else if header.format.uses_palette() {
        let mut p = Box::new([0u32; 256]);
        for v in p.iter_mut() {
            *v = (word()? as u32) << 16 | word()? as u32;
        }
        Table::Palette(p)
    } else {
        Table::None
    };
    let size = header.mem_required as usize;
    let raw = file.get(at..at + size)?;
    let data = if header.format.bytes_per_texel() == 2 {
        raw.chunks_exact(2).flat_map(|w| [w[1], w[0]]).collect()
    } else {
        raw.to_vec()
    };
    Some((header, table, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_small_file() {
        let mut file = b"3df v1.0\nrgb565\nlod range: 1 2\naspect ratio: 2 1\n".to_vec();
        // 2x1 level then 1x1 level, big-endian.
        file.extend_from_slice(&[0xf8, 0x00, 0x07, 0xe0, 0x00, 0x1f]);
        let (header, table, data) = load(&file).expect("valid file");
        assert_eq!((header.width, header.height), (2, 1));
        assert_eq!(header.large_lod, Lod::L2);
        assert_eq!(header.small_lod, Lod::L1);
        assert_eq!(header.aspect, Aspect::A2x1);
        assert_eq!(header.mem_required, 6);
        assert_eq!(table, Table::None);
        assert_eq!(data, [0x00, 0xf8, 0xe0, 0x07, 0x1f, 0x00]);
    }
}
