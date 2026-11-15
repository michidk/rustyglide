//! Shared C ABI plumbing for the Glide front ends (`glide2x`, `glide3x`):
//! the global context, argument conversion, and the exports both APIs
//! share, expanded into each library by [`common_exports!`].
#![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]

pub use log;
pub use rustyglide;
use rustyglide::gutex::GuNccTable;
use rustyglide::texture::{self, NccData, TexInfo};
use rustyglide::types::*;
use rustyglide::{Glide, GrVertex, Target};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::sync::Mutex;

pub type FxBool = c_int;
pub type ErrorCallback = extern "C" fn(*const c_char, FxBool);

pub struct State {
    pub glide: Glide,
    pub error_callback: Option<ErrorCallback>,
    pub retrace: bool,
    pub video_line: u32,
}

/// The global Glide context.
pub struct Global(pub Option<State>);

// SAFETY: Glide is a single-context API; every access goes through the mutex
// below. The non-`Send` parts are raw pointers into the game's own memory
// (texture info records) and a present hook this library never installs.
unsafe impl Send for Global {}

pub static GLOBAL: Mutex<Global> = Mutex::new(Global(None));

pub fn with<R: Default>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = GLOBAL.lock().unwrap_or_else(|e| e.into_inner());
    match guard.0.as_mut() {
        Some(state) => f(state),
        None => R::default(),
    }
}

pub fn glide<R: Default>(f: impl FnOnce(&mut Glide) -> R) -> R {
    with(|s| f(&mut s.glide))
}

/// Converts a raw value, logging and returning `None` when it is invalid.
pub fn arg<T: TryFrom<u32, Error = InvalidValue>>(call: &str, value: impl Into<i64>) -> Option<T> {
    let value = value.into() as u32;
    match T::try_from(value) {
        Ok(v) => Some(v),
        Err(e) => {
            log::warn!("{call}: {e}");
            None
        }
    }
}

#[macro_export]
macro_rules! typed {
    ($call:literal, $($v:ident: $t:ty),+ => $body:expr) => {{
        $(let Some($v) = $crate::arg::<$t>($call, $v) else { return Default::default() };)+
        $body
    }};
}

/// A shared view of `len` elements at `ptr`, empty for null.
pub unsafe fn slice<'a, T>(ptr: *const T, len: usize) -> &'a [T] {
    if ptr.is_null() || len == 0 {
        &[]
    } else {
        // SAFETY: the caller guarantees `ptr` points at `len` readable elements.
        unsafe { std::slice::from_raw_parts(ptr, len) }
    }
}

pub unsafe fn vertex<'a>(ptr: *const GrVertex) -> Option<&'a GrVertex> {
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

pub unsafe fn tex_info(call: &str, info: *const GrTexInfo) -> Option<(TexInfo, *mut c_void)> {
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

pub fn host_size(info: &TexInfo) -> usize {
    (info.large_lod as u32..=info.small_lod as u32)
        .filter_map(Lod::from_index)
        .map(|l| (texture::level_host_texels(l, info.aspect) * info.format.bytes_per_texel()) as usize)
        .sum()
}
/// Bytes of Glide 2.4x's `GrState`.
pub const GR_STATE_SIZE: usize = 312;
#[cfg(windows)]
pub fn window_target(hwnd: usize, size: Option<(u32, u32)>) -> Target {
    use raw_window_handle::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GWLP_HINSTANCE, GetClientRect, GetWindowLongPtrW};
    // SAFETY: plain Win32 queries on a window handle the game owns.
    let hwnd = if hwnd == 0 { unsafe { GetActiveWindow() as usize } } else { hwnd };
    let Some(handle) = std::num::NonZeroIsize::new(hwnd as isize) else {
        return Target::Headless { size: size.unwrap_or((640, 480)) };
    };
    let mut rect = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    // SAFETY: as above; `rect` is a valid out pointer.
    unsafe { GetClientRect(hwnd as _, &mut rect) };
    let client = ((rect.right - rect.left).max(1) as u32, (rect.bottom - rect.top).max(1) as u32);
    // Vulkan surfaces need the window's module instance as well as the window.
    let mut win32 = Win32WindowHandle::new(handle);
    // SAFETY: as above.
    let instance = unsafe { GetWindowLongPtrW(hwnd as _, GWLP_HINSTANCE) };
    win32.hinstance = std::num::NonZeroIsize::new(instance as isize);
    Target::Window {
        window: RawWindowHandle::Win32(win32),
        display: RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
        size: client,
    }
}
#[cfg(not(windows))]
pub fn window_target(_hwnd: usize, size: Option<(u32, u32)>) -> Target {
    // shortcut: no own window outside Windows yet; frames render off screen.
    // Hosts that own a window use the `rustyglide` crate directly.
    log::warn!("glide2x: no window support on this platform yet; rendering off screen");
    Target::Headless { size: size.unwrap_or((640, 480)) }
}
pub unsafe fn fog_out<'a>(table: *mut u8) -> Option<&'a mut [u8; FOG_TABLE_SIZE]> {
    // SAFETY: Glide 2 fog tables have 64 entries; null is rejected.
    unsafe { table.cast::<[u8; FOG_TABLE_SIZE]>().as_mut() }
}
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
pub unsafe fn download_table(call: &str, tmu: c_int, kind: u32, data: *const c_void, start: usize, end: usize) {
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

/// Expands the exports Glide 2 and Glide 3 share into the calling crate, as
/// `pub mod common` re-exported at its root.
#[macro_export]
macro_rules! common_exports {
    () => {
        pub mod common {
            #![allow(non_snake_case, clippy::missing_safety_doc, clippy::too_many_arguments)]
            use $crate::rustyglide::combine::{Combine, TexCombine};
            use $crate::rustyglide::types::*;
            use $crate::rustyglide::{Blend, Glide, Settings};
            use $crate::*;
            use std::ffi::{c_char, c_int, c_void};

            #[unsafe(no_mangle)]
            pub extern "system" fn grGlideInit() {
                $crate::rustyglide::logging::init_from_env();
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
            pub extern "system" fn grSplash(_x: f32, _y: f32, _w: f32, _h: f32, _frame: u32) {}

            #[unsafe(no_mangle)]
            pub extern "system" fn grErrorSetCallback(callback: Option<ErrorCallback>) {
                with(|s| s.error_callback = callback);
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grSstSelect(_which: c_int) {}

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
                $crate::log::info!("grSstWinOpen: window {hwnd:#x}, {resolution:?}, {color_format:?}, {origin:?}");
                glide(|g| match g.sst_win_open(target, resolution, color_format, origin) {
                    Ok(()) => 1,
                    Err(e) => {
                        $crate::log::error!("grSstWinOpen: {e}");
                        0
                    }
                })
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grSstVidMode(_which: u32, _mode: *mut c_void) {}

            #[unsafe(no_mangle)]
            pub extern "system" fn grSstOrigin(origin: c_int) {
                typed!("grSstOrigin", origin: Origin => glide(|g| g.sst_origin(origin)))
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grCheckForRoom(_n: i32) {}

            #[unsafe(no_mangle)]
            pub extern "system" fn grBufferClear(color: u32, alpha: u32, depth: u32) {
                glide(|g| g.buffer_clear(color, alpha as u8, depth as u16));
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grBufferSwap(interval: c_int) {
                glide(|g| g.buffer_swap(interval.max(0) as u32));
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
            pub extern "system" fn grAlphaControlsITRGBLighting(enable: FxBool) {
                glide(|g| g.alpha_controls_itrgb_lighting(enable != 0));
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grConstantColorValue(color: u32) {
                glide(|g| g.constant_color(color));
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
            pub extern "system" fn grFogColorValue(color: u32) {
                glide(|g| g.fog_color(color));
            }

            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn grFogTable(table: *const u8) {
                // SAFETY: Glide 2 fog tables have 64 entries.
                let Ok(table) = <[u8; FOG_TABLE_SIZE]>::try_from(unsafe { slice(table, FOG_TABLE_SIZE) }) else { return };
                glide(|g| g.fog_table(&table));
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

            #[unsafe(no_mangle)]
            pub extern "system" fn grTexMinAddress(tmu: c_int) -> u32 {
                typed!("grTexMinAddress", tmu: Tmu => glide(|g| g.tex_min_address(tmu)))
            }

            #[unsafe(no_mangle)]
            pub extern "system" fn grTexMaxAddress(tmu: c_int) -> u32 {
                typed!("grTexMaxAddress", tmu: Tmu => glide(|g| g.tex_max_address(tmu)))
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
            pub extern "system" fn grTexMultibase(tmu: c_int, enable: FxBool) {
                typed!("grTexMultibase", tmu: Tmu => glide(|g| g.tex_multibase(tmu, enable != 0)))
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
        }
        pub use common::*;
    };
}

// ---- Helpers the front ends export under their own signatures -----------------

/// `Gu3dfInfo`: header, the NCC/palette union, data pointer, size.
#[repr(C)]
pub struct Gu3dfInfo {
    pub width: u32,
    pub height: u32,
    pub small_lod: c_int,
    pub large_lod: c_int,
    pub aspect: c_int,
    pub format: c_int,
    pub table: [u32; 256],
    pub data: *mut c_void,
    pub mem_required: u32,
}

pub unsafe fn read_file(name: *const c_char) -> Option<Vec<u8>> {
    if name.is_null() {
        return None;
    }
    // SAFETY: Glide passes a NUL-terminated file name.
    let name = unsafe { CStr::from_ptr(name) };
    std::fs::read(name.to_str().ok()?).ok()
}

/// `gu3dfGetInfo`; `lod` and `aspect` give the API's numbering of Glide 2 values.
pub unsafe fn gu3df_get_info(
    name: *const c_char,
    info: *mut Gu3dfInfo,
    lod: fn(Lod) -> c_int,
    aspect: fn(Aspect) -> c_int,
) -> FxBool {
    // SAFETY: see read_file; `info` is the caller's Gu3dfInfo.
    let (Some(file), Some(info)) = (unsafe { read_file(name) }, unsafe { info.as_mut() }) else { return 0 };
    let Some((h, _)) = rustyglide::gu3df::parse_header(&file) else { return 0 };
    info.width = h.width;
    info.height = h.height;
    info.small_lod = lod(h.small_lod);
    info.large_lod = lod(h.large_lod);
    info.aspect = aspect(h.aspect);
    info.format = h.format as c_int;
    info.mem_required = h.mem_required;
    1
}

/// `gu3dfLoad`.
pub unsafe fn gu3df_load(name: *const c_char, info: *mut Gu3dfInfo) -> FxBool {
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

/// `grLfbWriteRegion` without a pixel-pipeline choice (writes bypass it).
pub unsafe fn lfb_write_region(
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
