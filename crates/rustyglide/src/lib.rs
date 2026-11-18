//! 3dfx Glide on wgpu.
//!
//! [`Glide`] combines the Glide state machine of `rustyglide-core` with the
//! wgpu renderer. State and drawing calls are the core's methods, reached
//! through `Deref`; the calls that need the GPU (opening the window, swapping,
//! LFB locks) are methods of `Glide` itself.
//!
//! ```no_run
//! # fn main() -> Result<(), rustyglide::Error> {
//! use rustyglide::{Glide, Settings, Target, types::*};
//! let mut glide = Glide::new(Settings::from_env());
//! glide.sst_win_open(Target::Headless { size: (640, 480) }, Resolution::R640x480, ColorFormat::Argb, Origin::UpperLeft)?;
//! glide.buffer_clear(0, 0, 0xffff);
//! glide.buffer_swap(1);
//! # Ok(()) }
//! ```

pub mod logging;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use rustyglide_core::types::{Buffer, ColorFormat, FilterMode, LfbWriteMode, Origin, Resolution, lock};
pub use rustyglide_core::*;
pub use rustyglide_wgpu::{Error, PresentFrame, PresentHook, wgpu};
use rustyglide_wgpu::{Gpu, Renderer};

/// Settings outside the Glide API.
#[derive(Debug, Clone)]
pub struct Settings {
    pub core: Config,
    /// Render at this multiple of the Glide resolution.
    pub scale: u32,
    /// Show the frame at the largest whole multiple that fits the window,
    /// with nearest filtering, instead of stretching it to fit.
    pub integer_scale: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { core: Config::default(), scale: 1, integer_scale: false }
    }
}

impl Settings {
    /// Defaults overridden by `RUSTYGLIDE_*` environment variables:
    /// `SCALE`, `INTEGER_SCALE`, `TMUS`, `TEXTURE_MEMORY_MB`, `FILTER`
    /// (`point` or `bilinear`) and `NO_MIPMAPS`. The backend is chosen with wgpu's `WGPU_BACKEND`.
    pub fn from_env() -> Self {
        // shortcut: environment only; a config file can follow when a game needs per-title settings.
        let var = |name: &str| std::env::var(format!("RUSTYGLIDE_{name}")).ok();
        let number = |name: &str| var(name).and_then(|v| v.parse::<u32>().ok());
        let mut s = Self::default();
        if let Some(v) = number("SCALE") {
            s.scale = v.clamp(1, 8);
        }
        if let Some(v) = number("TMUS") {
            s.core.tmus = v.clamp(1, 2);
        }
        if let Some(v) = number("TEXTURE_MEMORY_MB") {
            s.core.tmu_memory = v.clamp(1, 64) << 20;
        }
        s.core.force_filter = match var("FILTER").as_deref() {
            Some("point") => Some(FilterMode::PointSampled),
            Some("bilinear") => Some(FilterMode::Bilinear),
            _ => None,
        };
        s.core.disable_mipmaps = var("NO_MIPMAPS").is_some_and(|v| v != "0");
        s.integer_scale = var("INTEGER_SCALE").is_some_and(|v| v != "0");
        s
    }
}

/// Where Glide draws.
pub enum Target {
    /// No window; frames are only readable through LFB reads.
    Headless { size: (u32, u32) },
    /// A window owned by the caller.
    Window { window: RawWindowHandle, display: RawDisplayHandle, size: (u32, u32) },
}

impl Target {
    /// A window target from any raw-window-handle window (winit, SDL2, ...).
    ///
    /// # Safety
    ///
    /// The window must outlive the `Glide` that draws into it, or
    /// `sst_win_close` must be called before it is destroyed.
    pub unsafe fn window(window: &(impl HasWindowHandle + HasDisplayHandle), size: (u32, u32)) -> Result<Self, Error> {
        let w = window.window_handle().map_err(|e| Error::Surface(e.to_string()))?;
        let d = window.display_handle().map_err(|e| Error::Surface(e.to_string()))?;
        Ok(Self::Window { window: w.as_raw(), display: d.as_raw(), size })
    }

    fn size(&self) -> (u32, u32) {
        match self {
            Self::Headless { size } | Self::Window { size, .. } => *size,
        }
    }
}

/// A raw display handle wgpu may keep in its instance.
#[derive(Debug)]
struct Display(RawDisplayHandle);

// SAFETY: the handle is only an identifier of the caller's display
// connection; `Target::window`'s contract keeps it alive while wgpu uses it.
unsafe impl Send for Display {}
// SAFETY: as above; wgpu only reads it.
unsafe impl Sync for Display {}

impl HasDisplayHandle for Display {
    fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
        // SAFETY: the display outlives this value per `Target::window`.
        Ok(unsafe { raw_window_handle::DisplayHandle::borrow_raw(self.0) })
    }
}

/// Glide: state, texture memory and a renderer.
pub struct Glide {
    core: Context,
    settings: Settings,
    renderer: Option<Renderer>,
    hook: Option<PresentHook>,
    /// The last color buffer read back, valid until anything is drawn. Hosts
    /// such as DOSBox lock the frame buffer for reading many times per frame,
    /// and every GPU read-back stalls rendering.
    read_cache: Option<(Buffer, Vec<u8>)>,
}

impl std::ops::Deref for Glide {
    type Target = Context;
    fn deref(&self) -> &Context {
        &self.core
    }
}

impl std::ops::DerefMut for Glide {
    fn deref_mut(&mut self) -> &mut Context {
        &mut self.core
    }
}

impl Glide {
    pub fn new(settings: Settings) -> Self {
        Self { core: Context::new(settings.core.clone()), settings, renderer: None, hook: None, read_cache: None }
    }

    /// `grSstWinOpen`. A `Resolution::None` takes the target's size.
    pub fn sst_win_open(
        &mut self,
        target: Target,
        resolution: Resolution,
        color_format: ColorFormat,
        origin: Origin,
    ) -> Result<(), Error> {
        let size = resolution.size().unwrap_or(target.size());
        let scale = self.settings.scale;
        let mut renderer = match target {
            Target::Headless { .. } => Renderer::new(&Gpu::headless()?, None, size, scale)?,
            Target::Window { window, display, size: window_size } => {
                let mut descriptor =
                    wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(Display(display)));
                rustyglide_wgpu::configure(&mut descriptor);
                let instance = wgpu::Instance::new(descriptor);
                // SAFETY: `Target::window`'s contract keeps both handles valid
                // for as long as this renderer and its surface live.
                let surface = unsafe {
                    instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                        raw_display_handle: Some(display),
                        raw_window_handle: window,
                    })
                }
                .map_err(|e| Error::Surface(e.to_string()))?;
                let gpu = Gpu::new(instance, Some(&surface))?;
                let mut renderer = Renderer::new(&gpu, Some(surface), size, scale)?;
                renderer.resize(window_size.0, window_size.1);
                renderer
            }
        };
        renderer.integer_scale = self.settings.integer_scale;
        self.renderer = Some(renderer);
        self.core.win_open(size.0, size.1, color_format, origin);
        Ok(())
    }

    pub fn sst_win_close(&mut self) {
        self.core.take_commands();
        self.core.win_close();
        self.renderer = None;
        self.read_cache = None;
    }

    /// The window was resized to `width` x `height` pixels.
    pub fn resize(&mut self, width: u32, height: u32) {
        if let Some(r) = &mut self.renderer {
            r.resize(width, height);
        }
    }

    /// Runs after each presented frame, with the game image already drawn.
    pub fn set_present_hook(&mut self, hook: Option<PresentHook>) {
        self.hook = hook;
    }

    /// The renderer, once the window is open (for overlays that need the device).
    pub fn renderer(&self) -> Option<&Renderer> {
        self.renderer.as_ref()
    }

    /// Sends pending work to the GPU. Shows the front buffer if it was drawn to.
    pub fn flush(&mut self) {
        let commands = self.core.take_commands();
        if !commands.is_empty() {
            self.read_cache = None;
        }
        let gamma = self.core.gamma();
        let Some(r) = &mut self.renderer else { return };
        if r.execute(commands) || r.front_dirty() {
            r.present(gamma, self.hook.as_mut());
        }
    }

    /// `grBufferSwap`.
    pub fn buffer_swap(&mut self, interval: u32) {
        self.core.buffer_swap(interval);
        self.flush();
    }

    /// `grSstIdle`.
    pub fn sst_idle(&mut self) {
        self.flush();
    }

    /// `grLfbLock`. The returned pointer stays valid until the matching unlock.
    pub fn lfb_lock(
        &mut self,
        lock_type: u32,
        buffer: Buffer,
        write_mode: LfbWriteMode,
        origin: Origin,
        pixel_pipeline: bool,
    ) -> Option<LockInfo> {
        self.renderer.as_ref()?;
        if lock_type & lock::WRITE_ONLY != 0 {
            if !matches!(buffer, Buffer::Front | Buffer::Back | Buffer::Aux | Buffer::Depth) {
                return None;
            }
            return Some(self.core.lfb_lock_write(buffer, write_mode, origin, pixel_pipeline));
        }
        self.flush();
        let r = self.renderer.as_ref()?;
        Some(match buffer {
            Buffer::Front | Buffer::Back => {
                let rgba = cached_color(&mut self.read_cache, r, buffer);
                self.core.lfb_lock_read(buffer, origin, ReadPixels::Color(rgba))
            }
            Buffer::Aux | Buffer::Depth => {
                let depth = r.read_depth();
                self.core.lfb_lock_read(buffer, origin, ReadPixels::Depth(&depth))
            }
            _ => return None,
        })
    }

    /// `grLfbUnlock`.
    pub fn lfb_unlock(&mut self, lock_type: u32, _buffer: Buffer) -> bool {
        self.core.lfb_unlock(lock_type & lock::WRITE_ONLY != 0)
    }

    /// `grLfbReadRegion`: 565 pixels into `dst` with `stride` bytes per row.
    #[allow(clippy::too_many_arguments)]
    pub fn lfb_read_region(
        &mut self,
        buffer: Buffer,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        stride: usize,
        dst: &mut [u8],
    ) -> bool {
        if !matches!(buffer, Buffer::Front | Buffer::Back) {
            return false;
        }
        self.flush();
        let Some(r) = &self.renderer else { return false };
        let rgba = cached_color(&mut self.read_cache, r, buffer);
        self.core.lfb_read_region(rgba, x, y, width, height, stride, dst);
        true
    }

    /// The color buffer as RGBA8 rows, top first (screenshots and tests).
    pub fn read_pixels(&mut self, buffer: Buffer) -> Vec<u8> {
        self.flush();
        self.renderer.as_ref().map(|r| r.read_color(buffer)).unwrap_or_default()
    }
}

/// `buffer`'s pixels, read back from the GPU only when `cache` holds another buffer.
fn cached_color<'a>(cache: &'a mut Option<(Buffer, Vec<u8>)>, r: &Renderer, buffer: Buffer) -> &'a [u8] {
    if cache.as_ref().is_none_or(|(b, _)| *b != buffer) {
        *cache = Some((buffer, r.read_color(buffer)));
    }
    &cache.as_ref().expect("filled above").1
}
