//! A window of our own for Linux and BSD games that pass no usable window.
//!
//! Glide's single `hWnd` cannot describe a Wayland surface or an X11 window,
//! and Glide games for Linux never passed one. winit picks Wayland or X11, and
//! its event loop runs on a thread of its own because the game owns the main
//! thread. The window lives until the process exits.

use raw_window_handle::{RawDisplayHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::wayland::EventLoopBuilderExtWayland;
use winit::platform::x11::EventLoopBuilderExtX11;
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{Window, WindowId};

/// The window's handles and its size in pixels.
pub struct Opened {
    pub window: RawWindowHandle,
    pub display: RawDisplayHandle,
    pub size: (u32, u32),
}

// SAFETY: the handles identify a window and display connection that the event
// thread keeps alive for the rest of the process; wgpu may use them from any
// thread (winit initialises Xlib for threads, and Wayland displays are thread-safe).
unsafe impl Send for Opened {}

struct Shared {
    size: AtomicU64,
    resized: AtomicBool,
}

struct State {
    window: RawWindowHandle,
    display: RawDisplayHandle,
    shared: Arc<Shared>,
}

// SAFETY: as for `Opened`.
unsafe impl Send for State {}
// SAFETY: as for `Opened`; the handles are only read after creation.
unsafe impl Sync for State {}

static STATE: OnceLock<Option<State>> = OnceLock::new();

const fn pack(w: u32, h: u32) -> u64 {
    (w as u64) << 32 | h as u64
}

const fn unpack(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, v as u32)
}

/// Opens the window on first use, `size` logical pixels large, and returns it;
/// `None` when there is no display to open one on.
// shortcut: later opens reuse the first window without resizing it; follow a
// game's resolution changes when one needs it.
pub fn open(size: (u32, u32)) -> Option<Opened> {
    let state = STATE.get_or_init(|| match spawn(size) {
        Ok(state) => Some(state),
        Err(e) => {
            log::warn!("no window opened ({e}); rendering off screen");
            None
        }
    });
    state.as_ref().map(|s| Opened {
        window: s.window,
        display: s.display,
        size: unpack(s.shared.size.load(Ordering::Relaxed)),
    })
}

/// The window's new size if it changed since the last call.
pub fn take_resize() -> Option<(u32, u32)> {
    let shared = &STATE.get()?.as_ref()?.shared;
    shared.resized.swap(false, Ordering::AcqRel).then(|| unpack(shared.size.load(Ordering::Acquire)))
}

fn spawn(size: (u32, u32)) -> Result<State, String> {
    let shared = Arc::new(Shared { size: AtomicU64::new(pack(size.0, size.1)), resized: AtomicBool::new(false) });
    let (tx, rx) = mpsc::channel();
    let thread_shared = shared.clone();
    std::thread::Builder::new()
        .name("rustyglide-window".into())
        .spawn(move || {
            let mut builder = EventLoop::builder();
            EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
            EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
            let event_loop = match builder.build() {
                Ok(event_loop) => event_loop,
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    return;
                }
            };
            let mut app = App { size, tx: Some(tx), window: None, shared: thread_shared };
            if let Err(e) = event_loop.run_app(&mut app) {
                log::error!("window event loop: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    let opened: Opened = rx.recv().map_err(|_| "window thread ended".to_string())??;
    Ok(State { window: opened.window, display: opened.display, shared })
}

struct App {
    size: (u32, u32),
    tx: Option<mpsc::Sender<Result<Opened, String>>>,
    window: Option<Window>,
    shared: Arc<Shared>,
}

impl App {
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<Opened, String> {
        let attributes = Window::default_attributes()
            .with_title("rustyglide")
            .with_inner_size(LogicalSize::new(self.size.0, self.size.1));
        let window = event_loop.create_window(attributes).map_err(|e| e.to_string())?;
        let handle = window.window_handle().map_err(|e| e.to_string())?.as_raw();
        let display = window.display_handle().map_err(|e| e.to_string())?.as_raw();
        let PhysicalSize { width, height } = window.inner_size();
        self.shared.size.store(pack(width.max(1), height.max(1)), Ordering::Release);
        log::info!("opened a {width}x{height} window ({handle:?})");
        self.window = Some(window);
        Ok(Opened { window: handle, display, size: (width.max(1), height.max(1)) })
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Some(tx) = self.tx.take()
        {
            let _ = tx.send(self.create(event_loop));
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            // Games read input elsewhere and have no close hook; closing the window quits, as it would a fullscreen game.
            WindowEvent::CloseRequested => std::process::exit(0),
            WindowEvent::Resized(PhysicalSize { width, height }) if width > 0 && height > 0 => {
                self.shared.size.store(pack(width, height), Ordering::Release);
                self.shared.resized.store(true, Ordering::Release);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn packs_sizes() {
        assert_eq!(super::unpack(super::pack(640, 480)), (640, 480));
        assert_eq!(super::unpack(super::pack(u32::MAX, 1)), (u32::MAX, 1));
    }
}
