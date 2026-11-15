//! Linear frame buffer pixel formats.
//!
//! Read locks expose the color buffer as 565 and the aux buffer as 16-bit
//! depth. Write locks hand out a buffer pre-filled with a sentinel; on unlock
//! the pixels that differ from it are the ones the game wrote, as OpenGLide
//! does, so no read-back is needed for a write.

use crate::types::{ColorFormat, LfbWriteMode};

/// Unwritten 16-bit pixel: almost black, unlikely to be drawn on purpose.
pub const SENTINEL16: u16 = 0x0821;
/// Unwritten 32-bit pixel.
pub const SENTINEL32: u32 = 0x5a01_0203;

pub fn rgba_to_565([r, g, b, _]: [u8; 4]) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

fn expand5(v: u32) -> u8 {
    ((v << 3) | (v >> 2)) as u8
}

fn expand6(v: u32) -> u8 {
    ((v << 2) | (v >> 4)) as u8
}

/// Write state that shapes how locked pixels are interpreted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriteFormat {
    pub mode: LfbWriteMode,
    pub color_format: ColorFormat,
    pub swizzle_bytes: bool,
    pub swap_words: bool,
    pub constant_alpha: u8,
    pub constant_depth: u16,
}

/// A pixel decoded from a write: color, alpha and an optional depth value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WrittenPixel {
    pub rgba: [u8; 4],
    pub depth: Option<u16>,
}

impl WriteFormat {
    fn color16(&self, v: u32) -> [u8; 4] {
        match self.mode {
            LfbWriteMode::W555 | LfbWriteMode::W555Depth => {
                [expand5((v >> 10) & 0x1f), expand5((v >> 5) & 0x1f), expand5(v & 0x1f), self.constant_alpha]
            }
            LfbWriteMode::W1555 | LfbWriteMode::W1555Depth => [
                expand5((v >> 10) & 0x1f),
                expand5((v >> 5) & 0x1f),
                expand5(v & 0x1f),
                if v & 0x8000 != 0 { 255 } else { 0 },
            ],
            _ => [expand5(v >> 11), expand6((v >> 5) & 0x3f), expand5(v & 0x1f), self.constant_alpha],
        }
    }

    fn word32(&self, mut v: u32) -> u32 {
        if self.swap_words {
            v = v.rotate_left(16);
        }
        if self.swizzle_bytes {
            v = v.swap_bytes();
        }
        v
    }

    /// Decodes the pixels a game wrote into a sentinel-filled buffer of
    /// `width` x `height` with `stride` bytes per row. Rows are returned top
    /// first; unwritten pixels are `None`.
    pub fn decode(&self, data: &[u8], stride: usize, width: u32, height: u32) -> Vec<Option<WrittenPixel>> {
        let bpp = self.mode.bytes_per_pixel() as usize;
        let mut out = Vec::with_capacity((width * height) as usize);
        for y in 0..height as usize {
            for x in 0..width as usize {
                let at = y * stride + x * bpp;
                let Some(raw) = data.get(at..at + bpp) else {
                    out.push(None);
                    continue;
                };
                out.push(if bpp == 2 {
                    let v = u16::from_le_bytes([raw[0], raw[1]]);
                    (v != SENTINEL16).then(|| self.pixel16(v as u32))
                } else {
                    let v = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
                    (v != SENTINEL32).then(|| self.pixel32(v))
                });
            }
        }
        out
    }

    fn pixel16(&self, v: u32) -> WrittenPixel {
        if self.mode == LfbWriteMode::Za16 {
            return WrittenPixel { rgba: [0, 0, 0, self.constant_alpha], depth: Some(v as u16) };
        }
        WrittenPixel { rgba: self.color16(v), depth: None }
    }

    fn pixel32(&self, v: u32) -> WrittenPixel {
        let v = self.word32(v);
        match self.mode {
            LfbWriteMode::W565Depth | LfbWriteMode::W555Depth | LfbWriteMode::W1555Depth => {
                WrittenPixel { rgba: self.color16(v & 0xffff), depth: Some((v >> 16) as u16) }
            }
            LfbWriteMode::W888 => {
                let [r, g, b, _] = self.color_format.unpack(v);
                WrittenPixel { rgba: [r, g, b, self.constant_alpha], depth: None }
            }
            _ => WrittenPixel { rgba: self.color_format.unpack(v), depth: None },
        }
    }
}

/// Fills a write-lock buffer with the sentinel for `mode`.
pub fn fill_sentinel(data: &mut [u8], mode: LfbWriteMode) {
    if mode.bytes_per_pixel() == 2 {
        for chunk in data.chunks_exact_mut(2) {
            chunk.copy_from_slice(&SENTINEL16.to_le_bytes());
        }
    } else {
        for chunk in data.chunks_exact_mut(4) {
            chunk.copy_from_slice(&SENTINEL32.to_le_bytes());
        }
    }
}

/// Decodes a `grLfbWriteRegion` source pixel of format `src_format`.
pub fn region_pixel(src_format: u32, raw: &[u8], format: &WriteFormat) -> Option<WrittenPixel> {
    let mode = LfbWriteMode::try_from(src_format).ok()?;
    let f = WriteFormat { mode, ..*format };
    Some(if mode.bytes_per_pixel() == 2 {
        f.pixel16(u16::from_le_bytes([*raw.first()?, *raw.get(1)?]) as u32)
    } else {
        f.pixel32(u32::from_le_bytes(raw.get(..4)?.try_into().ok()?))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(mode: LfbWriteMode) -> WriteFormat {
        WriteFormat {
            mode,
            color_format: ColorFormat::Argb,
            swizzle_bytes: false,
            swap_words: false,
            constant_alpha: 0xff,
            constant_depth: 0,
        }
    }

    #[test]
    fn sentinel_marks_unwritten_pixels() {
        let mut buf = vec![0u8; 8];
        fill_sentinel(&mut buf, LfbWriteMode::W565);
        buf[2..4].copy_from_slice(&0xf800u16.to_le_bytes());
        let px = format(LfbWriteMode::W565).decode(&buf, 4, 2, 2);
        assert_eq!(px[0], None);
        assert_eq!(px[1], Some(WrittenPixel { rgba: [255, 0, 0, 255], depth: None }));
        assert_eq!(px[2..], [None, None]);
    }

    #[test]
    fn decodes_32bit_modes() {
        let f = format(LfbWriteMode::W8888);
        assert_eq!(f.pixel32(0x80112233).rgba, [0x11, 0x22, 0x33, 0x80]);
        let f = format(LfbWriteMode::W565Depth);
        let p = f.pixel32(0x1234_001f);
        assert_eq!(p.rgba, [0, 0, 255, 255]);
        assert_eq!(p.depth, Some(0x1234));
    }

    #[test]
    fn packs_565() {
        assert_eq!(rgba_to_565([255, 0, 0, 255]), 0xf800);
        assert_eq!(rgba_to_565([0, 255, 0, 0]), 0x07e0);
    }
}
