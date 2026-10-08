//! What a graphical frontend reads from the engine besides the
//! conversation: bundled docs, the active colors and the memory file.

use super::Engine;

impl Engine {
    /// Read a bundled documentation file (`GET_STARTED.md`, `COMMANDS.md`, …)
    /// so a GUI shell can render it. Mirrors the TUI's `/docs` and `/tutor`.
    pub fn read_doc(&self, name: &str) -> anyhow::Result<String> {
        let name = name.trim();
        anyhow::ensure!(!name.is_empty(), "usage: /docs <file>");
        anyhow::ensure!(
            !name.contains('/') && !name.contains('\\') && !name.starts_with('.'),
            "invalid doc name: {name}"
        );
        crate::docs::read(name)
    }

    /// The colors a UI should render with: the active theme file when one is
    /// selected, otherwise the config `[colors]` section. Mirrors the TUI's
    /// theme resolution in `run_interactive`.
    pub fn active_colors(&self) -> Option<crate::config::ColorsConfig> {
        if let Some(name) = self.context.current_theme_name.as_deref()
            && let Some(content) = self.context.themes.get(name)
            && let Ok(colors) = serde_json::from_str::<crate::config::ColorsConfig>(content)
        {
            return Some(colors);
        }
        self.cfg.colors.clone()
    }

    /// Absolute path of `MEMORY.md`, the file `/memory editor` opens.
    #[cfg(feature = "memory")]
    pub fn memory_editor_path(&self) -> std::path::PathBuf {
        use crate::extras::memory::Mem;
        Mem::open().memory_md()
    }
}
