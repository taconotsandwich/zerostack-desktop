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

use std::sync::OnceLock;

use anyhow::anyhow;

use crate::startup::Startup;

/// Logical window size at launch, matching the design canvas boards.
const WINDOW_SIZE: (f32, f32) = (1280.0, 800.0);

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

    let boot = std::cell::RefCell::new(Some(app::App::new(startup)));
    let window = iced::application(
        move || boot.borrow_mut().take().expect("iced boots the app once"),
        app::App::update,
        app::App::view,
    )
    .title(app::App::title)
    .executor::<SharedRuntime>()
    .window_size(WINDOW_SIZE);
    tokio::task::block_in_place(|| window.run()).map_err(|e| anyhow!("desktop window failed: {e}"))
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
