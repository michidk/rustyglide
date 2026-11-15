//! Glide semantics without a GPU.
//!
//! [`Context`] holds the complete Glide 2 state, texture memory and LFB
//! buffers. Drawing calls become [`Command`]s that a renderer executes; the
//! core never touches a GPU, window or operating system type.

pub mod combine;
pub mod command;
pub mod fog;
pub mod gu3df;
pub mod gutex;
pub mod lfb;
pub mod texture;
pub mod types;
pub mod vertex;

mod clip;
mod snapshot;
mod texcache;

use combine::{Combine, TexCombine};
pub use command::*;
use fog::FogTable;
use lfb::WriteFormat;
use texture::{NccData, Palette, TexInfo, TmuMemory};
use types::*;
pub use vertex::{DrawVertex, GrTmuVertex, GrVertex};

/// Settings that are not part of the Glide API.
#[derive(Debug, Clone)]
pub struct Config {
    /// TMUs reported to the game, 1 to 2.
    pub tmus: u32,
    /// Texture memory per TMU in bytes.
    pub tmu_memory: u32,
    /// Frame buffer memory reported to the game, in MB.
    pub fb_memory_mb: u32,
    /// Replace the game's filter modes (for example always bilinear).
    pub force_filter: Option<FilterMode>,
    /// Ignore mipmapping requests.
    pub disable_mipmaps: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { tmus: 1, tmu_memory: 4 << 20, fb_memory_mb: 4, force_filter: None, disable_mipmaps: false }
    }
}

/// The source a TMU samples from (`grTexSource`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TexSource {
    pub address: u32,
    pub even_odd: u32,
    pub info: TexInfo,
}

#[derive(Clone)]
struct TmuState {
    memory: TmuMemory,
    source: Option<TexSource>,
    multibase: bool,
    multibase_address: [u32; 4],
    combine: TexCombine,
    min_filter: FilterMode,
    mag_filter: FilterMode,
    mipmap: MipMapMode,
    lod_blend: bool,
    clamp_s: ClampMode,
    clamp_t: ClampMode,
    lod_bias: f32,
    detail: (i32, u8, f32),
    palette: Box<Palette>,
    palette_hash: u64,
    ncc: [NccData; 2],
    ncc_hash: [u64; 2],
    ncc_select: NccTable,
}

impl TmuState {
    fn new(memory: u32) -> Self {
        Self {
            memory: TmuMemory::new(memory as usize),
            source: None,
            multibase: false,
            multibase_address: [0; 4],
            combine: combine::DEFAULT_TEX,
            min_filter: FilterMode::PointSampled,
            mag_filter: FilterMode::PointSampled,
            mipmap: MipMapMode::Disable,
            lod_blend: false,
            clamp_s: ClampMode::Clamp,
            clamp_t: ClampMode::Clamp,
            lod_bias: 0.0,
            detail: (0, 1, 1.0),
            palette: Box::new([0; 256]),
            palette_hash: 0,
            ncc: [NccData::default(); 2],
            ncc_hash: [0; 2],
            ncc_select: NccTable::Ncc0,
        }
    }
}

/// An active LFB lock.
struct Lock {
    buffer: Buffer,
    origin: Origin,
    pipeline: bool,
    format: WriteFormat,
    data: Vec<u8>,
    stride: usize,
}

/// What `grLfbLock` returns.
#[derive(Debug, Clone, Copy)]
pub struct LockInfo {
    pub data: *mut u8,
    pub stride: u32,
    pub write_mode: LfbWriteMode,
    pub origin: Origin,
}

/// Pixels read back from the renderer for an LFB read, rows top first.
pub enum ReadPixels<'a> {
    /// RGBA8.
    Color(&'a [u8]),
    /// Depth in 0..=1.
    Depth(&'a [f32]),
}

/// The Glide state machine.
pub struct Context {
    config: Config,
    open: bool,
    width: u32,
    height: u32,
    origin: Origin,
    color_format: ColorFormat,
    render_buffer: Buffer,
    color: Combine,
    alpha: Combine,
    delta0: bool,
    alpha_controls_itrgb: bool,
    constant: u32,
    blend: Blend,
    alpha_test: CmpFnc,
    alpha_ref: u8,
    chroma: ChromakeyMode,
    chroma_value: u32,
    clip: (u32, u32, u32, u32),
    write_rgb: bool,
    write_alpha: bool,
    cull: CullMode,
    depth_mode: DepthBufferMode,
    depth_func: CmpFnc,
    depth_mask: bool,
    depth_bias: i16,
    dither: DitherMode,
    fog_mode: FogMode,
    fog_color: u32,
    fog_table: FogTable,
    gamma: f32,
    stw_hint: u32,
    tmus: Vec<TmuState>,
    lfb_format: WriteFormat,
    read_lock: Option<Lock>,
    write_lock: Option<Lock>,
    commands: Vec<Command>,
    textures: texcache::TextureCache,
    /// Batch parameters valid until state changes.
    batch: Option<BatchParams>,
    pub(crate) gutex: gutex::Manager,
    tris_processed: u32,
    tris_drawn: u32,
}

#[derive(Clone, PartialEq)]
struct BatchParams {
    buffer: Buffer,
    pipeline: PipelineKey,
    state: ShaderState,
    textures: [Option<TextureBinding>; 2],
    scissor: Rect,
}

const DEFAULT_BLEND: Blend =
    Blend { rgb_src: BlendFactor::One, rgb_dst: BlendFactor::Zero, alpha_src: BlendFactor::One, alpha_dst: BlendFactor::Zero };

impl Context {
    pub fn new(config: Config) -> Self {
        let tmus = (0..config.tmus.clamp(1, 2)).map(|_| TmuState::new(config.tmu_memory)).collect();
        Self {
            open: false,
            width: 640,
            height: 480,
            origin: Origin::UpperLeft,
            color_format: ColorFormat::Argb,
            render_buffer: Buffer::Back,
            color: combine::DEFAULT_COLOR,
            alpha: combine::DEFAULT_ALPHA,
            delta0: false,
            alpha_controls_itrgb: false,
            constant: !0,
            blend: DEFAULT_BLEND,
            alpha_test: CmpFnc::Always,
            alpha_ref: 0,
            chroma: ChromakeyMode::Disable,
            chroma_value: 0,
            clip: (0, 0, 640, 480),
            write_rgb: true,
            write_alpha: false,
            cull: CullMode::Disable,
            depth_mode: DepthBufferMode::Disable,
            depth_func: CmpFnc::Less,
            depth_mask: false,
            depth_bias: 0,
            dither: DitherMode::D4x4,
            fog_mode: FogMode(0),
            fog_color: 0,
            fog_table: [0; types::FOG_TABLE_SIZE],
            gamma: 1.0,
            stw_hint: 0,
            tmus,
            lfb_format: WriteFormat {
                mode: LfbWriteMode::W565,
                color_format: ColorFormat::Argb,
                swizzle_bytes: false,
                swap_words: false,
                constant_alpha: 0xff,
                constant_depth: 0,
            },
            read_lock: None,
            write_lock: None,
            commands: Vec::new(),
            textures: texcache::TextureCache::default(),
            batch: None,
            gutex: gutex::Manager::default(),
            tris_processed: 0,
            tris_drawn: 0,
            config,
        }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Takes the commands produced since the last call.
    pub fn take_commands(&mut self) -> Vec<Command> {
        std::mem::take(&mut self.commands)
    }

    fn dirty(&mut self) {
        self.batch = None;
    }

    fn tmu(&mut self, tmu: Tmu) -> Option<&mut TmuState> {
        self.dirty();
        self.tmus.get_mut(tmu.index())
    }

    // ---- Initialization and the SST ------------------------------------

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn screen_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn origin(&self) -> Origin {
        self.origin
    }

    pub fn gamma(&self) -> f32 {
        self.gamma
    }

    /// The core part of `grSstWinOpen`: size and formats, then Glide's reset state.
    pub fn win_open(&mut self, width: u32, height: u32, color_format: ColorFormat, origin: Origin) {
        let config = self.config.clone();
        let memory: Vec<TmuMemory> = self.tmus.iter().map(|t| t.memory.clone()).collect();
        let destroyed = self.textures.clear();
        *self = Self::new(config);
        for (t, m) in self.tmus.iter_mut().zip(memory) {
            t.memory = m;
        }
        self.commands.extend(destroyed.into_iter().map(Command::DestroyTexture));
        self.open = true;
        self.width = width;
        self.height = height;
        self.color_format = color_format;
        self.lfb_format.color_format = color_format;
        self.origin = if origin == Origin::Any { Origin::UpperLeft } else { origin };
        self.clip = (0, 0, width, height);
    }

    pub fn win_close(&mut self) {
        self.open = false;
        self.read_lock = None;
        self.write_lock = None;
    }

    pub fn sst_origin(&mut self, origin: Origin) {
        self.dirty();
        self.origin = if origin == Origin::Any { Origin::UpperLeft } else { origin };
    }

    pub fn hints(&mut self, hint: Hint, mask: u32) {
        if hint == Hint::StwHint {
            self.stw_hint = mask;
        }
    }

    pub fn tri_stats(&self) -> (u32, u32) {
        (self.tris_processed, self.tris_drawn)
    }

    pub fn reset_tri_stats(&mut self) {
        self.tris_processed = 0;
        self.tris_drawn = 0;
    }

    // ---- Combiners --------------------------------------------------------

    pub fn color_combine(&mut self, combine: Combine) {
        self.dirty();
        self.color = combine;
        self.delta0 = false;
    }

    pub fn alpha_combine(&mut self, combine: Combine) {
        self.dirty();
        self.alpha = combine;
    }

    pub fn gu_color_combine_function(&mut self, fnc: ColorCombineFnc) {
        let (combine, delta0) = combine::gu_color_combine(fnc);
        self.color_combine(combine);
        self.delta0 = delta0;
    }

    pub fn gu_alpha_source(&mut self, source: AlphaSource) {
        self.alpha_combine(combine::gu_alpha_source(source));
    }

    pub fn alpha_controls_itrgb_lighting(&mut self, enable: bool) {
        self.dirty();
        self.alpha_controls_itrgb = enable;
    }

    pub fn constant_color(&mut self, color: u32) {
        self.dirty();
        self.constant = color;
    }

    /// `grConstantColorValue4`: components in 0..=255.
    pub fn constant_color4(&mut self, a: f32, r: f32, g: f32, b: f32) {
        let q = |v: f32| v.clamp(0.0, 255.0) as u32;
        let argb = q(a) << 24 | q(r) << 16 | q(g) << 8 | q(b);
        // Store in the window's color format so unpacking stays uniform.
        let [r, g, b, a] = ColorFormat::Argb.unpack(argb);
        self.constant = pack(self.color_format, [r, g, b, a]);
        self.dirty();
    }

    pub fn tex_combine(&mut self, tmu: Tmu, combine: TexCombine) {
        if let Some(t) = self.tmu(tmu) {
            t.combine = combine;
        }
    }

    pub fn gu_tex_combine_function(&mut self, tmu: Tmu, fnc: TextureCombineFnc) {
        self.tex_combine(tmu, combine::gu_tex_combine(fnc));
    }

    // ---- Pixel operations ------------------------------------------------

    pub fn alpha_blend_function(&mut self, blend: Blend) {
        self.dirty();
        self.blend = blend;
    }

    pub fn alpha_test_function(&mut self, function: CmpFnc) {
        self.dirty();
        self.alpha_test = function;
    }

    pub fn alpha_test_reference(&mut self, value: u8) {
        self.dirty();
        self.alpha_ref = value;
    }

    pub fn chromakey_mode(&mut self, mode: ChromakeyMode) {
        self.dirty();
        self.chroma = mode;
    }

    pub fn chromakey_value(&mut self, value: u32) {
        self.dirty();
        self.chroma_value = value;
    }

    pub fn clip_window(&mut self, min_x: u32, min_y: u32, max_x: u32, max_y: u32) {
        self.dirty();
        self.clip = (min_x.min(max_x), min_y.min(max_y), max_x, max_y);
    }

    pub fn color_mask(&mut self, rgb: bool, alpha: bool) {
        self.dirty();
        self.write_rgb = rgb;
        self.write_alpha = alpha;
    }

    pub fn cull_mode(&mut self, mode: CullMode) {
        self.cull = mode;
    }

    pub fn depth_bias_level(&mut self, level: i16) {
        self.dirty();
        self.depth_bias = level;
    }

    pub fn depth_buffer_function(&mut self, function: CmpFnc) {
        self.dirty();
        self.depth_func = function;
    }

    pub fn depth_buffer_mode(&mut self, mode: DepthBufferMode) {
        self.dirty();
        self.depth_mode = mode;
    }

    pub fn depth_mask(&mut self, enable: bool) {
        self.dirty();
        self.depth_mask = enable;
    }

    pub fn dither_mode(&mut self, mode: DitherMode) {
        // Rendering keeps 8 bits per channel; the mode is state only.
        self.dither = mode;
    }

    pub fn disable_all_effects(&mut self) {
        self.dirty();
        self.blend = DEFAULT_BLEND;
        self.alpha_test = CmpFnc::Always;
        self.chroma = ChromakeyMode::Disable;
        self.depth_mode = DepthBufferMode::Disable;
        self.fog_mode = FogMode(0);
    }

    pub fn fog_color(&mut self, color: u32) {
        self.dirty();
        self.fog_color = color;
    }

    pub fn fog_mode(&mut self, mode: FogMode) {
        self.dirty();
        self.fog_mode = mode;
    }

    pub fn fog_table(&mut self, table: &FogTable) {
        self.dirty();
        self.fog_table = *table;
    }

    pub fn gamma_correction(&mut self, gamma: f32) {
        if gamma.is_finite() && gamma > 0.0 {
            self.gamma = gamma;
        }
    }

    pub fn render_buffer(&mut self, buffer: Buffer) {
        self.dirty();
        self.render_buffer = if buffer == Buffer::Front { Buffer::Front } else { Buffer::Back };
    }

    // ---- Textures ---------------------------------------------------------

    pub fn tex_min_address(&self, _tmu: Tmu) -> u32 {
        0
    }

    pub fn tex_max_address(&self, tmu: Tmu) -> u32 {
        self.tmus.get(tmu.index()).map_or(0, |t| t.memory.len().saturating_sub(8))
    }

    pub fn tex_source(&mut self, tmu: Tmu, address: u32, even_odd: u32, info: TexInfo) {
        if let Some(t) = self.tmu(tmu) {
            t.source = Some(TexSource { address, even_odd, info });
        }
    }

    pub fn tex_clamp_mode(&mut self, tmu: Tmu, s: ClampMode, t: ClampMode) {
        if let Some(state) = self.tmu(tmu) {
            state.clamp_s = s;
            state.clamp_t = t;
        }
    }

    pub fn tex_filter_mode(&mut self, tmu: Tmu, min: FilterMode, mag: FilterMode) {
        if let Some(t) = self.tmu(tmu) {
            t.min_filter = min;
            t.mag_filter = mag;
        }
    }

    pub fn tex_mipmap_mode(&mut self, tmu: Tmu, mode: MipMapMode, lod_blend: bool) {
        if let Some(t) = self.tmu(tmu) {
            t.mipmap = mode;
            t.lod_blend = lod_blend;
        }
    }

    pub fn tex_lod_bias(&mut self, tmu: Tmu, bias: f32) {
        if let Some(t) = self.tmu(tmu) {
            t.lod_bias = bias;
        }
    }

    pub fn tex_detail_control(&mut self, tmu: Tmu, lod_bias: i32, scale: u8, max: f32) {
        if let Some(t) = self.tmu(tmu) {
            t.detail = (lod_bias, scale, max);
        }
    }

    pub fn tex_multibase(&mut self, tmu: Tmu, enable: bool) {
        if let Some(t) = self.tmu(tmu) {
            t.multibase = enable;
        }
    }

    pub fn tex_multibase_address(&mut self, tmu: Tmu, range: TexBaseRange, address: u32) {
        if let Some(t) = self.tmu(tmu) {
            t.multibase_address[range as usize] = address;
        }
    }

    pub fn tex_ncc_table(&mut self, tmu: Tmu, table: NccTable) {
        if let Some(t) = self.tmu(tmu) {
            t.ncc_select = table;
        }
    }

    fn texture_written(&mut self, tmu: Tmu, range: std::ops::Range<u32>) {
        self.dirty();
        let destroyed = self.textures.invalidate(tmu, range);
        self.commands.extend(destroyed.into_iter().map(Command::DestroyTexture));
    }

    /// `grTexDownloadMipMapLevelPartial`: rows `start..=end` of one level.
    #[allow(clippy::too_many_arguments)]
    pub fn tex_download_level_partial(
        &mut self,
        tmu: Tmu,
        address: u32,
        lod: Lod,
        large: Lod,
        aspect: Aspect,
        format: TextureFormat,
        even_odd: u32,
        data: &[u8],
        rows: std::ops::RangeInclusive<u32>,
    ) {
        if !texture::level_in_mask(lod, even_odd) {
            return;
        }
        let (w, h) = texture::level_dims(lod, aspect);
        let row_bytes = (w * format.bytes_per_texel()) as usize;
        let first = (*rows.start()).min(h);
        let last = (*rows.end()).min(h.saturating_sub(1));
        if first > last {
            return;
        }
        let level = address + texture::level_offset(lod, large, aspect, format, even_odd);
        let start = level + first * row_bytes as u32;
        let len = ((last - first + 1) as usize * row_bytes).min(data.len());
        let Some(t) = self.tmus.get_mut(tmu.index()) else { return };
        let written = t.memory.write(start, &data[..len]);
        self.texture_written(tmu, written);
    }

    /// `grTexDownloadMipMapLevel`.
    #[allow(clippy::too_many_arguments)]
    pub fn tex_download_level(
        &mut self,
        tmu: Tmu,
        address: u32,
        lod: Lod,
        large: Lod,
        aspect: Aspect,
        format: TextureFormat,
        even_odd: u32,
        data: &[u8],
    ) {
        self.tex_download_level_partial(tmu, address, lod, large, aspect, format, even_odd, data, 0..=u32::MAX);
    }

    /// `grTexDownloadMipMap`: every level of `info`, read consecutively from `data`.
    pub fn tex_download_mipmap(&mut self, tmu: Tmu, address: u32, even_odd: u32, info: TexInfo, data: &[u8]) {
        let mut offset = 0usize;
        for lod in (info.large_lod as u32..=info.small_lod as u32).filter_map(Lod::from_index) {
            let size = (texture::level_host_texels(lod, info.aspect) * info.format.bytes_per_texel()) as usize;
            let level = data.get(offset..(offset + size).min(data.len())).unwrap_or(&[]);
            self.tex_download_level(tmu, address, lod, info.large_lod, info.aspect, info.format, even_odd, level);
            offset += size;
        }
    }

    /// `grTexDownloadTablePartial` for palettes: entries `start..=end`.
    pub fn tex_download_palette(&mut self, tmu: Tmu, entries: &[u32], start: usize) {
        let Some(t) = self.tmu(tmu) else { return };
        for (slot, value) in t.palette.iter_mut().skip(start).zip(entries) {
            *slot = value & 0x00ff_ffff;
        }
        t.palette_hash = hash(&t.palette[..]);
    }

    /// `grTexDownloadTable` for an NCC table.
    pub fn tex_download_ncc(&mut self, tmu: Tmu, which: NccTable, table: &NccData) {
        let Some(t) = self.tmu(tmu) else { return };
        t.ncc[which as usize] = *table;
        t.ncc_hash[which as usize] = hash(table);
    }

    // ---- Buffers and drawing ----------------------------------------------

    pub fn buffer_clear(&mut self, color: u32, alpha: u8, depth: u16) {
        let [r, g, b, _] = self.color_format.unpack(color);
        self.commands.push(Command::Clear {
            buffer: self.render_buffer,
            rect: self.scissor(),
            color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, alpha as f32 / 255.0],
            depth: depth as f32 / 65535.0,
            write_rgb: self.write_rgb,
            write_alpha: self.write_alpha,
            write_depth: self.depth_mask,
        });
    }

    pub fn buffer_swap(&mut self, interval: u32) {
        self.commands.push(Command::Swap { interval });
    }

    fn scissor(&self) -> Rect {
        let (x0, y0, x1, y1) = self.clip;
        let x1 = x1.min(self.width);
        let y1 = y1.min(self.height);
        let (top, bottom) = match self.origin {
            Origin::LowerLeft => (self.height.saturating_sub(y1), self.height.saturating_sub(y0)),
            _ => (y0, y1),
        };
        Rect { x: x0.min(x1), y: top, width: x1.saturating_sub(x0), height: bottom.saturating_sub(top) }
    }

    fn convert(&self, v: &GrVertex) -> DrawVertex {
        let y = vertex::unsnap(v.y);
        let y = if self.origin == Origin::LowerLeft { self.height as f32 - y } else { y };
        let t0 = v.tmuvtx[0];
        let oow0 = if self.stw_hint & stw_hint::W_DIFF_TMU0 != 0 { t0.oow } else { v.oow };
        let t1 = if self.stw_hint & stw_hint::ST_DIFF_TMU1 != 0 { v.tmuvtx[1] } else { t0 };
        let oow1 = if self.stw_hint & stw_hint::W_DIFF_TMU1 != 0 { v.tmuvtx[1].oow } else { oow0 };
        DrawVertex {
            pos: [vertex::unsnap(v.x), y],
            ooz: v.ooz,
            oow: v.oow,
            color: [v.r, v.g, v.b, v.a],
            tex0: [t0.sow, t0.tow, oow0],
            tex1: [t1.sow, t1.tow, oow1],
        }
    }

    /// Signed area in Glide's coordinates; the SDK's culling test.
    fn culled(&self, a: &GrVertex, b: &GrVertex, c: &GrVertex) -> bool {
        let area = (a.x - b.x) * (b.y - c.y) - (b.x - c.x) * (a.y - b.y);
        if area == 0.0 || area.is_nan() {
            return true;
        }
        match self.cull {
            CullMode::Disable => false,
            CullMode::Negative => area < 0.0,
            CullMode::Positive => area > 0.0,
        }
    }

    pub fn draw_triangle(&mut self, a: &GrVertex, b: &GrVertex, c: &GrVertex) {
        self.tris_processed += 1;
        if self.culled(a, b, c) {
            return;
        }
        self.tris_drawn += 1;
        let mut v = [self.convert(a), self.convert(b), self.convert(c)];
        if self.delta0 {
            let color = v[0].color;
            for v in &mut v[1..] {
                v.color[..3].copy_from_slice(&color[..3]);
            }
        }
        self.push(&v);
    }

    /// Draws a one-pixel-wide line as two triangles.
    pub fn draw_line(&mut self, a: &GrVertex, b: &GrVertex) {
        let (a, mut b) = (self.convert(a), self.convert(b));
        if self.delta0 {
            b.color = a.color;
        }
        let dx = b.pos[0] - a.pos[0];
        let dy = b.pos[1] - a.pos[1];
        let (ox, oy) = if dx.abs() >= dy.abs() { (0.0, 0.5) } else { (0.5, 0.0) };
        let shift = |v: &DrawVertex, s: f32| {
            let mut v = *v;
            v.pos[0] += ox * s;
            v.pos[1] += oy * s;
            v
        };
        let quad = [shift(&a, -1.0), shift(&b, -1.0), shift(&b, 1.0), shift(&a, -1.0), shift(&b, 1.0), shift(&a, 1.0)];
        self.push(&quad);
    }

    /// Draws a point as the pixel it falls in.
    pub fn draw_point(&mut self, p: &GrVertex) {
        let v = self.convert(p);
        let (x, y) = (v.pos[0].floor(), v.pos[1].floor());
        let at = |dx: f32, dy: f32| DrawVertex { pos: [x + dx, y + dy], ..v };
        self.push(&[at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)]);
    }

    /// `grDrawPolygon` / `grDrawPlanarPolygon`: a fan over `indices` into `vertices`.
    pub fn draw_polygon(&mut self, indices: &[i32], vertices: &[GrVertex]) {
        let get = |i: i32| vertices.get(usize::try_from(i).ok()?);
        let Some(first) = indices.first().and_then(|&i| get(i)) else { return };
        for pair in indices[1..].windows(2) {
            if let (Some(b), Some(c)) = (get(pair[0]), get(pair[1])) {
                self.draw_triangle(first, b, c);
            }
        }
    }

    /// `grDrawPolygonVertexList` / `grDrawPlanarPolygonVertexList`.
    pub fn draw_polygon_list(&mut self, vertices: &[GrVertex]) {
        if let Some((first, rest)) = vertices.split_first() {
            for pair in rest.windows(2) {
                self.draw_triangle(first, &pair[0], &pair[1]);
            }
        }
    }

    /// `guDrawTriangleWithClip`: clips to the clip window before drawing.
    pub fn draw_triangle_clipped(&mut self, a: &GrVertex, b: &GrVertex, c: &GrVertex) {
        let (x0, y0, x1, y1) = self.clip;
        let polygon = clip::clip_polygon(&[*a, *b, *c], [x0 as f32, y0 as f32, x1 as f32, y1 as f32]);
        self.draw_polygon_list(&polygon);
    }

    /// `guDrawPolygonVertexListWithClip`.
    pub fn draw_polygon_list_clipped(&mut self, vertices: &[GrVertex]) {
        let (x0, y0, x1, y1) = self.clip;
        let polygon = clip::clip_polygon(vertices, [x0 as f32, y0 as f32, x1 as f32, y1 as f32]);
        self.draw_polygon_list(&polygon);
    }

    fn push(&mut self, vertices: &[DrawVertex]) {
        if self.batch.is_none() {
            let params = self.batch_params();
            self.batch = Some(params);
        }
        let params = self.batch.as_ref().expect("batch parameters were just computed");
        if let Some(Command::Draw(last)) = self.commands.last_mut()
            && last.buffer == params.buffer
            && last.pipeline == params.pipeline
            && last.textures == params.textures
            && last.scissor == params.scissor
            && last.state == params.state
        {
            last.vertices.extend_from_slice(vertices);
            return;
        }
        let params = params.clone();
        self.commands.push(Command::Draw(DrawBatch {
            buffer: params.buffer,
            pipeline: params.pipeline,
            state: params.state,
            textures: params.textures,
            scissor: params.scissor,
            vertices: vertices.to_vec(),
        }));
    }

    fn pipeline_key(&self) -> PipelineKey {
        let blend = (self.blend != DEFAULT_BLEND).then_some(self.blend);
        let depth_on = self.depth_mode != DepthBufferMode::Disable;
        PipelineKey {
            blend,
            depth_compare: depth_on.then_some(self.depth_func),
            depth_write: depth_on && self.depth_mask,
            write_rgb: self.write_rgb,
            write_alpha: self.write_alpha,
        }
    }

    fn unpack_f(&self, color: u32) -> [f32; 4] {
        self.color_format.unpack(color).map(|c| c as f32 / 255.0)
    }

    fn shader_state(&self, textures: &[Option<TextureBinding>; 2]) -> ShaderState {
        let mut flags = 0;
        if textures[0].is_some() {
            flags |= flag::TMU0;
        }
        if textures[1].is_some() {
            flags |= flag::TMU1;
        }
        if self.chroma == ChromakeyMode::Enable {
            flags |= flag::CHROMA;
        }
        if self.alpha_controls_itrgb {
            flags |= flag::ALPHA_CONTROLS_ITRGB;
        }
        let [cr, cg, cb, _] = self.color_format.unpack(self.chroma_value);
        let tex_combine = |i: usize| self.tmus.get(i).map_or(combine::DEFAULT_TEX.pack(), |t| t.combine.pack());
        let mut tex_scale = [1.0 / 256.0; 4];
        for (i, t) in self.tmus.iter().enumerate() {
            if let Some(source) = t.source {
                let (w, h) = texture::level_dims(source.info.large_lod, source.info.aspect);
                let long = w.max(h) as f32;
                tex_scale[i * 2] = long / (256.0 * w as f32);
                tex_scale[i * 2 + 1] = long / (256.0 * h as f32);
            }
        }
        let detail = |i: usize| {
            self.tmus.get(i).map_or([0.0; 4], |t| [t.detail.0 as f32, t.detail.1 as f32, t.detail.2, 0.0])
        };
        let lod_bias = |i: usize| self.tmus.get(i).map_or(0.0, |t| t.lod_bias);
        let mut fog_table = [0u32; 16];
        for (i, word) in fog_table.iter_mut().enumerate() {
            *word = u32::from_le_bytes(self.fog_table[i * 4..i * 4 + 4].try_into().expect("four entries"));
        }
        ShaderState {
            combine: [self.color.pack(), self.alpha.pack(), tex_combine(0), tex_combine(1)],
            flags: [
                flags,
                self.alpha_test as u32 | (self.alpha_ref as u32) << 8,
                (cr as u32) << 16 | (cg as u32) << 8 | cb as u32,
                self.fog_mode.0 | (self.depth_mode as u32) << 12,
            ],
            constant: self.unpack_f(self.constant),
            fog_color: self.unpack_f(self.fog_color),
            tex_scale,
            bias: [lod_bias(0), lod_bias(1), self.depth_bias as f32, 0.0],
            detail0: detail(0),
            detail1: detail(1),
            screen: [self.width as f32, self.height as f32, 0.0, 0.0],
            fog_table,
        }
    }

    fn binding(&mut self, tmu: usize) -> Option<TextureBinding> {
        let t = self.tmus.get(tmu)?;
        let source = t.source?;
        let table_hash = if source.info.format.uses_palette() {
            t.palette_hash
        } else if source.info.format.uses_ncc() {
            t.ncc_hash[t.ncc_select as usize]
        } else {
            0
        };
        let key = texcache::Key {
            tmu: tmu as u8,
            source,
            multibase: t.multibase.then_some(t.multibase_address),
            table_hash,
        };
        let force = self.config.force_filter;
        let mipmap = if self.config.disable_mipmaps { MipMapMode::Disable } else { t.mipmap };
        let sampler = SamplerKey {
            min: force.unwrap_or(t.min_filter),
            mag: force.unwrap_or(t.mag_filter),
            mipmap,
            lod_blend: t.lod_blend && mipmap != MipMapMode::Disable,
            clamp_s: t.clamp_s,
            clamp_t: t.clamp_t,
        };
        let (texture, created) = self.textures.get_or_insert(key, || {
            let tables = texture::DecodeTables { palette: &t.palette, ncc: &t.ncc[t.ncc_select as usize] };
            texcache::decode(&t.memory, &key, &tables)
        });
        if let Some((id, levels)) = created {
            self.commands.push(Command::CreateTexture { id, levels });
        }
        for id in self.textures.take_evicted() {
            self.commands.push(Command::DestroyTexture(id));
        }
        Some(TextureBinding { texture, sampler })
    }

    fn batch_params(&mut self) -> BatchParams {
        let needs_texture = self.color.uses_texture() || self.alpha.uses_texture() || self.alpha_controls_itrgb;
        let mut textures = [None, None];
        if needs_texture {
            let tmu0 = self.tmus[0].combine;
            if tmu0.uses_local() {
                textures[0] = self.binding(0);
            }
            if tmu0.uses_other() && self.tmus.len() > 1 && self.tmus[1].combine.uses_local() {
                textures[1] = self.binding(1);
            }
        }
        BatchParams {
            buffer: self.render_buffer,
            pipeline: self.pipeline_key(),
            state: self.shader_state(&textures),
            textures,
            scissor: self.scissor(),
        }
    }

    // ---- Linear frame buffer ----------------------------------------------

    pub fn lfb_constant_alpha(&mut self, alpha: u8) {
        self.lfb_format.constant_alpha = alpha;
    }

    pub fn lfb_constant_depth(&mut self, depth: u16) {
        self.lfb_format.constant_depth = depth;
    }

    pub fn lfb_write_color_format(&mut self, format: ColorFormat) {
        self.lfb_format.color_format = format;
    }

    pub fn lfb_write_color_swizzle(&mut self, swizzle_bytes: bool, swap_words: bool) {
        self.lfb_format.swizzle_bytes = swizzle_bytes;
        self.lfb_format.swap_words = swap_words;
    }

    /// Fills a read lock from pixels the renderer read back.
    pub fn lfb_lock_read(&mut self, buffer: Buffer, origin: Origin, pixels: ReadPixels<'_>) -> LockInfo {
        let origin = if origin == Origin::Any { Origin::UpperLeft } else { origin };
        let (w, h) = (self.width as usize, self.height as usize);
        let stride = w * 2;
        let mut data = vec![0u8; stride * h];
        for y in 0..h {
            let src_row = if origin == Origin::LowerLeft { h - 1 - y } else { y };
            for x in 0..w {
                let i = src_row * w + x;
                let v = match pixels {
                    ReadPixels::Color(rgba) => {
                        rgba.get(i * 4..i * 4 + 4).map_or(0, |p| lfb::rgba_to_565([p[0], p[1], p[2], p[3]]))
                    }
                    ReadPixels::Depth(depth) => depth.get(i).map_or(0, |d| (d.clamp(0.0, 1.0) * 65535.0).round() as u16),
                };
                data[y * stride + x * 2..y * stride + x * 2 + 2].copy_from_slice(&v.to_le_bytes());
            }
        }
        let mut lock = Lock { buffer, origin, pipeline: false, format: self.lfb_format, data, stride };
        let info = LockInfo { data: lock.data.as_mut_ptr(), stride: stride as u32, write_mode: LfbWriteMode::W565, origin };
        self.read_lock = Some(lock);
        info
    }

    pub fn lfb_lock_write(&mut self, buffer: Buffer, mode: LfbWriteMode, origin: Origin, pipeline: bool) -> LockInfo {
        let origin = if origin == Origin::Any { Origin::UpperLeft } else { origin };
        let mode = if mode == LfbWriteMode::Any { LfbWriteMode::W565 } else { mode };
        let stride = self.width as usize * mode.bytes_per_pixel() as usize;
        let mut data = vec![0u8; stride * self.height as usize];
        lfb::fill_sentinel(&mut data, mode);
        let format = WriteFormat { mode, ..self.lfb_format };
        let mut lock = Lock { buffer, origin, pipeline, format, data, stride };
        let info = LockInfo { data: lock.data.as_mut_ptr(), stride: stride as u32, write_mode: mode, origin };
        self.write_lock = Some(lock);
        info
    }

    /// Releases a lock. A write lock turns the pixels the game wrote into a command.
    pub fn lfb_unlock(&mut self, write: bool) -> bool {
        if !write {
            return self.read_lock.take().is_some();
        }
        let Some(lock) = self.write_lock.take() else { return false };
        let pixels = lock.format.decode(&lock.data, lock.stride, self.width, self.height);
        let rows: Vec<Vec<Option<lfb::WrittenPixel>>> =
            pixels.chunks(self.width as usize).map(<[_]>::to_vec).collect();
        let rows: Vec<_> = if lock.origin == Origin::LowerLeft { rows.into_iter().rev().collect() } else { rows };
        self.emit_lfb_write(lock.buffer, 0, 0, self.width, &rows.concat(), lock.pipeline);
        true
    }

    fn emit_lfb_write(&mut self, buffer: Buffer, x: u32, y: u32, width: u32, pixels: &[Option<lfb::WrittenPixel>], pipeline: bool) {
        // Shrink to the written pixels' bounding box.
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        for (i, p) in pixels.iter().enumerate() {
            if p.is_some() {
                let (px, py) = (i as u32 % width, i as u32 / width);
                x0 = x0.min(px);
                y0 = y0.min(py);
                x1 = x1.max(px + 1);
                y1 = y1.max(py + 1);
            }
        }
        if x0 >= x1 {
            return;
        }
        let (w, h) = (x1 - x0, y1 - y0);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        let mut mask = Vec::with_capacity((w * h) as usize);
        let mut depth = Vec::with_capacity((w * h) as usize);
        let mut has_depth = false;
        for py in y0..y1 {
            for px in x0..x1 {
                match pixels[(py * width + px) as usize] {
                    Some(p) => {
                        rgba.extend_from_slice(&p.rgba);
                        mask.push(255);
                        has_depth |= p.depth.is_some();
                        depth.push(p.depth.unwrap_or(self.lfb_format.constant_depth));
                    }
                    None => {
                        rgba.extend_from_slice(&[0; 4]);
                        mask.push(0);
                        depth.push(0);
                    }
                }
            }
        }
        let rect = Rect { x: x + x0, y: y + y0, width: w, height: h };
        let pipeline = pipeline.then(|| {
            let mut state = self.shader_state(&[None, None]);
            state.flags[0] |= flag::LFB | flag::LFB_PIPELINE | if has_depth { flag::LFB_DEPTH } else { 0 };
            (self.pipeline_key(), state)
        });
        let scissor = if pipeline.is_some() { self.scissor() } else { Rect { x: 0, y: 0, width: self.width, height: self.height } };
        self.commands.push(Command::LfbWrite {
            buffer: if buffer == Buffer::Front { Buffer::Front } else { Buffer::Back },
            rect,
            rgba,
            mask,
            depth: has_depth.then_some(depth),
            pipeline,
            scissor,
        });
    }

    /// `grLfbWriteRegion`. `stride` is in bytes and may be negative.
    #[allow(clippy::too_many_arguments)]
    pub fn lfb_write_region(
        &mut self,
        buffer: Buffer,
        x: u32,
        y: u32,
        src_format: u32,
        width: u32,
        height: u32,
        stride: i32,
        data: &[u8],
    ) -> bool {
        let format = if src_format == lfb_src::RLE16 { 0 } else { src_format };
        let Ok(mode) = LfbWriteMode::try_from(format) else { return false };
        let bpp = mode.bytes_per_pixel() as usize;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for row in 0..height as i64 {
            let start = row * stride as i64;
            for col in 0..width as usize {
                let at = start + (col * bpp) as i64;
                let raw = usize::try_from(at).ok().and_then(|at| data.get(at..at + bpp));
                pixels.push(raw.and_then(|raw| lfb::region_pixel(format, raw, &self.lfb_format)));
            }
        }
        // The region's y is in the current origin's space.
        let (y, pixels) = if self.origin == Origin::LowerLeft {
            let flipped: Vec<_> = pixels.chunks(width.max(1) as usize).rev().flatten().copied().collect();
            (self.height.saturating_sub(y + height), flipped)
        } else {
            (y, pixels)
        };
        self.emit_lfb_write(buffer, x, y, width, &pixels, false);
        true
    }

    /// Packs read-back pixels of a region as 565 rows into `dst` (`grLfbReadRegion`).
    pub fn lfb_read_region(&self, rgba: &[u8], x: u32, y: u32, width: u32, height: u32, stride: usize, dst: &mut [u8]) {
        let w = self.width as usize;
        for row in 0..height as usize {
            let sy = y as usize + row;
            let sy = if self.origin == Origin::LowerLeft { (self.height as usize).wrapping_sub(1 + sy) } else { sy };
            for col in 0..width as usize {
                let i = sy.wrapping_mul(w) + x as usize + col;
                let v = rgba.get(i * 4..i * 4 + 4).map_or(0, |p| lfb::rgba_to_565([p[0], p[1], p[2], p[3]]));
                if let Some(out) = dst.get_mut(row * stride + col * 2..row * stride + col * 2 + 2) {
                    out.copy_from_slice(&v.to_le_bytes());
                }
            }
        }
    }

    // ---- State save and restore -------------------------------------------

    /// `grGlideGetState`: serializes the state into `out` (at least 272 bytes).
    pub fn get_state(&self, out: &mut [u8]) {
        snapshot::save(self, out);
    }

    /// `grGlideSetState`.
    pub fn set_state(&mut self, data: &[u8]) {
        self.dirty();
        snapshot::restore(self, data);
    }
}

fn pack(format: ColorFormat, [r, g, b, a]: [u8; 4]) -> u32 {
    let (r, g, b, a) = (r as u32, g as u32, b as u32, a as u32);
    match format {
        ColorFormat::Argb => a << 24 | r << 16 | g << 8 | b,
        ColorFormat::Abgr => a << 24 | b << 16 | g << 8 | r,
        ColorFormat::Rgba => r << 24 | g << 16 | b << 8 | a,
        ColorFormat::Bgra => b << 24 | g << 16 | r << 8 | a,
    }
}

fn hash<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::hash::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests;
