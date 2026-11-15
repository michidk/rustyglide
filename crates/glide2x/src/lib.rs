//! The Glide 2.4x C ABI over `rustyglide`, built as `glide2x.dll` /
//! `libglide2x.so`.
//!
//! Every export converts raw C values to typed ones; an invalid value is
//! logged once and the call ignored, as 3dfx's release library does.
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use glide_abi::rustyglide::gutex::{Attributes, GrMipMapInfo, GuNccTable};
use glide_abi::rustyglide::texture::{self, TexInfo};
use glide_abi::rustyglide::types::*;
use glide_abi::rustyglide::{GrVertex, Settings};
use glide_abi::*;
use std::ffi::{c_char, c_int, c_void};

pub use glide_abi::{GrLfbInfo, GrTexInfo};

glide_abi::common_exports!();

// ---- Initialization and the SST -----------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGlideGetVersion(version: *mut c_char) {
    if version.is_null() {
        return;
    }
    let text = format!("Glide 2.46 - rustyglide {}\0", env!("CARGO_PKG_VERSION"));
    let len = text.len().min(80);
    // SAFETY: Glide's contract gives an 80-byte buffer.
    unsafe {
        std::ptr::copy_nonoverlapping(text.as_ptr(), version.cast(), len);
        *version.add(79) = 0;
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grGlideShamelessPlug(_on: FxBool) {}

/// `GrHwConfiguration` with the Voodoo member of the board union.
#[repr(C)]
pub struct HwConfig {
    num_sst: c_int,
    ssts: [SstConfig; 4],
}

#[repr(C)]
struct SstConfig {
    kind: c_int,
    fb_ram: c_int,
    fbi_rev: c_int,
    n_texelfx: c_int,
    sli_detect: FxBool,
    tmu: [[c_int; 2]; 2],
}

unsafe fn query(config: *mut HwConfig) -> FxBool {
    // SAFETY: the game passes a GrHwConfiguration.
    let Some(c) = (unsafe { config.as_mut() }) else { return 0 };
    let settings = Settings::from_env().core;
    c.num_sst = 1;
    c.ssts[0] = SstConfig {
        kind: 0,
        fb_ram: settings.fb_memory_mb as c_int,
        fbi_rev: 2,
        n_texelfx: settings.tmus as c_int,
        sli_detect: 0,
        tmu: [[1, (settings.tmu_memory >> 20) as c_int]; 2],
    };
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grSstQueryHardware(config: *mut HwConfig) -> FxBool {
    // SAFETY: forwarded from the caller.
    unsafe { query(config) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grSstQueryBoards(config: *mut HwConfig) -> FxBool {
    // SAFETY: the game passes a GrHwConfiguration.
    if let Some(c) = unsafe { config.as_mut() } {
        c.num_sst = 1;
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstWinClose() {
    glide(|g| g.sst_win_close());
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstControl(_code: u32) -> FxBool {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstControlMode(_mode: u32) -> FxBool {
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstIdle() {
    glide(|g| g.sst_idle());
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstIsBusy() -> FxBool {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstStatus() -> u32 {
    // FIFOs empty, nothing busy; the retrace bit toggles so polling loops end.
    with(|s| {
        s.retrace = !s.retrace;
        0x0fff_f03f | (s.retrace as u32) << 6
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstVRetraceOn() -> FxBool {
    with(|s| {
        s.retrace = !s.retrace;
        s.retrace as FxBool
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstVideoLine() -> u32 {
    with(|s| {
        s.video_line = (s.video_line + 1) % 525;
        s.video_line
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstScreenWidth() -> u32 {
    glide(|g| g.screen_size().0)
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstScreenHeight() -> u32 {
    glide(|g| g.screen_size().1)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grSstPerfStats(stats: *mut [u32; 5]) {
    // SAFETY: the game passes a GrSstPerfStats_t.
    if let Some(s) = unsafe { stats.as_mut() } {
        *s = [0; 5];
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstResetPerfStats() {}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTriStats(processed: *mut u32, drawn: *mut u32) {
    let (p, d) = glide(|g| g.tri_stats());
    // SAFETY: the game passes two FxU32 out pointers; null is skipped.
    unsafe {
        if let Some(out) = processed.as_mut() {
            *out = p;
        }
        if let Some(out) = drawn.as_mut() {
            *out = d;
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grResetTriStats() {
    glide(|g| g.reset_tri_stats());
}

#[unsafe(no_mangle)]
pub extern "system" fn grHints(hint: u32, mask: u32) {
    typed!("grHints", hint: Hint => glide(|g| g.hints(hint, mask)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grParameterData(_param: u32, _components: u32, _type: u32, _offset: i32) {}

// ---- Buffers ---------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grBufferNumPending() -> c_int {
    0
}

// ---- Combine, blend and pixel tests ------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn guColorCombineFunction(fnc: c_int) {
    typed!("guColorCombineFunction", fnc: ColorCombineFnc => glide(|g| g.gu_color_combine_function(fnc)))
}

#[unsafe(no_mangle)]
pub extern "system" fn guAlphaSource(source: c_int) {
    typed!("guAlphaSource", source: AlphaSource => glide(|g| g.gu_alpha_source(source)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grConstantColorValue4(a: f32, r: f32, g: f32, b: f32) {
    glide(|gl| gl.constant_color4(a, r, g, b));
}

#[unsafe(no_mangle)]
pub extern "system" fn grGammaCorrectionValue(value: f32) {
    glide(|g| g.gamma_correction(value));
}

// ---- Fog -------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grFogMode(mode: c_int) {
    typed!("grFogMode", mode: FogMode => glide(|g| g.fog_mode(mode)))
}

// ---- Drawing -------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawTriangle(a: *const GrVertex, b: *const GrVertex, c: *const GrVertex) {
    // SAFETY: the game passes valid vertices.
    if let (Some(a), Some(b), Some(c)) = unsafe { (vertex(a), vertex(b), vertex(c)) } {
        glide(|g| g.draw_triangle(a, b, c));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawTriangle(
    a: *const GrVertex,
    b: *const GrVertex,
    c: *const GrVertex,
    _ab: FxBool,
    _bc: FxBool,
    _ca: FxBool,
) {
    // shortcut: antialiased primitives draw without edge antialiasing, like OpenGLide.
    // SAFETY: as grDrawTriangle.
    unsafe { grDrawTriangle(a, b, c) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawLine(a: *const GrVertex, b: *const GrVertex) {
    // SAFETY: the game passes valid vertices.
    if let (Some(a), Some(b)) = unsafe { (vertex(a), vertex(b)) } {
        glide(|g| g.draw_line(a, b));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawLine(a: *const GrVertex, b: *const GrVertex) {
    // SAFETY: as grDrawLine.
    unsafe { grDrawLine(a, b) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPoint(p: *const GrVertex) {
    // SAFETY: the game passes a valid vertex.
    if let Some(p) = unsafe { vertex(p) } {
        glide(|g| g.draw_point(p));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawPoint(p: *const GrVertex) {
    // SAFETY: as grDrawPoint.
    unsafe { grDrawPoint(p) }
}

unsafe fn polygon(n: c_int, indices: *const c_int, vertices: *const GrVertex) {
    // SAFETY: the game passes `n` indices into its vertex array.
    let indices = unsafe { slice(indices, n.max(0) as usize) };
    let count = indices.iter().map(|&i| i.max(0) as usize + 1).max().unwrap_or(0);
    // SAFETY: every index addresses an element of `vertices`.
    let vertices = unsafe { slice(vertices, count) };
    glide(|g| g.draw_polygon(indices, vertices));
}

unsafe fn polygon_list(n: c_int, vertices: *const GrVertex) {
    // SAFETY: the game passes `n` vertices.
    let vertices = unsafe { slice(vertices, n.max(0) as usize) };
    glide(|g| g.draw_polygon_list(vertices));
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPolygon(n: c_int, indices: *const c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon.
    unsafe { polygon(n, indices, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPlanarPolygon(n: c_int, indices: *const c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon.
    unsafe { polygon(n, indices, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawPolygon(n: c_int, indices: *const c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon.
    unsafe { polygon(n, indices, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPolygonVertexList(n: c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon_list.
    unsafe { polygon_list(n, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grDrawPlanarPolygonVertexList(n: c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon_list.
    unsafe { polygon_list(n, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grAADrawPolygonVertexList(n: c_int, vertices: *const GrVertex) {
    // SAFETY: see polygon_list.
    unsafe { polygon_list(n, vertices) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guDrawTriangleWithClip(a: *const GrVertex, b: *const GrVertex, c: *const GrVertex) {
    // SAFETY: the game passes valid vertices.
    if let (Some(a), Some(b), Some(c)) = unsafe { (vertex(a), vertex(b), vertex(c)) } {
        glide(|g| g.draw_triangle_clipped(a, b, c));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guAADrawTriangleWithClip(a: *const GrVertex, b: *const GrVertex, c: *const GrVertex) {
    // SAFETY: as guDrawTriangleWithClip.
    unsafe { guDrawTriangleWithClip(a, b, c) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guDrawPolygonVertexListWithClip(n: c_int, vertices: *const GrVertex) {
    // SAFETY: the game passes `n` vertices.
    let vertices = unsafe { slice(vertices, n.max(0) as usize) };
    glide(|g| g.draw_polygon_list_clipped(vertices));
}

// ---- Textures ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grTexCalcMemRequired(small: c_int, large: c_int, aspect: c_int, format: c_int) -> u32 {
    typed!("grTexCalcMemRequired", small: Lod, large: Lod, aspect: Aspect, format: TextureFormat =>
        texture::mem_required(small, large, aspect, format, level_mask::BOTH))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexTextureMemRequired(even_odd: u32, info: *const GrTexInfo) -> u32 {
    // SAFETY: the game passes a GrTexInfo.
    let Some((i, _)) = (unsafe { tex_info("grTexTextureMemRequired", info) }) else { return 0 };
    texture::mem_required(i.small_lod, i.large_lod, i.aspect, i.format, even_odd)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexSource(tmu: c_int, address: u32, even_odd: u32, info: *const GrTexInfo) {
    // SAFETY: the game passes a GrTexInfo.
    let Some((i, _)) = (unsafe { tex_info("grTexSource", info) }) else { return };
    typed!("grTexSource", tmu: Tmu => glide(|g| g.tex_source(tmu, address, even_odd, i)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexCombineFunction(tmu: c_int, fnc: c_int) {
    typed!("grTexCombineFunction", tmu: Tmu, fnc: TextureCombineFnc => glide(|g| g.gu_tex_combine_function(tmu, fnc)))
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexCombineFunction(tmu: c_int, fnc: c_int) {
    grTexCombineFunction(tmu, fnc)
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexMultibaseAddress(
    tmu: c_int,
    range: u32,
    address: u32,
    _even_odd: u32,
    _info: *const GrTexInfo,
) {
    typed!("grTexMultibaseAddress", tmu: Tmu, range: TexBaseRange => glide(|g| g.tex_multibase_address(tmu, range, address)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexNCCTable(tmu: c_int, table: u32) {
    typed!("grTexNCCTable", tmu: Tmu, table: NccTable => glide(|g| g.tex_ncc_table(tmu, table)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadMipMap(tmu: c_int, address: u32, even_odd: u32, info: *const GrTexInfo) {
    // SAFETY: the game passes a GrTexInfo whose data holds every level.
    let Some((i, data)) = (unsafe { tex_info("grTexDownloadMipMap", info) }) else { return };
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
    typed!("grTexDownloadMipMapLevel", tmu: Tmu, lod: Lod, large: Lod, aspect: Aspect, format: TextureFormat => {
        let size = (texture::level_host_texels(lod, aspect) * format.bytes_per_texel()) as usize;
        // SAFETY: the game passes one whole level.
        let data = unsafe { slice(data.cast::<u8>(), size) };
        glide(|g| g.tex_download_level(tmu, address, lod, large, aspect, format, even_odd, data))
    })
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
) {
    typed!("grTexDownloadMipMapLevelPartial", tmu: Tmu, lod: Lod, large: Lod, aspect: Aspect, format: TextureFormat => {
        let (w, _) = texture::level_dims(lod, aspect);
        let (start, end) = (start.max(0) as u32, end.max(0) as u32);
        let rows = end.saturating_sub(start) as usize + 1;
        // SAFETY: `data` holds rows start..=end of the level.
        let data = unsafe { slice(data.cast::<u8>(), rows * (w * format.bytes_per_texel()) as usize) };
        glide(|g| g.tex_download_level_partial(tmu, address, lod, large, aspect, format, even_odd, data, start..=end))
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadTable(tmu: c_int, kind: u32, data: *const c_void) {
    // SAFETY: see download_table.
    unsafe { download_table("grTexDownloadTable", tmu, kind, data, 0, 255) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grTexDownloadTablePartial(
    tmu: c_int,
    kind: u32,
    data: *const c_void,
    start: c_int,
    end: c_int,
) {
    // SAFETY: see download_table.
    unsafe { download_table("grTexDownloadTablePartial", tmu, kind, data, start.max(0) as usize, end.max(0) as usize) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConvertAndDownloadRle(
    tmu: c_int,
    address: u32,
    lod: c_int,
    large: c_int,
    aspect: c_int,
    format: c_int,
    even_odd: u32,
    bm_data: *const u8,
    bm_h: i32,
    u0: u32,
    v0: u32,
    _width: u32,
    height: u32,
    dest_width: u32,
    dest_height: u32,
    tlut: *const u16,
) {
    // Glidos' run-length bitmaps: a 4-byte header, a byte per line with its
    // encoded length, then lines of literal palette indices (< 0xe0) or
    // runs (0xe1..0xff count, then the index), each line ending in 0xe0.
    if bm_data.is_null() || tlut.is_null() || dest_width == 0 {
        return;
    }
    let read = |i: usize| -> u8 {
        // SAFETY: the bitmap is well formed: indices stay inside its encoded lines.
        unsafe { *bm_data.add(i) }
    };
    let lookup = |i: u8| -> u16 {
        // SAFETY: the table has 256 entries.
        unsafe { *tlut.add(i as usize) }
    };
    let mut offset = 4 + bm_h.max(0) as usize;
    for j in 0..v0 as usize {
        offset += read(4 + j) as usize;
    }
    let mut texels = vec![0u16; (dest_width * dest_height) as usize];
    let mut line = Vec::new();
    // `j` indexes the per-line length table from the first decoded line on.
    for (k, j) in (0..height.min(dest_height) as usize).zip(v0 as usize..) {
        line.clear();
        let mut at = offset;
        loop {
            let c = read(at);
            if c == 0xe0 {
                break;
            }
            if c > 0xe0 {
                line.extend(std::iter::repeat_n(lookup(read(at + 1)), (c & 0x1f) as usize));
                at += 2;
            } else {
                line.push(lookup(c));
                at += 1;
            }
        }
        for x in 0..dest_width as usize {
            texels[k * dest_width as usize + x] = line.get(u0 as usize + x).copied().unwrap_or(0);
        }
        offset += read(4 + j) as usize;
    }
    if height < dest_height && height > 0 {
        let (last, rest) = texels.split_at_mut((height * dest_width) as usize);
        rest[..dest_width as usize].copy_from_slice(&last[last.len() - dest_width as usize..]);
    }
    let bytes: Vec<u8> = texels.iter().flat_map(|t| t.to_le_bytes()).collect();
    typed!("ConvertAndDownloadRle", tmu: Tmu, lod: Lod, large: Lod, aspect: Aspect, format: TextureFormat => {
        let info = TexInfo { small_lod: lod, large_lod: large, aspect, format };
        glide(|g| g.tex_download_mipmap(tmu, address, even_odd, info, &bytes))
    })
}

// ---- gu texture manager and 3DF files --------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn guTexAllocateMemory(
    tmu: c_int,
    even_odd: u32,
    width: c_int,
    height: c_int,
    format: c_int,
    mipmap: c_int,
    small: c_int,
    large: c_int,
    aspect: c_int,
    clamp_s: c_int,
    clamp_t: c_int,
    min_filter: c_int,
    mag_filter: c_int,
    lod_bias: f32,
    trilinear: FxBool,
) -> u32 {
    let null = rustyglide::gutex::NULL_MIPMAP;
    let call = "guTexAllocateMemory";
    let Some(tmu) = arg::<Tmu>(call, tmu) else { return null };
    let Some(attrs) =
        attributes(call, width, height, format, mipmap, small, large, aspect, clamp_s, clamp_t, min_filter, mag_filter)
    else {
        return null;
    };
    with(|s| s.glide.gu_tex_allocate(tmu, even_odd & 0xff, attrs, lod_bias, trilinear != 0))
}

fn attributes(
    call: &str,
    width: c_int,
    height: c_int,
    format: c_int,
    mipmap: c_int,
    small: c_int,
    large: c_int,
    aspect: c_int,
    clamp_s: c_int,
    clamp_t: c_int,
    min_filter: c_int,
    mag_filter: c_int,
) -> Option<Attributes> {
    Some(Attributes {
        width,
        height,
        format: arg(call, format)?,
        mipmap: arg(call, mipmap)?,
        small_lod: arg(call, small)?,
        large_lod: arg(call, large)?,
        aspect: arg(call, aspect)?,
        clamp_s: arg(call, clamp_s)?,
        clamp_t: arg(call, clamp_t)?,
        min_filter: arg(call, min_filter)?,
        mag_filter: arg(call, mag_filter)?,
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexChangeAttributes(
    id: u32,
    width: c_int,
    height: c_int,
    format: c_int,
    mipmap: c_int,
    small: c_int,
    large: c_int,
    aspect: c_int,
    clamp_s: c_int,
    clamp_t: c_int,
    min_filter: c_int,
    mag_filter: c_int,
) -> FxBool {
    let call = "guTexChangeAttributes";
    let Some(attrs) =
        attributes(call, width, height, format, mipmap, small, large, aspect, clamp_s, clamp_t, min_filter, mag_filter)
    else {
        return 0;
    };
    glide(|g| g.gu_tex_change_attributes(id, attrs) as FxBool)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guTexDownloadMipMap(id: u32, src: *const c_void, ncc: *const GuNccTable) {
    glide(|g| {
        let Some(info) = g.gu_tex_info(id).copied() else { return };
        let (Ok(small), Ok(large), Ok(aspect), Ok(format)) = (
            Lod::try_from(info.lod_min as u32),
            Lod::try_from(info.lod_max as u32),
            Aspect::try_from(info.aspect_ratio as u32),
            TextureFormat::try_from(info.format as u32),
        ) else {
            return;
        };
        let size = host_size(&TexInfo { small_lod: small, large_lod: large, aspect, format });
        // SAFETY: the game passes every level of the texture and an optional NCC table.
        let (data, table) = unsafe { (slice(src.cast::<u8>(), size), ncc.as_ref().map(GuNccTable::table)) };
        g.gu_tex_download_mipmap(id, data, table.as_ref());
    });
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guTexDownloadMipMapLevel(id: u32, lod: c_int, src: *mut *const c_void) {
    let Some(lod) = arg::<Lod>("guTexDownloadMipMapLevel", lod) else { return };
    // SAFETY: the game passes a pointer to its level data pointer.
    let Some(ptr) = (unsafe { src.as_mut() }) else { return };
    glide(|g| {
        let Some(info) = g.gu_tex_info(id).copied() else { return };
        let (Ok(aspect), Ok(format)) =
            (Aspect::try_from(info.aspect_ratio as u32), TextureFormat::try_from(info.format as u32))
        else {
            return;
        };
        let size = (texture::level_host_texels(lod, aspect) * format.bytes_per_texel()) as usize;
        // SAFETY: `*ptr` holds one level.
        let data = unsafe { slice(ptr.cast::<u8>(), size) };
        let used = g.gu_tex_download_level(id, lod, data);
        // SAFETY: advancing within the caller's buffer, as Glide does.
        *ptr = unsafe { ptr.cast::<u8>().add(used) }.cast();
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexGetMipMapInfo(id: u32) -> *mut GrMipMapInfo {
    glide(|g| g.gu_tex_info(id).map(|i| i as *mut GrMipMapInfo)).unwrap_or(std::ptr::null_mut())
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexGetCurrentMipMap(tmu: c_int) -> u32 {
    let Some(tmu) = arg::<Tmu>("guTexGetCurrentMipMap", tmu) else { return rustyglide::gutex::NULL_MIPMAP };
    with(|s| s.glide.gu_tex_current(tmu))
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexMemQueryAvail(tmu: c_int) -> u32 {
    typed!("guTexMemQueryAvail", tmu: Tmu => glide(|g| g.gu_tex_mem_query_avail(tmu)))
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexMemReset() {
    glide(|g| g.gu_tex_mem_reset());
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexSource(id: u32) {
    glide(|g| g.gu_tex_source(id));
}

#[unsafe(no_mangle)]
pub extern "system" fn guTexCreateColorMipMap() -> *mut c_void {
    std::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfGetInfo(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: forwarded from the caller.
    unsafe { gu3df_get_info(name, info, |l| l as c_int, |a| a as c_int) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfLoad(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: forwarded from the caller.
    unsafe { gu3df_load(name, info) }
}

// ---- Linear frame buffer ----------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grLfbWriteRegion(
    buffer: c_int,
    x: u32,
    y: u32,
    format: u32,
    width: u32,
    height: u32,
    stride: i32,
    src: *const c_void,
) -> FxBool {
    // SAFETY: forwarded from the caller.
    unsafe { lfb_write_region(buffer, x, y, format, width, height, stride, src) }
}

// ---- Utilities -------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn guEndianSwapWords(value: u32) -> u32 {
    value.rotate_left(16)
}

#[unsafe(no_mangle)]
pub extern "system" fn guEndianSwapBytes(value: u32) -> u32 {
    (value as u16).swap_bytes() as u32
}

/// Encodes 16-bit pixels as runs of `length << 16 | color`; returns the bytes
/// written, or that would be written when `dst` is null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn guEncodeRLE16(dst: *mut c_void, src: *const c_void, width: u32, height: u32) -> c_int {
    // SAFETY: `src` holds width * height pixels.
    let pixels = unsafe { slice(src.cast::<u16>(), width as usize * height as usize) };
    let mut runs = Vec::new();
    let mut i = 0;
    while i < pixels.len() {
        let mut length = 1;
        while i + length < pixels.len() && pixels[i + length] == pixels[i] && length < 0x7fff {
            length += 1;
        }
        runs.push((length as u32) << 16 | pixels[i] as u32);
        i += length;
    }
    if !dst.is_null() {
        // SAFETY: the caller sized `dst` with a previous null-destination call.
        unsafe { std::ptr::copy_nonoverlapping(runs.as_ptr(), dst.cast::<u32>(), runs.len()) };
    }
    (runs.len() * 4) as c_int
}

#[unsafe(no_mangle)]
pub extern "system" fn guMovieStart() {}

#[unsafe(no_mangle)]
pub extern "system" fn guMovieStop() {}

#[unsafe(no_mangle)]
pub extern "system" fn guMovieSetName(_name: *const c_char) {}

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
