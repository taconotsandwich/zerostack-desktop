//! Design tokens from the desktop canvas: palette, metrics and fonts.
//!
//! Every color, size and font the desktop draws with comes from here, so the
//! window stays in step with the canvas boards.

use iced::font::{Family, Weight};
use iced::theme::Palette;
use iced::{Font, Theme};

/// Canvas palette.
pub mod color {
    use iced::Color;

    /// Sidebar and deepest surfaces.
    pub const BG0: Color = Color::from_rgb8(0x14, 0x15, 0x12);
    /// Window background.
    pub const BG1: Color = Color::from_rgb8(0x1C, 0x1E, 0x1A);
    /// Raised surfaces: bubbles, cards, composer.
    pub const BG2: Color = Color::from_rgb8(0x26, 0x28, 0x1F);
    /// Inset wells inside cards (commands, code).
    pub const WELL: Color = Color::from_rgb8(0x1E, 0x20, 0x1B);
    /// Hairline dividers.
    pub const LINE: Color = Color::from_rgb8(0x2C, 0x2E, 0x27);
    /// Borders of raised surfaces.
    pub const EDGE: Color = Color::from_rgb8(0x3A, 0x3D, 0x34);

    /// Headings and primary text.
    pub const INK: Color = Color::from_rgb8(0xED, 0xEA, 0xDE);
    /// Assistant prose.
    pub const BODY: Color = Color::from_rgb8(0xC9, 0xC5, 0xB8);
    /// Secondary labels and icons.
    pub const MUTED: Color = Color::from_rgb8(0xA3, 0xA0, 0x97);
    /// Timestamps and hints.
    pub const FAINT: Color = Color::from_rgb8(0x7B, 0x78, 0x6F);

    /// Selection, focus and the running spinner.
    pub const ACCENT: Color = Color::from_rgb8(0x9F, 0xB2, 0xFF);
    /// Text on accent backgrounds.
    pub const ACCENT_INK: Color = Color::from_rgb8(0xC6, 0xD2, 0xFF);
    /// Accent wash behind the selected option.
    pub const ACCENT_BG: Color = Color::from_rgba8(0x9F, 0xB2, 0xFF, 0.14);

    /// Inline code and commands.
    pub const GOLD: Color = Color::from_rgb8(0xD8, 0xC6, 0x90);
    /// Waiting on the user.
    pub const AMBER: Color = Color::from_rgb8(0xF0, 0xA3, 0x5E);
    /// Errors, rejections and removed lines.
    pub const DANGER: Color = Color::from_rgb8(0xF2, 0x94, 0x8B);
    /// Success and added lines.
    pub const ADD: Color = Color::from_rgb8(0xA6, 0xD0, 0x7A);
    /// Wash behind added diff lines.
    pub const ADD_BG: Color = Color::from_rgba8(0x7F, 0xB2, 0x4A, 0.14);
    /// Wash behind removed diff lines.
    pub const DEL_BG: Color = Color::from_rgba8(0xF2, 0x94, 0x8B, 0.12);

    /// Row hover wash.
    pub const HOVER: Color = Color::from_rgba8(0xED, 0xEA, 0xDE, 0.05);
    /// Selected row wash.
    pub const SELECTED: Color = Color::from_rgba8(0xED, 0xEA, 0xDE, 0.09);
    /// The zerostack mark.
    pub const LOGO_GREEN: Color = Color::from_rgb8(0x4A, 0x7A, 0x23);

    /// Drop shadow under floating cards.
    pub const SHADOW: Color = Color::from_rgba8(0, 0, 0, 0.35);
}

/// Canvas layout metrics, in logical pixels.
pub mod metric {
    /// Sidebar width.
    pub const SIDEBAR_WIDTH: f32 = 260.0;
    /// Main header height.
    pub const HEADER_HEIGHT: f32 = 48.0;
    /// Width of the transcript and composer column.
    pub const COLUMN_WIDTH: f32 = 720.0;

    /// Sidebar and settings list rows.
    pub const ROW_HEIGHT: f32 = 32.0;
    /// Transcript activity rows ("Read 4 files").
    pub const ACTIVITY_HEIGHT: f32 = 22.0;
    /// Permission card options.
    pub const OPTION_HEIGHT: f32 = 38.0;

    pub const RADIUS_XS: f32 = 6.0;
    pub const RADIUS_SM: f32 = 8.0;
    pub const RADIUS_MD: f32 = 10.0;
    pub const RADIUS_LG: f32 = 12.0;
    pub const RADIUS_XL: f32 = 16.0;
    /// Composer pill.
    pub const RADIUS_PILL: f32 = 20.0;
}

/// Canvas type scale: (font size, line height), in logical pixels.
pub mod text {
    /// Message prose and the composer.
    pub const BODY: (f32, f32) = (15.0, 24.0);
    /// Headings, list rows and options.
    pub const UI: (f32, f32) = (14.0, 20.0);
    /// Activity rows and header meta.
    pub const SMALL: (f32, f32) = (13.0, 18.0);
    /// Hints and timestamps.
    pub const CAPTION: (f32, f32) = (12.0, 16.0);
    /// Inline code.
    pub const CODE: (f32, f32) = (13.0, 20.0);
}

const INTER: &str = "Inter";
const JETBRAINS_MONO: &str = "JetBrains Mono";

/// Inter at weight 400.
pub const SANS: Font = Font {
    family: Family::Name(INTER),
    ..Font::DEFAULT
};
/// Inter at weight 500.
pub const SANS_MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..SANS
};
/// Inter at weight 600.
pub const SANS_SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..SANS
};
/// JetBrains Mono at weight 400.
pub const MONO: Font = Font {
    family: Family::Name(JETBRAINS_MONO),
    ..Font::DEFAULT
};
/// JetBrains Mono at weight 500.
pub const MONO_MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..MONO
};
/// JetBrains Mono at weight 700.
pub const MONO_BOLD: Font = Font {
    weight: Weight::Bold,
    ..MONO
};

/// Font files loaded at launch, one per weight in use.
pub const FONT_FILES: [&[u8]; 6] = [
    include_bytes!("../../../assets/desktop/fonts/Inter-Regular.otf"),
    include_bytes!("../../../assets/desktop/fonts/Inter-Medium.otf"),
    include_bytes!("../../../assets/desktop/fonts/Inter-SemiBold.otf"),
    include_bytes!("../../../assets/desktop/fonts/JetBrainsMono-Regular.otf"),
    include_bytes!("../../../assets/desktop/fonts/JetBrainsMono-Medium.otf"),
    include_bytes!("../../../assets/desktop/fonts/JetBrainsMono-Bold.otf"),
];

/// The iced theme built from the canvas palette. Widgets that need more than
/// the palette style themselves in `style.rs`.
pub fn theme() -> Theme {
    Theme::custom(
        "zerostack",
        Palette {
            background: color::BG1,
            text: color::INK,
            primary: color::ACCENT,
            success: color::ADD,
            warning: color::AMBER,
            danger: color::DANGER,
        },
    )
}
