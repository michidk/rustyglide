//! `GrVertex` and the vertex the renderer draws.

use bytemuck::{Pod, Zeroable};

/// Per-TMU texture coordinates of a `GrVertex`.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct GrTmuVertex {
    /// s/w, scaled so that 256 spans the texture's long side.
    pub sow: f32,
    /// t/w, scaled likewise.
    pub tow: f32,
    /// 1/w for this TMU.
    pub oow: f32,
}

/// Glide 2's `GrVertex` with two TMUs: 60 bytes, the layout games pass by pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct GrVertex {
    pub x: f32,
    pub y: f32,
    /// Ignored by the hardware.
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// 65535/z, for the Z-buffer.
    pub ooz: f32,
    pub a: f32,
    /// 1/w, for the W-buffer, fog and texturing.
    pub oow: f32,
    pub tmuvtx: [GrTmuVertex; 2],
}

const _: () = assert!(std::mem::size_of::<GrVertex>() == 60);

/// The vertex format of the renderer: screen position with an upper-left
/// origin, Glide depth values, color in 0..=255 and per-TMU s/w, t/w, 1/w.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Pod, Zeroable)]
pub struct DrawVertex {
    pub pos: [f32; 2],
    pub ooz: f32,
    pub oow: f32,
    pub color: [f32; 4],
    pub tex0: [f32; 3],
    pub tex1: [f32; 3],
}

/// Removes the snapping bias games add to coordinates (`x + 3<<18` puts the
/// 1/16-pixel grid in the float's low mantissa bits).
pub fn unsnap(v: f32) -> f32 {
    const BIAS: f32 = (3 << 18) as f32;
    if v >= (1 << 18) as f32 { v - BIAS } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsnap_removes_bias_only_when_present() {
        assert_eq!(unsnap(786432.0 + 12.5), 12.5);
        assert_eq!(unsnap(12.5), 12.5);
    }
}
