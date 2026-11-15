//! The Glide 2.4x C ABI over `rustyglide`, built as `glide2x.dll` /
//! `libglide2x.so`.
//!
//! Every export converts raw C values to typed ones; an invalid value is
//! logged once and the call ignored, as 3dfx's release library does.
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

use rustyglide::combine::{Combine, TexCombine};
use rustyglide::gutex::{Attributes, GrMipMapInfo, GuNccTable};
use rustyglide::texture::{self, NccData, TexInfo};
use rustyglide::types::*;
use rustyglide::{Blend, Glide, GrVertex, Settings, Target};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

type FxBool = c_int;
type ErrorCallback = extern "C" fn(*const c_char, FxBool);

struct State {
    glide: Glide,
    error_callback: Option<ErrorCallback>,
    retrace: bool,
    video_line: u32,
}

/// The global Glide context.
struct Global(Option<State>);

// SAFETY: Glide is a single-context API; every access goes through the mutex
// below. The non-`Send` parts are raw pointers into the game's own memory
// (texture info records) and a present hook this library never installs.
unsafe impl Send for Global {}

static GLOBAL: Mutex<Global> = Mutex::new(Global(None));

fn with<R: Default>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    match guard.0.as_mut() {
        Some(state) => f(state),
        None => R::default(),
    }
}

fn glide<R: Default>(f: impl FnOnce(&mut Glide) -> R) -> R {
    with(|s| f(&mut s.glide))
}

/// Converts a raw value, logging and returning `None` when it is invalid.
fn arg<T: TryFrom<u32, Error = InvalidValue>>(call: &str, value: impl Into<i64>) -> Option<T> {
    let value = value.into() as u32;
    match T::try_from(value) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("{call}: {e}");
            None
        }
    }
}

macro_rules! typed {
    ($call:literal, $($v:ident: $t:ty),+ => $body:expr) => {{
        $(let Some($v) = arg::<$t>($call, $v) else { return Default::default() };)+
        $body
    }};
}

/// A shared view of `len` elements at `ptr`, empty for null.
unsafe fn slice<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller guarantees `ptr` points at `len` readable elements.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

unsafe fn vertex<'a>(ptr: *const GrVertex) -> Option<&'a GrVertex> {
    // SAFETY: Glide callers pass valid vertex pointers; null is rejected.
    unsafe { ptr.as_ref() }
}

/// `GrTexInfo`.
#[repr(C)]
pub struct GrTexInfo {
    pub small_lod: c_int,
    pub large_lod: c_int,
    pub aspect_ratio: c_int,
    pub format: c_int,
    pub data: *mut c_void,
}

unsafe fn tex_info(call: &str, info: *const GrTexInfo) -> Option<(TexInfo, *mut c_void)> {
    // SAFETY: Glide callers pass a valid GrTexInfo pointer; null is rejected.
    let i = unsafe { info.as_ref() }?;
    Some((
        TexInfo {
            small_lod: arg(call, i.small_lod)?,
            large_lod: arg(call, i.large_lod)?,
            aspect: arg(call, i.aspect_ratio)?,
            format: arg(call, i.format)?,
        },
        i.data,
    ))
}

fn host_size(info: &TexInfo) -> usize {
    (info.large_lod as u32..=info.small_lod as u32)
        .filter_map(Lod::from_index)
        .map(|l| (texture::level_host_texels(l, info.aspect) * info.format.bytes_per_texel()) as usize)
        .sum()
}

// ---- Initialization and the SST -----------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grGlideInit() {
    rustyglide::logging::init_from_env();
    let mut guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    if guard.0.is_none() {
        guard.0 = Some(State {
            glide: Glide::new(Settings::from_env()),
            error_callback: None,
            retrace: false,
            video_line: 0,
        });
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn grGlideShutdown() {
    let mut guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(mut s) = guard.0.take() {
        s.glide.sst_win_close();
    }
}

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

/// Bytes of Glide 2.4x's `GrState`.
const GR_STATE_SIZE: usize = 312;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGlideGetState(state: *mut c_void) {
    if state.is_null() {
        return;
    }
    // SAFETY: the game passes a GrState, 312 bytes.
    let out = unsafe { std::slice::from_raw_parts_mut(state.cast::<u8>(), GR_STATE_SIZE) };
    glide(|g| g.get_state(out));
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grGlideSetState(state: *const c_void) {
    // SAFETY: the game passes a GrState it got from grGlideGetState.
    let data = unsafe { slice(state.cast::<u8>(), GR_STATE_SIZE) };
    glide(|g| g.set_state(data));
}

#[unsafe(no_mangle)]
pub extern "system" fn grGlideShamelessPlug(_on: FxBool) {}

#[unsafe(no_mangle)]
pub extern "system" fn grSplash(_x: f32, _y: f32, _w: f32, _h: f32, _frame: u32) {}

#[unsafe(no_mangle)]
pub extern "system" fn grErrorSetCallback(callback: Option<ErrorCallback>) {
    with(|s| s.error_callback = callback);
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstSelect(_which: c_int) {}

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

#[cfg(windows)]
fn window_target(hwnd: usize, size: Option<(u32, u32)>) -> Target {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
    // SAFETY: plain Win32 queries on a window handle the game owns.
    let hwnd = if hwnd == 0 { unsafe { GetActiveWindow() as usize } } else { hwnd };
    let Some(handle) = std::num::NonZeroIsize::new(hwnd as isize) else {
        return Target::Headless { size: size.unwrap_or((640, 480)) };
    };
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: as above; `rect` is a valid out pointer.
    unsafe { GetClientRect(hwnd as _, &mut rect) };
    let client = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
    Target::Window {
        window: RawWindowHandle::Win32(Win32WindowHandle::new(handle)),
        display: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
        size: client,
    }
}

#[cfg(not(windows))]
fn window_target(_hwnd: usize, size: Option<(u32, u32)>) -> Target {
    // shortcut: no own window outside Windows yet; frames render off screen.
    // Hosts that own a window use the `rustyglide` crate directly.
    log::warn!("glide2x: no window support on this platform yet; rendering off screen");
    Target::Headless { size: size.unwrap_or((640, 480)) }
}

#[unsafe(no_mangle)]
pub extern "system" fn grSstWinOpen(
    hwnd: usize,
    resolution: c_int,
    _refresh: c_int,
    color_format: c_int,
    origin: c_int,
    _color_buffers: c_int,
    _aux_buffers: c_int,
) -> FxBool {
    let (Some(resolution), Some(color_format), Some(origin)) = (
        arg::<Resolution>("grSstWinOpen", resolution),
        arg::<ColorFormat>("grSstWinOpen", color_format),
        arg::<Origin>("grSstWinOpen", origin),
    ) else {
        return 0;
    };
    let target = window_target(hwnd, resolution.size());
    log::info!("grSstWinOpen: window {hwnd:#x}, {resolution:?}, {color_format:?}, {origin:?}");
    glide(|g| match g.sst_win_open(target, resolution, color_format, origin) {
        Ok(()) => 1,
        Err(e) => {
            log::error!("grSstWinOpen: {e}");
            0
        }
    })
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
pub extern "system" fn grSstVidMode(_which: u32, _mode: *mut c_void) {}

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
pub extern "system" fn grSstOrigin(origin: c_int) {
    typed!("grSstOrigin", origin: Origin => glide(|g| g.sst_origin(origin)))
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
pub extern "system" fn grCheckForRoom(_n: i32) {}

#[unsafe(no_mangle)]
pub extern "system" fn grParameterData(_param: u32, _components: u32, _type: u32, _offset: i32) {}

// ---- Buffers ---------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grBufferClear(color: u32, alpha: u32, depth: u32) {
    glide(|g| g.buffer_clear(color, alpha as u8, depth as u16));
}

#[unsafe(no_mangle)]
pub extern "system" fn grBufferSwap(interval: c_int) {
    glide(|g| g.buffer_swap(interval.max(0) as u32));
}

#[unsafe(no_mangle)]
pub extern "system" fn grBufferNumPending() -> c_int {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn grRenderBuffer(buffer: c_int) {
    typed!("grRenderBuffer", buffer: Buffer => glide(|g| g.render_buffer(buffer)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grColorMask(rgb: FxBool, alpha: FxBool) {
    glide(|g| g.color_mask(rgb != 0, alpha != 0));
}

#[unsafe(no_mangle)]
pub extern "system" fn grClipWindow(min_x: u32, min_y: u32, max_x: u32, max_y: u32) {
    glide(|g| g.clip_window(min_x, min_y, max_x, max_y));
}

// ---- Combine, blend and pixel tests ------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grColorCombine(function: c_int, factor: c_int, local: c_int, other: c_int, invert: FxBool) {
    typed!("grColorCombine", function: CombineFunction, factor: CombineFactor, local: CombineLocal, other: CombineOther =>
        glide(|g| g.color_combine(Combine::new(function, factor, local, other, invert != 0))))
}

#[unsafe(no_mangle)]
pub extern "system" fn grAlphaCombine(function: c_int, factor: c_int, local: c_int, other: c_int, invert: FxBool) {
    typed!("grAlphaCombine", function: CombineFunction, factor: CombineFactor, local: CombineLocal, other: CombineOther =>
        glide(|g| g.alpha_combine(Combine::new(function, factor, local, other, invert != 0))))
}

#[unsafe(no_mangle)]
pub extern "system" fn guColorCombineFunction(fnc: c_int) {
    typed!("guColorCombineFunction", fnc: ColorCombineFnc => glide(|g| g.gu_color_combine_function(fnc)))
}

#[unsafe(no_mangle)]
pub extern "system" fn guAlphaSource(source: c_int) {
    typed!("guAlphaSource", source: AlphaSource => glide(|g| g.gu_alpha_source(source)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grAlphaControlsITRGBLighting(enable: FxBool) {
    glide(|g| g.alpha_controls_itrgb_lighting(enable != 0));
}

#[unsafe(no_mangle)]
pub extern "system" fn grConstantColorValue(color: u32) {
    glide(|g| g.constant_color(color));
}

#[unsafe(no_mangle)]
pub extern "system" fn grConstantColorValue4(a: f32, r: f32, g: f32, b: f32) {
    glide(|gl| gl.constant_color4(a, r, g, b));
}

#[unsafe(no_mangle)]
pub extern "system" fn grAlphaBlendFunction(rgb_src: c_int, rgb_dst: c_int, alpha_src: c_int, alpha_dst: c_int) {
    typed!("grAlphaBlendFunction", rgb_src: BlendFactor, rgb_dst: BlendFactor, alpha_src: BlendFactor, alpha_dst: BlendFactor =>
        glide(|g| g.alpha_blend_function(Blend { rgb_src, rgb_dst, alpha_src, alpha_dst })))
}

#[unsafe(no_mangle)]
pub extern "system" fn grAlphaTestFunction(function: c_int) {
    typed!("grAlphaTestFunction", function: CmpFnc => glide(|g| g.alpha_test_function(function)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grAlphaTestReferenceValue(value: u32) {
    glide(|g| g.alpha_test_reference(value as u8));
}

#[unsafe(no_mangle)]
pub extern "system" fn grChromakeyMode(mode: c_int) {
    typed!("grChromakeyMode", mode: ChromakeyMode => glide(|g| g.chromakey_mode(mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grChromakeyValue(value: u32) {
    glide(|g| g.chromakey_value(value));
}

#[unsafe(no_mangle)]
pub extern "system" fn grCullMode(mode: c_int) {
    typed!("grCullMode", mode: CullMode => glide(|g| g.cull_mode(mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grDepthBiasLevel(level: i32) {
    glide(|g| g.depth_bias_level(level as i16));
}

#[unsafe(no_mangle)]
pub extern "system" fn grDepthBufferFunction(function: c_int) {
    typed!("grDepthBufferFunction", function: CmpFnc => glide(|g| g.depth_buffer_function(function)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grDepthBufferMode(mode: c_int) {
    typed!("grDepthBufferMode", mode: DepthBufferMode => glide(|g| g.depth_buffer_mode(mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grDepthMask(enable: FxBool) {
    glide(|g| g.depth_mask(enable != 0));
}

#[unsafe(no_mangle)]
pub extern "system" fn grDisableAllEffects() {
    glide(|g| g.disable_all_effects());
}

#[unsafe(no_mangle)]
pub extern "system" fn grDitherMode(mode: c_int) {
    typed!("grDitherMode", mode: DitherMode => glide(|g| g.dither_mode(mode)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grGammaCorrectionValue(value: f32) {
    glide(|g| g.gamma_correction(value));
}

// ---- Fog -------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "system" fn grFogColorValue(color: u32) {
    glide(|g| g.fog_color(color));
}

#[unsafe(no_mangle)]
pub extern "system" fn grFogMode(mode: c_int) {
    typed!("grFogMode", mode: FogMode => glide(|g| g.fog_mode(mode)))
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grFogTable(table: *const u8) {
    // SAFETY: Glide 2 fog tables have 64 entries.
    let Ok(table) = <[u8; FOG_TABLE_SIZE]>::try_from(unsafe { slice(table, FOG_TABLE_SIZE) }) else { return };
    glide(|g| g.fog_table(&table));
}

unsafe fn fog_out<'a>(table: *mut u8) -> Option<&'a mut [u8; FOG_TABLE_SIZE]> {
    // SAFETY: Glide 2 fog tables have 64 entries; null is rejected.
    unsafe { table.cast::<[u8; FOG_TABLE_SIZE]>().as_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guFogGenerateExp(table: *mut u8, density: f32) {
    // SAFETY: see fog_out.
    if let Some(t) = unsafe { fog_out(table) } {
        rustyglide::fog::generate_exp(t, density);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guFogGenerateExp2(table: *mut u8, density: f32) {
    // SAFETY: see fog_out.
    if let Some(t) = unsafe { fog_out(table) } {
        rustyglide::fog::generate_exp2(t, density);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn guFogGenerateLinear(table: *mut u8, near: f32, far: f32) {
    // SAFETY: see fog_out.
    if let Some(t) = unsafe { fog_out(table) } {
        rustyglide::fog::generate_linear(t, near, far);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn guFogTableIndexToW(i: c_int) -> f32 {
    rustyglide::fog::index_to_w(i.clamp(0, 63) as usize)
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
pub extern "system" fn grTexMinAddress(tmu: c_int) -> u32 {
    typed!("grTexMinAddress", tmu: Tmu => glide(|g| g.tex_min_address(tmu)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexMaxAddress(tmu: c_int) -> u32 {
    typed!("grTexMaxAddress", tmu: Tmu => glide(|g| g.tex_max_address(tmu)))
}

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
pub extern "system" fn grTexClampMode(tmu: c_int, s: c_int, t: c_int) {
    typed!("grTexClampMode", tmu: Tmu, s: ClampMode, t: ClampMode => glide(|g| g.tex_clamp_mode(tmu, s, t)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexFilterMode(tmu: c_int, min: c_int, mag: c_int) {
    typed!("grTexFilterMode", tmu: Tmu, min: FilterMode, mag: FilterMode => glide(|g| g.tex_filter_mode(tmu, min, mag)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexMipMapMode(tmu: c_int, mode: c_int, lod_blend: FxBool) {
    typed!("grTexMipMapMode", tmu: Tmu, mode: MipMapMode => glide(|g| g.tex_mipmap_mode(tmu, mode, lod_blend != 0)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexLodBiasValue(tmu: c_int, bias: f32) {
    typed!("grTexLodBiasValue", tmu: Tmu => glide(|g| g.tex_lod_bias(tmu, bias)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexDetailControl(tmu: c_int, lod_bias: c_int, scale: u32, max: f32) {
    typed!("grTexDetailControl", tmu: Tmu => glide(|g| g.tex_detail_control(tmu, lod_bias, scale as u8, max)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grTexCombine(
    tmu: c_int,
    rgb_function: c_int,
    rgb_factor: c_int,
    alpha_function: c_int,
    alpha_factor: c_int,
    rgb_invert: FxBool,
    alpha_invert: FxBool,
) {
    typed!("grTexCombine", tmu: Tmu, rgb_function: CombineFunction, rgb_factor: CombineFactor,
        alpha_function: CombineFunction, alpha_factor: CombineFactor =>
        glide(|g| g.tex_combine(tmu, TexCombine::new(rgb_function, rgb_factor, alpha_function, alpha_factor, rgb_invert != 0, alpha_invert != 0))))
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
pub extern "system" fn grTexMultibase(tmu: c_int, enable: FxBool) {
    typed!("grTexMultibase", tmu: Tmu => glide(|g| g.tex_multibase(tmu, enable != 0)))
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

unsafe fn download_table(call: &str, tmu: c_int, kind: u32, data: *const c_void, start: usize, end: usize) {
    let (Some(tmu), Some(kind)) = (arg::<Tmu>(call, tmu), arg::<TexTable>(call, kind)) else { return };
    match kind {
        TexTable::Palette => {
            // SAFETY: palettes are 256 FxU32 entries.
            let entries = unsafe { slice(data.cast::<u32>(), 256) };
            if let Some(range) = entries.get(start..=end.min(255)) {
                glide(|g| g.tex_download_palette(tmu, range, start));
            }
        }
        TexTable::Ncc0 | TexTable::Ncc1 => {
            // SAFETY: NCC tables are a GuNccTable.
            let bytes = unsafe { slice(data.cast::<u8>(), std::mem::size_of::<GuNccTable>()) };
            let which = if kind == TexTable::Ncc0 { NccTable::Ncc0 } else { NccTable::Ncc1 };
            if let Some(table) = NccData::from_bytes(bytes) {
                glide(|g| g.tex_download_ncc(tmu, which, &table));
            }
        }
    }
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

/// `Gu3dfInfo`: header, the NCC/palette union, data pointer, size.
#[repr(C)]
pub struct Gu3dfInfo {
    width: u32,
    height: u32,
    small_lod: c_int,
    large_lod: c_int,
    aspect: c_int,
    format: c_int,
    table: [u32; 256],
    data: *mut c_void,
    mem_required: u32,
}

unsafe fn read_file(name: *const c_char) -> Option<Vec<u8>> {
    if name.is_null() {
        return None;
    }
    // SAFETY: Glide passes a NUL-terminated file name.
    let name = unsafe { CStr::from_ptr(name) };
    std::fs::read(name.to_str().ok()?).ok()
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfGetInfo(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: see read_file; `info` is the caller's Gu3dfInfo.
    let (Some(file), Some(info)) = (unsafe { read_file(name) }, unsafe { info.as_mut() }) else { return 0 };
    let Some((h, _)) = rustyglide::gu3df::parse_header(&file) else { return 0 };
    info.width = h.width;
    info.height = h.height;
    info.small_lod = h.small_lod as c_int;
    info.large_lod = h.large_lod as c_int;
    info.aspect = h.aspect as c_int;
    info.format = h.format as c_int;
    info.mem_required = h.mem_required;
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn gu3dfLoad(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
    // SAFETY: see read_file; `info` is the caller's Gu3dfInfo with `data` sized by gu3dfGetInfo.
    let (Some(file), Some(info)) = (unsafe { read_file(name) }, unsafe { info.as_mut() }) else { return 0 };
    let Some((_, table, data)) = rustyglide::gu3df::load(&file) else { return 0 };
    match table {
        rustyglide::gu3df::Table::None => {}
        rustyglide::gu3df::Table::Palette(p) => info.table = *p,
        rustyglide::gu3df::Table::Ncc(t) => {
            let ncc = GuNccTable::from_table(&t);
            // SAFETY: GuNccTable (112 bytes) fits the 1024-byte table union.
            unsafe { std::ptr::write_unaligned(info.table.as_mut_ptr().cast::<GuNccTable>(), ncc) };
        }
    }
    if info.data.is_null() {
        return 0;
    }
    let len = data.len().min(info.mem_required as usize);
    // SAFETY: the caller allocated mem_required bytes at `data`.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), info.data.cast::<u8>(), len) };
    1
}

// ---- Linear frame buffer ----------------------------------------------------------

/// `GrLfbInfo_t`.
#[repr(C)]
#[derive(Debug)]
pub struct GrLfbInfo {
    pub size: c_int,
    pub lfb_ptr: *mut c_void,
    pub stride_in_bytes: u32,
    pub write_mode: c_int,
    pub origin: c_int,
}

impl Default for GrLfbInfo {
    fn default() -> Self {
        Self {
            size: std::mem::size_of::<Self>() as c_int,
            lfb_ptr: std::ptr::null_mut(),
            stride_in_bytes: 0,
            write_mode: 0,
            origin: 0,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grLfbLock(
    lock_type: u32,
    buffer: c_int,
    write_mode: c_int,
    origin: c_int,
    pixel_pipeline: FxBool,
    info: *mut GrLfbInfo,
) -> FxBool {
    let call = "grLfbLock";
    // SAFETY: the caller passes a GrLfbInfo; its `size` field varies by version and is not trusted.
    let Some(info) = (unsafe { info.as_mut() }) else { return 0 };
    let (Some(buffer), Some(origin)) = (arg::<Buffer>(call, buffer), arg::<Origin>(call, origin)) else { return 0 };
    let write_mode = arg::<LfbWriteMode>(call, write_mode).unwrap_or(LfbWriteMode::Any);
    let Some(lock) = glide(|g| g.lfb_lock(lock_type, buffer, write_mode, origin, pixel_pipeline != 0)) else {
        return 0;
    };
    info.lfb_ptr = lock.data.cast();
    info.stride_in_bytes = lock.stride;
    info.write_mode = lock.write_mode as c_int;
    info.origin = lock.origin as c_int;
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn grLfbUnlock(lock_type: u32, buffer: c_int) -> FxBool {
    typed!("grLfbUnlock", buffer: Buffer => glide(|g| g.lfb_unlock(lock_type, buffer) as FxBool))
}

#[unsafe(no_mangle)]
pub extern "system" fn grLfbConstantAlpha(alpha: u32) {
    glide(|g| g.lfb_constant_alpha(alpha as u8));
}

#[unsafe(no_mangle)]
pub extern "system" fn grLfbConstantDepth(depth: u32) {
    glide(|g| g.lfb_constant_depth(depth as u16));
}

#[unsafe(no_mangle)]
pub extern "system" fn grLfbWriteColorFormat(format: c_int) {
    typed!("grLfbWriteColorFormat", format: ColorFormat => glide(|g| g.lfb_write_color_format(format)))
}

#[unsafe(no_mangle)]
pub extern "system" fn grLfbWriteColorSwizzle(swizzle_bytes: FxBool, swap_words: FxBool) {
    glide(|g| g.lfb_write_color_swizzle(swizzle_bytes != 0, swap_words != 0));
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn grLfbReadRegion(
    buffer: c_int,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    stride: u32,
    dst: *mut c_void,
) -> FxBool {
    let Some(buffer) = arg::<Buffer>("grLfbReadRegion", buffer) else { return 0 };
    if dst.is_null() || height == 0 {
        return 0;
    }
    let len = stride as usize * (height as usize - 1) + width as usize * 2;
    // SAFETY: the caller provides `height` rows of `stride` bytes.
    let dst = unsafe { std::slice::from_raw_parts_mut(dst.cast::<u8>(), len) };
    glide(|g| g.lfb_read_region(buffer, x, y, width, height, stride as usize, dst) as FxBool)
}

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
    let Some(buffer) = arg::<Buffer>("grLfbWriteRegion", buffer) else { return 0 };
    if src.is_null() || height == 0 {
        return 0;
    }
    let src = src.cast::<u8>();
    let data: Vec<u8> = if format == lfb_src::RLE16 {
        // Runs cover width x height texels; read them until they do.
        let mut runs = Vec::new();
        let mut covered = 0u64;
        let mut at = 0usize;
        while covered < width as u64 * height as u64 {
            // SAFETY: the RLE stream covers the region.
            let run = unsafe { std::ptr::read_unaligned(src.add(at).cast::<u32>()) };
            runs.extend_from_slice(&run.to_le_bytes());
            covered += (run >> 16).max(1) as u64;
            at += 4;
        }
        runs
    } else {
        let bpp = LfbWriteMode::try_from(format).map_or(2, |m| m.bytes_per_pixel()) as usize;
        let row = width as usize * bpp;
        // Copy rows into a positive-stride buffer; Glide allows negative strides.
        (0..height as isize)
            .flat_map(|r| {
                // SAFETY: the caller provides `height` rows `stride` bytes apart.
                unsafe { std::slice::from_raw_parts(src.offset(r * stride as isize), row) }.iter().copied()
            })
            .collect()
    };
    let stride = if format == lfb_src::RLE16 { 0 } else { (data.len() / height as usize) as i32 };
    glide(|g| g.lfb_write_region(buffer, x, y, format, width, height, stride, &data) as FxBool)
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
        let source = include_str!("lib.rs");
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
