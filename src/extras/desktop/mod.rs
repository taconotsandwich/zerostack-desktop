//! Native desktop window (`zerostack --desktop`).
//!
//! One window serves one folder. The process cwd is global (tools, sandbox,
//! context and permission checks all read it), so opening another folder
//! spawns another `zerostack --desktop` process there.
//!
//! The window takes the main thread, which winit needs on macOS, from inside
//! the multi-thread runtime that ran startup. iced tasks and agent turns
//! spawn on that same runtime, so the HTTP clients and child processes built
//! during startup keep their reactor.

pub(crate) mod app;
pub(crate) mod commands;
pub(crate) mod controller;
pub(crate) mod icon;
mod select;
pub(crate) mod settings;
mod style;
pub(crate) mod theme;
pub(crate) mod transcript;
mod view;
mod widgets;

use std::sync::OnceLock;

use anyhow::anyhow;

use crate::startup::Startup;

/// Logical window size at launch, matching the design canvas boards.
const WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

/// The app icon: the canvas mark on the macOS icon grid, 512px RGBA.
pub(crate) const APP_ICON: &[u8] = include_bytes!("../../../assets/desktop/icon.png");

/// Handle to the startup runtime. iced builds its executor through
/// `Executor::new()`, which takes no arguments, so the handle is parked here.
static RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

/// Open the window for the current folder and block until it closes.
///
/// Called from `main`'s async `run`: the window loop leaves the async context
/// through `block_in_place`, which the multi-thread runtime allows on the
/// `block_on` thread.
pub fn run(startup: Startup) -> anyhow::Result<()> {
    RUNTIME
        .set(tokio::runtime::Handle::current())
        .map_err(|_| anyhow!("desktop runtime is already running"))?;

    let folder = startup.session.working_dir.to_string();
    let info = controller::session_info(&startup.session);
    let link =
        controller::Controller::from_startup(startup).spawn(tokio::runtime::Handle::current())?;

    let boot = std::cell::RefCell::new(Some(app::App::new(folder, info, link)));
    let mut window = iced::application(
        move || boot.borrow_mut().take().expect("iced boots the app once"),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .subscription(app::App::subscription)
    .theme(theme::theme())
    .default_font(theme::SANS)
    .executor::<SharedRuntime>()
    .window(window_settings());
    for font in theme::FONT_FILES {
        window = window.font(font);
    }
    tokio::task::block_in_place(|| window.run()).map_err(|e| anyhow!("desktop window failed: {e}"))
}

/// The window at the canvas size. On macOS the title bar is hidden and the
/// content runs under the traffic lights, like the canvas boards.
fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: iced::Size::new(WINDOW_SIZE.0, WINDOW_SIZE.1),
        min_size: Some(iced::Size::new(720.0, 480.0)),
        icon: decode_rgba(APP_ICON).and_then(|(rgba, width, height)| {
            iced::window::icon::from_rgba(rgba, width, height).ok()
        }),
        #[cfg(target_os = "macos")]
        platform_specific: iced::window::settings::PlatformSpecific {
            title_hidden: true,
            titlebar_transparent: true,
            fullsize_content_view: true,
        },
        ..iced::window::Settings::default()
    }
}

/// Decode an 8-bit RGBA PNG into its pixels, width and height.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut reader = png::Decoder::new(bytes).read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some((pixels, frame.width, frame.height))
}

/// Show the app icon in the Dock. winit ignores window icons on macOS and
/// a bare binary has no bundle icon, so it goes on NSApplication. Must run
/// on the main thread once the window exists; elsewhere it does nothing.
/// The one AppKit setter has no safe binding.
#[allow(unsafe_code)]
pub(crate) fn show_dock_icon() {
    #[cfg(target_os = "macos")]
    {
        use objc2::{AnyThread, MainThreadMarker};
        use objc2_app_kit::{NSApplication, NSImage};
        use objc2_foundation::NSData;

        let Some(main_thread) = MainThreadMarker::new() else {
            return;
        };
        let data = NSData::with_bytes(APP_ICON);
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
            return;
        };
        let app = NSApplication::sharedApplication(main_thread);
        // SAFETY: on the main thread, with an image that is not nil.
        unsafe { app.setApplicationIconImage(Some(&image)) };
    }
}

/// iced executor that spawns on the startup runtime instead of building its
/// own.
struct SharedRuntime(tokio::runtime::Handle);

impl iced::Executor for SharedRuntime {
    fn new() -> Result<Self, std::io::Error> {
        RUNTIME
            .get()
            .cloned()
            .map(Self)
            .ok_or_else(|| std::io::Error::other("desktop runtime is not set"))
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        drop(self.0.spawn(future));
    }

    fn block_on<T>(&self, future: impl Future<Output = T>) -> T {
        self.0.block_on(future)
    }

    fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.0.enter();
        f()
    }
}
