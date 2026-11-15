//! Plain-data commands the core produces and the renderer consumes.

use crate::types::{Buffer, ClampMode, CmpFnc, FilterMode, MipMapMode};
use crate::vertex::DrawVertex;
use bytemuck::{Pod, Zeroable};

/// Identifies a decoded texture the renderer holds.
pub type TextureId = u32;

/// Per-draw state for the fixed-function shader. Layout is shared with
/// `shader.wgsl`; every field is a 16-byte vector so it matches WGSL's
/// uniform layout without padding rules.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct ShaderState {
    /// Packed color combine, alpha combine, TMU0 combine, TMU1 combine.
    pub combine: [u32; 4],
    /// Flag bits, alpha test (function | ref << 8), chroma key RGB, fog mode | depth mode << 12.
    pub flags: [u32; 4],
    /// Constant color, 0..=1.
    pub constant: [f32; 4],
    /// Fog color, 0..=1.
    pub fog_color: [f32; 4],
    /// 1/s range and 1/t range of TMU0 then TMU1 (Glide's 0..256 space to 0..1).
    pub tex_scale: [f32; 4],
    /// LOD bias of TMU0 and TMU1, depth bias, unused.
    pub bias: [f32; 4],
    /// Detail bias, scale and max of TMU0; unused.
    pub detail0: [f32; 4],
    /// Detail bias, scale and max of TMU1; unused.
    pub detail1: [f32; 4],
    /// Glide screen width and height, unused.
    pub screen: [f32; 4],
    /// The 64-entry fog table, four entries per word.
    pub fog_table: [u32; 16],
}

/// Bits of `ShaderState::flags[0]`.
pub mod flag {
    pub const TMU0: u32 = 1 << 0;
    pub const TMU1: u32 = 1 << 1;
    pub const CHROMA: u32 = 1 << 2;
    pub const ALPHA_CONTROLS_ITRGB: u32 = 1 << 3;
    /// The draw is an LFB write: the color comes from the LFB texture.
    pub const LFB: u32 = 1 << 4;
    /// LFB write through the pixel pipeline (fog, alpha test, chroma key).
    pub const LFB_PIPELINE: u32 = 1 << 5;
    /// LFB write carries its own depth.
    pub const LFB_DEPTH: u32 = 1 << 6;
}

/// Source and destination blend factors for color and alpha, Glide values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Blend {
    pub rgb_src: crate::types::BlendFactor,
    pub rgb_dst: crate::types::BlendFactor,
    pub alpha_src: crate::types::BlendFactor,
    pub alpha_dst: crate::types::BlendFactor,
}

/// The fixed state a GPU pipeline is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineKey {
    /// `None` when blending is off (ONE, ZERO).
    pub blend: Option<Blend>,
    /// `None` when the depth test is off.
    pub depth_compare: Option<CmpFnc>,
    pub depth_write: bool,
    pub write_rgb: bool,
    pub write_alpha: bool,
}

/// How a TMU's texture is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SamplerKey {
    pub min: FilterMode,
    pub mag: FilterMode,
    pub mipmap: MipMapMode,
    /// Linear blending between mipmap levels.
    pub lod_blend: bool,
    pub clamp_s: ClampMode,
    pub clamp_t: ClampMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureBinding {
    pub texture: TextureId,
    pub sampler: SamplerKey,
}

/// A pixel rectangle with an upper-left origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Triangles sharing one state, drawn into `buffer`.
#[derive(Debug, Clone)]
pub struct DrawBatch {
    pub buffer: Buffer,
    pub pipeline: PipelineKey,
    pub state: ShaderState,
    pub textures: [Option<TextureBinding>; 2],
    pub scissor: Rect,
    pub vertices: Vec<DrawVertex>,
}

#[derive(Debug, Clone)]
pub enum Command {
    Draw(DrawBatch),
    /// Clears `rect` of `buffer`; each part only when its write flag is set.
    Clear {
        buffer: Buffer,
        rect: Rect,
        color: [f32; 4],
        depth: f32,
        write_rgb: bool,
        write_alpha: bool,
        write_depth: bool,
    },
    /// Adds a texture: RGBA8 levels, largest first.
    CreateTexture {
        id: TextureId,
        levels: Vec<(u32, u32, Vec<u8>)>,
    },
    DestroyTexture(TextureId),
    /// Pixels written through the LFB: RGBA8 rows (top first) of `rect`,
    /// a mask of written pixels and optional 16-bit depth values. With
    /// `pipeline` set the pixels are blended and depth tested like a draw
    /// using `pipeline_key` and `state`.
    LfbWrite {
        buffer: Buffer,
        rect: Rect,
        rgba: Vec<u8>,
        mask: Vec<u8>,
        depth: Option<Vec<u16>>,
        pipeline: Option<(PipelineKey, ShaderState)>,
        scissor: Rect,
    },
    /// Shows the back buffer: buffers are exchanged.
    Swap {
        interval: u32,
    },
}
