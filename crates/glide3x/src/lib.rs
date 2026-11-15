//! The Glide 3.x C ABI over `rustyglide`, built as `glide3x.dll` /
//! `libglide3x.so`.
//!
//! Glide 3 shares most calls with Glide 2 (`glide_abi::common_exports!`).
//! What differs is handled here, following OpenGLide-ng's Glide 3 wrapper:
//! vertices are read through the game's vertex layout and handed to the core
//! as Glide 2 vertices, LOD and aspect values are converted from Glide 3's
//! log2 numbering, and calls without an equivalent are accepted as no-ops.
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use glide_abi::rustyglide::texture::{self, TexInfo};
use glide_abi::rustyglide::types::*;
use glide_abi::rustyglide::{GrTmuVertex, GrVertex};
use glide_abi::*;
use std::ffi::{c_char, c_int, c_void};
use std::sync::Mutex;

pub use glide_abi::{GrLfbInfo, GrTexInfo};

glide_abi::common_exports!();

// ---- Vertex layout ----------------------------------------------------------------

/// `grVertexLayout` parameters, in [`Layout::offsets`] order.
const PARAMS: [u32; 12] = [0x01, 0x02, 0x03, 0x04, 0x05, 0x10, 0x20, 0x30, 0x40, 0x41, 0x50, 0x51];
const XY: usize = 0;
const Z: usize = 1;
const W: usize = 2;
const Q: usize = 3;
const A: usize = 5;
const RGB: usize = 6;
const PARGB: usize = 7;
const ST0: usize = 8;
const ST1: usize = 9;
const Q0: usize = 10;
const Q1: usize = 11;

/// The game's vertex layout and coordinate space. `grGlideGetVertexLayout`
/// copies it out verbatim, so it is plain data.
#[repr(C)]
#[derive(Clone, Copy)]
struct Layout {
    /// Byte offset of each parameter, or -1 when disabled.
    offsets: [i32; 12],
    clip_coords: i32,
    viewport: [f32; 4],
    depth_range: [f32; 2],
}

const LAYOUT_SIZE: usize = std::mem::size_of::<Layout>();

impl Layout {
    const RESET: Self =
        Self { offsets: [-1; 12], clip_coords: 0, viewport: [0.0, 0.0, 640.0, 480.0], depth_range: [0.0, 1.0] };
}

static LAYOUT: Mutex<Layout> = Mutex::new(Layout::RESET);

fn layout() -> Layout {
    *LAYOUT.lock().unwrap_or_else(|e| e.into_inner())
}

fn update(f: impl FnOnce(&mut Layout)) {
    f(&mut LAYOUT.lock().unwrap_or_else(|e| e.into_inner()));
}

/// Reads a game vertex through the layout into a Glide 2 vertex.
///
/// # Safety
/// `p` points to a vertex holding every enabled parameter.
unsafe fn read(l: &Layout, p: *const u8) -> GrVertex {
    let f = |i: usize, k: usize| -> Option<f32> {
        (l.offsets[i] >= 0).then(|| {
            // SAFETY: the caller guarantees enabled parameters are readable.
            unsafe { p.add(l.offsets[i] as usize + k * 4).cast::<f32>().read_unaligned() }
        })
    };
    let mut v = GrVertex { x: f(XY, 0).unwrap_or(0.0), y: f(XY, 1).unwrap_or(0.0), ..Default::default() };
    v.ooz = f(Z, 0).unwrap_or(0.0);
    v.oow = f(Q, 0).or(f(W, 0)).or(f(Q0, 0)).unwrap_or(1.0);
    if let Some(r) = f(RGB, 0) {
        (v.r, v.g, v.b) = (r, f(RGB, 1).unwrap_or(0.0), f(RGB, 2).unwrap_or(0.0));
    }
    v.a = f(A, 0).unwrap_or(255.0);
    if l.offsets[PARGB] >= 0 {
        // SAFETY: as above; a packed ARGB word.
        let argb = unsafe { p.add(l.offsets[PARGB] as usize).cast::<u32>().read_unaligned() };
        let c = |shift: u32| ((argb >> shift) & 0xff) as f32;
        (v.a, v.r, v.g, v.b) = (c(24), c(16), c(8), c(0));
    }
    let tmu = |st: usize, q: usize, oow: f32| GrTmuVertex {
        sow: f(st, 0).unwrap_or(0.0),
        tow: f(st, 1).unwrap_or(0.0),
        oow: f(q, 0).unwrap_or(oow),
    };
    let t0 = tmu(ST0, Q0, v.oow);
    v.tmuvtx = [t0, if l.offsets[ST1] >= 0 { tmu(ST1, Q1, t0.oow) } else { t0 }];
    if l.clip_coords != 0 {
        to_window(l, &mut v);
    }
    v
}

/// Clip coordinates to window coordinates: perspective divide, viewport and
/// depth range; colors arrive in 0..=1.
fn to_window(l: &Layout, v: &mut GrVertex) {
    let w = if v.oow == 0.0 { 1.0 } else { v.oow };
    let oow = 1.0 / w;
    let [vx, vy, vw, vh] = l.viewport;
    v.x = vx + (v.x * oow + 1.0) * vw / 2.0;
    v.y = vy + (v.y * oow + 1.0) * vh / 2.0;
    let [n, f] = l.depth_range;
    v.ooz = ((v.ooz * oow) * (f - n) / 2.0 + (f + n) / 2.0) * 65535.0;
    v.oow = oow;
    for c in [&mut v.r, &mut v.g, &mut v.b, &mut v.a] {
        *c *= 255.0;
    }
    // shortcut: texture coordinates are divided by w but not rescaled from
    // 0..1; correct once a clip-space Glide 3 game shows how it scales them.
    for t in &mut v.tmuvtx {
        (t.sow, t.tow, t.oow) = (t.sow * oow, t.tow * oow, oow);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grVertexLayout(param: u32, offset: i32, mode: u32) {
    let Some(i) = PARAMS.iter().position(|&p| p == param) else {
        log::warn!("grVertexLayout: unknown parameter {param:#x}");
        return;
    };
    update(|l| {
        l.offsets[i] = if mode != 0 { offset } else { -1 };
        // Packed and separate colors exclude each other.
        if mode != 0 && i == PARGB {
            l.offsets[A] = -1;
            l.offsets[RGB] = -1;
        } else if mode != 0 && (i == A || i == RGB) {
            l.offsets[PARGB] = -1;
        }
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGlideGetVertexLayout(out: *mut c_void) {
    if !out.is_null() {
        // SAFETY: the game passes a buffer of grGet(GR_GLIDE_VERTEXLAYOUT_SIZE) bytes.
        unsafe { out.cast::<Layout>().write_unaligned(layout()) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGlideSetVertexLayout(layout: *const c_void) {
    if !layout.is_null() {
        // SAFETY: a layout from grGlideGetVertexLayout.
        let saved = unsafe { layout.cast::<Layout>().read_unaligned() };
        update(|l| *l = saved);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grCoordinateSpace(mode: u32) {
    update(|l| l.clip_coords = (mode != 0) as i32);
}

#[unsafe(no_mangle)]
pub extern "system" fn grViewport(x: i32, y: i32, width: i32, height: i32) {
    update(|l| l.viewport = [x as f32, y as f32, width as f32, height as f32]);
}

#[unsafe(no_mangle)]
pub extern "system" fn grDepthRange(near: f32, far: f32) {
    update(|l| l.depth_range = [near, far]);
}

// ---- Drawing -------------------------------------------------------------------

unsafe fn vertex_at(p: *const c_void) -> Option<GrVertex> {
    // SAFETY: forwarded from the caller.
    (!p.is_null()).then(|| unsafe { read(&layout(), p.cast()) })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPoint(p: *const c_void) {
    // SAFETY: the game passes a vertex in its layout.
    if let Some(p) = unsafe { vertex_at(p) } {
        glide(|g| g.draw_point(&p));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawLine(a: *const c_void, b: *const c_void) {
    // SAFETY: the game passes vertices in its layout.
    if let (Some(a), Some(b)) = unsafe { (vertex_at(a), vertex_at(b)) } {
        glide(|g| g.draw_line(&a, &b));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawTriangle(a: *const c_void, b: *const c_void, c: *const c_void) {
    // SAFETY: the game passes vertices in its layout.
    if let (Some(a), Some(b), Some(c)) = unsafe { (vertex_at(a), vertex_at(b), vertex_at(c)) } {
        glide(|g| g.draw_triangle(&a, &b, &c));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawTriangle(
    a: *const c_void,
    b: *const c_void,
    c: *const c_void,
    _ab: FxBool,
    _bc: FxBool,
    _ca: FxBool,
) {
    // shortcut: drawn without edge antialiasing, like OpenGLide.
    // SAFETY: as grDrawTriangle.
    unsafe { grDrawTriangle(a, b, c) }
}

const POINTS: u32 = 0;
const LINE_STRIP: u32 = 1;
const LINES: u32 = 2;
const POLYGON: u32 = 3;
const TRIANGLE_STRIP: u32 = 4;
const TRIANGLE_FAN: u32 = 5;
const TRIANGLES: u32 = 6;
const TRIANGLE_STRIP_CONTINUE: u32 = 7;
const TRIANGLE_FAN_CONTINUE: u32 = 8;

/// Vertices a strip or fan carries into its `_CONTINUE` call.
struct Primitive {
    /// Strip: the last two vertices; fan: the center and the last vertex.
    kept: Vec<GrVertex>,
    /// Triangles drawn in the current strip, for alternating winding.
    count: usize,
}

static PRIMITIVE: Mutex<Primitive> = Mutex::new(Primitive { kept: Vec::new(), count: 0 });

fn draw_array(mode: u32, vertices: Vec<GrVertex>) {
    let mut prim = PRIMITIVE.lock().unwrap_or_else(|e| e.into_inner());
    glide(|g| match mode {
        POINTS => vertices.iter().for_each(|v| g.draw_point(v)),
        LINE_STRIP => vertices.windows(2).for_each(|p| g.draw_line(&p[0], &p[1])),
        LINES => vertices.as_chunks::<2>().0.iter().for_each(|[a, b]| g.draw_line(a, b)),
        POLYGON => g.draw_polygon_list(&vertices),
        TRIANGLES => vertices.as_chunks::<3>().0.iter().for_each(|[a, b, c]| g.draw_triangle(a, b, c)),
        TRIANGLE_STRIP | TRIANGLE_STRIP_CONTINUE => {
            if mode == TRIANGLE_STRIP {
                prim.kept.clear();
                prim.count = 0;
            }
            for v in vertices {
                prim.kept.push(v);
                if prim.kept.len() == 3 {
                    let [a, b, c] = [prim.kept[0], prim.kept[1], prim.kept[2]];
                    // Every other triangle is reversed to keep one winding for culling.
                    if prim.count.is_multiple_of(2) {
                        g.draw_triangle(&a, &b, &c)
                    } else {
                        g.draw_triangle(&b, &a, &c)
                    }
                    prim.count += 1;
                    prim.kept.remove(0);
                }
            }
        }
        TRIANGLE_FAN | TRIANGLE_FAN_CONTINUE => {
            if mode == TRIANGLE_FAN {
                prim.kept.clear();
            }
            for v in vertices {
                if prim.kept.len() == 2 {
                    g.draw_triangle(&prim.kept[0], &prim.kept[1], &v);
                    prim.kept[1] = v;
                } else {
                    prim.kept.push(v);
                }
            }
        }
        _ => log::warn!("grDrawVertexArray: unknown mode {mode}"),
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawVertexArray(mode: u32, count: u32, pointers: *const *const c_void) {
    let l = layout();
    // SAFETY: the game passes `count` vertex pointers.
    let pointers = unsafe { slice(pointers, count as usize) };
    // SAFETY: each pointer addresses a vertex in the layout.
    let vertices = pointers.iter().filter(|p| !p.is_null()).map(|&p| unsafe { read(&l, p.cast()) }).collect();
    draw_array(mode, vertices);
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawVertexArrayContiguous(mode: u32, count: u32, base: *const c_void, stride: u32) {
    if base.is_null() {
        return;
    }
    let l = layout();
    // SAFETY: the game passes `count` vertices `stride` bytes apart.
    let vertices =
        (0..count as usize).map(|i| unsafe { read(&l, base.cast::<u8>().add(i * stride as usize)) }).collect();
    draw_array(mode, vertices);
}

// ---- Textures: Glide 3 numbers LODs and aspects in log2 ---------------------------

fn lod2(lod3: i32) -> u32 {
    (8 - lod3) as u32
}

fn aspect2(aspect3: i32) -> u32 {
    (3 - aspect3) as u32
}

unsafe fn tex_info3(call: &str, info: *const GrTexInfo) -> Option<(TexInfo, *mut c_void)> {
    // SAFETY: Glide callers pass a valid GrTexInfo pointer; null is rejected.
    let i = unsafe { info.as_ref() }?;
    Some((
        TexInfo {
            small_lod: arg(call, lod2(i.small_lod))?,
            large_lod: arg(call, lod2(i.large_lod))?,
            aspect: arg(call, aspect2(i.aspect_ratio))?,
            format: arg(call, i.format)?,
        },
        i.data,
    ))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexCalcMemRequired(small: c_int, large: c_int, aspect: c_int, format: c_int) -> u32 {
    let (small, large, aspect) = (lod2(small), lod2(large), aspect2(aspect));
    typed!("grTexCalcMemRequired", small: Lod, large: Lod, aspect: Aspect, format: TextureFormat =>
        texture::mem_required(small, large, aspect, format, level_mask::BOTH))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexTextureMemRequired(even_odd: u32, info: *const GrTexInfo) -> u32 {
    // SAFETY: the game passes a GrTexInfo.
    let Some((i, _)) = (unsafe { tex_info3("grTexTextureMemRequired", info) }) else { return 0 };
    texture::mem_required(i.small_lod, i.large_lod, i.aspect, i.format, even_odd)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexSource(tmu: c_int, address: u32, even_odd: u32, info: *const GrTexInfo) {
    // SAFETY: the game passes a GrTexInfo.
    let Some((i, _)) = (unsafe { tex_info3("grTexSource", info) }) else { return };
    typed!("grTexSource", tmu: Tmu => glide(|g| g.tex_source(tmu, address, even_odd, i)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadMipMap(tmu: c_int, address: u32, even_odd: u32, info: *const GrTexInfo) {
    // SAFETY: the game passes a GrTexInfo whose data holds every level.
    let Some((i, data)) = (unsafe { tex_info3("grTexDownloadMipMap", info) }) else { return };
    // SAFETY: as above.
    let data = unsafe { slice(data.cast::<u8>(), host_size(&i)) };
    typed!("grTexDownloadMipMap", tmu: Tmu => glide(|g| g.tex_download_mipmap(tmu, address, even_odd, i, data)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadMipMapLevel(
    tmu: c_int,
    address: u32,
    lod: c_int,
    large: c_int,
    aspect: c_int,
    format: c_int,
    even_odd: u32,
    data: *const c_void,
) {
    // SAFETY: forwarded from the caller; one whole level.
    unsafe { grTexDownloadMipMapLevelPartial(tmu, address, lod, large, aspect, format, even_odd, data, 0, 255) };
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadMipMapLevelPartial(
    tmu: c_int,
    address: u32,
    lod: c_int,
    large: c_int,
    aspect: c_int,
    format: c_int,
    even_odd: u32,
    data: *const c_void,
    start: c_int,
    end: c_int,
) -> FxBool {
    let (lod, large, aspect) = (lod2(lod), lod2(large), aspect2(aspect));
    typed!("grTexDownloadMipMapLevelPartial", tmu: Tmu, lod: Lod, large: Lod, aspect: Aspect, format: TextureFormat => {
        let (w, h) = texture::level_dims(lod, aspect);
        let (start, end) = (start.max(0) as u32, (end.max(0) as u32).min(h.saturating_sub(1)));
        let rows = end.saturating_sub(start) as usize + 1;
        // SAFETY: `data` holds rows start..=end of the level.
        let data = unsafe { slice(data.cast::<u8>(), rows * (w * format.bytes_per_texel()) as usize) };
        glide(|g| g.tex_download_level_partial(tmu, address, lod, large, aspect, format, even_odd, data, start..=end));
        1
    })
}

/// Glide 3 tables apply to every TMU.
unsafe fn download_all(call: &str, kind: u32, data: *const c_void, start: usize, end: usize) {
    let tmus = glide(|g| g.config().tmus);
    for tmu in 0..tmus as c_int {
        // SAFETY: forwarded from the caller.
        unsafe { download_table(call, tmu, kind, data, start, end) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadTable(kind: u32, data: *const c_void) {
    // SAFETY: forwarded from the caller.
    unsafe { download_all("grTexDownloadTable", kind, data, 0, 255) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadTablePartial(kind: u32, data: *const c_void, start: c_int, end: c_int) {
    // SAFETY: forwarded from the caller.
    unsafe { download_all("grTexDownloadTablePartial", kind, data, start.max(0) as usize, end.max(0) as usize) }
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexNCCTable(table: u32) {
    let Some(table) = arg::<NccTable>("grTexNCCTable", table) else { return };
    glide(|g| {
        for tmu in [Tmu::Tmu0, Tmu::Tmu1] {
            g.tex_ncc_table(tmu, table);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexMultibaseAddress(
    tmu: c_int,
    range: u32,
    address: u32,
    _even_odd: u32,
    _info: *const GrTexInfo,
) {
    // Glide 3 numbers the ranges from 32..1 (0) up to 256 (3).
    let range = 3u32.wrapping_sub(range);
    typed!("grTexMultibaseAddress", tmu: Tmu, range: TexBaseRange => glide(|g| g.tex_multibase_address(tmu, range, address)))
}

// ---- Fog and gamma ----------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grFogMode(mode: u32) {
    // Glide 3: 1 table on fog coordinate, 2 table on W, 3 iterated Z, 4 iterated alpha.
    let base = match mode & 0xff {
        // shortcut: no fog coordinate yet, so it uses the W table like OpenGLide.
        1 | 2 => FogMode::WITH_TABLE,
        3 => FogMode::WITH_ITERATED_Z,
        4 => FogMode::WITH_ITERATED_ALPHA,
        _ => FogMode::DISABLE,
    };
    glide(|g| g.fog_mode(FogMode(base | (mode & (FogMode::MULT2 | FogMode::ADD2)))));
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grLoadGammaTable(entries: u32, red: *const u32, _green: *const u32, _blue: *const u32) {
    // SAFETY: the game passes `entries` values per channel.
    let red = unsafe { slice(red, entries as usize) };
    // shortcut: one gamma value estimated from the red ramp's midpoint; the
    // renderer applies a single exponent, not a table.
    if let Some(&mid) = red.get(red.len() / 2)
        && mid > 0
        && mid < 255
    {
        let x = (red.len() / 2) as f32 / (red.len() - 1).max(1) as f32;
        let gamma = x.ln() / (mid as f32 / 255.0).ln();
        glide(|g| g.gamma_correction(gamma));
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn guGammaCorrectionRGB(red: f32, green: f32, blue: f32) {
    glide(|g| g.gamma_correction((red + green + blue) / 3.0));
}

// ---- Contexts, queries and stubs --------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grSstWinClose(_context: u32) -> FxBool {
    glide(|g| g.sst_win_close());
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grSelectContext(_context: u32) -> FxBool {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grFinish() {
    glide(|g| g.sst_idle());
}

#[unsafe(no_mangle)]
pub extern "system" fn grFlush() {
    glide(|g| g.flush());
}

#[unsafe(no_mangle)]
pub extern "system" fn grEnable(_mode: u32) {}

#[unsafe(no_mangle)]
pub extern "system" fn grDisable(_mode: u32) {}

#[unsafe(no_mangle)]
pub extern "system" fn grSstConfigPipeline(_a: c_int, _b: c_int, _c: c_int) {}

#[unsafe(no_mangle)]
pub extern "system" fn grGetProcAddress(_name: *const c_char) -> *const c_void {
    // No extensions are offered.
    std::ptr::null()
}

/// `GR_VERTEX_PARAMETER` for `grReset`.
const VERTEX_PARAMETER: u32 = 0x29;

#[unsafe(no_mangle)]
pub extern "system" fn grReset(what: u32) -> FxBool {
    if what == VERTEX_PARAMETER {
        update(|l| l.offsets = [-1; 12]);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grGetString(name: u32) -> *const c_char {
    let s: &'static [u8] = match name {
        0xa0 => b" \0",
        0xa1 => b"Voodoo2\0",
        0xa2 => b"Glide\0",
        0xa3 => b"3Dfx Interactive\0",
        0xa4 => b"3.01\0",
        _ => return std::ptr::null(),
    };
    s.as_ptr().cast()
}

/// `grGet`: board facts, as OpenGLide reports them. Returns the bytes written.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGet(name: u32, length: u32, params: *mut i32) -> u32 {
    if params.is_null() || length == 0 {
        return 0;
    }
    let (tmus, tmu_memory, fb_mb, size) = glide(|g| {
        let c = g.config();
        (c.tmus as i32, c.tmu_memory as i32, c.fb_memory_mb as i32, g.screen_size())
    });
    let l = layout();
    let values: Vec<i32> = match name {
        0x01 => vec![16],                    // GR_BITS_DEPTH
        0x02 => vec![5, 6, 5, 0],            // GR_BITS_RGBA
        0x04 => vec![FOG_TABLE_SIZE as i32], // GR_FOG_TABLE_ENTRIES
        0x05 => vec![256],                   // GR_GAMMA_TABLE_ENTRIES
        0x06 => vec![GR_STATE_SIZE as i32],  // GR_GLIDE_STATE_SIZE
        0x07 => vec![LAYOUT_SIZE as i32],    // GR_GLIDE_VERTEXLAYOUT_SIZE
        0x09 => vec![1],                     // GR_LFB_PIXEL_PIPE
        0x0a => vec![256],                   // GR_MAX_TEXTURE_SIZE
        0x0b => vec![3],                     // GR_MAX_TEXTURE_ASPECT_RATIO
        0x0c => vec![fb_mb << 20],           // GR_MEMORY_FB
        0x0d => vec![tmu_memory],            // GR_MEMORY_TMU
        0x0f | 0x11 => vec![1],              // GR_NUM_BOARDS, GR_NUM_FB
        0x13 => vec![tmus],                  // GR_NUM_TMU
        0x15 => vec![2],                     // GR_REVISION_FB
        0x16 => vec![1],                     // GR_REVISION_TMU
        0x23 => vec![1],                     // GR_SUPPORTS_PASSTHRU
        0x24 => vec![8],                     // GR_TEXTURE_ALIGN
        0x26 => {
            // GR_VIEWPORT
            let [x, y, w, h] = l.viewport;
            let w = if w == 0.0 { size.0 as f32 } else { w };
            let h = if h == 0.0 { size.1 as f32 } else { h };
            vec![x as i32, y as i32, w as i32, h as i32]
        }
        0x27 => vec![0, 0xffff], // GR_WDEPTH_MIN_MAX
        0x28 => vec![0xffff, 0], // GR_ZDEPTH_MIN_MAX
        0x2a => vec![8],         // GR_BITS_GAMMA
        // Statistics, busy, pending swaps and everything else read as zero.
        _ => Vec::new(),
    };
    let n = (length / 4) as usize;
    // SAFETY: the caller provides `length` bytes.
    let out = unsafe { std::slice::from_raw_parts_mut(params, n) };
    out.fill(0);
    for (o, v) in out.iter_mut().zip(values) {
        *o = v;
    }
    length
}

/// `GrResolution`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GrResolution {
    pub resolution: i32,
    pub refresh: i32,
    pub num_color_buffers: i32,
    pub num_aux_buffers: i32,
}

/// `grQueryResolutions`: 640x480 to 1600x1200, double buffered with depth.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn grQueryResolutions(template: *const GrResolution, output: *mut GrResolution) -> i32 {
    const ANY: i32 = -1;
    // SAFETY: the game passes a template or null.
    let t = unsafe { template.as_ref() }.copied();
    let matches: Vec<GrResolution> = (0x07..=0x0e)
        .map(|resolution| GrResolution { resolution, refresh: 0, num_color_buffers: 2, num_aux_buffers: 1 })
        .filter(|r| t.is_none_or(|t| t.resolution == ANY || t.resolution == r.resolution))
        .collect();
    let bytes = (matches.len() * std::mem::size_of::<GrResolution>()) as i32;
    if !output.is_null() {
        // SAFETY: the caller sized `output` with a previous null-output call.
        unsafe { std::ptr::copy_nonoverlapping(matches.as_ptr(), output, matches.len()) };
    }
    bytes
}

// ---- Files and the LFB ------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfGetInfo(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: forwarded from the caller; Glide 3 numbers LODs and aspects in log2.
    unsafe { gu3df_get_info(name, info, |l| 8 - l as c_int, |a| 3 - a as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfLoad(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: forwarded from the caller.
    unsafe { gu3df_load(name, info) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grLfbWriteRegion(
    buffer: c_int,
    x: u32,
    y: u32,
    format: u32,
    width: u32,
    height: u32,
    _pixel_pipeline: FxBool,
    stride: i32,
    src: *const c_void,
) -> FxBool {
    // shortcut: region writes bypass the pixel pipeline, like OpenGLide.
    // SAFETY: forwarded from the caller.
    unsafe { lfb_write_region(buffer, x, y, format, width, height, stride, src) }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_listed_export_is_defined() {
        let source = [include_str!("lib.rs"), include_str!("../../glide-abi/src/lib.rs")].concat();
        let table = include_str!("../exports.txt");
        let missing: Vec<&str> = table
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| l.split_whitespace().next())
            .filter(|name| !source.contains(&format!("extern \"system\" fn {name}(")))
            .collect();
        assert!(missing.is_empty(), "exports without a definition: {missing:?}");
    }
}
