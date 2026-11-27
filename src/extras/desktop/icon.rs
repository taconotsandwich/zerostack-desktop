//! Heroicons (outline 24 with a 1.5 stroke, plus solid `stop`) and the
//! zerostack mark, embedded at build time.

use iced::widget::{Svg, svg};
use iced::{Color, Radians, Rotation};

macro_rules! icons {
    ($($variant:ident => $path:literal,)*) => {
        /// An icon the desktop draws. Each maps to one embedded SVG.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Icon {
            $($variant,)*
        }

        impl Icon {
            /// Every icon, for asset checks.
            #[cfg(test)]
            pub const ALL: &[Icon] = &[$(Icon::$variant,)*];

            pub fn bytes(self) -> &'static [u8] {
                match self {
                    $(Icon::$variant => include_bytes!(concat!("../../../assets/desktop/icons/", $path)),)*
                }
            }
        }
    };
}

icons! {
    ArrowPath => "outline/arrow-path.svg",
    ArrowUp => "outline/arrow-up.svg",
    Check => "outline/check.svg",
    CheckCircle => "outline/check-circle.svg",
    ChevronDoubleLeft => "outline/chevron-double-left.svg",
    ChevronDown => "outline/chevron-down.svg",
    ChevronLeft => "outline/chevron-left.svg",
    ChevronRight => "outline/chevron-right.svg",
    Clock => "outline/clock.svg",
    Cog6Tooth => "outline/cog-6-tooth.svg",
    CommandLine => "outline/command-line.svg",
    DocumentDuplicate => "outline/document-duplicate.svg",
    EllipsisHorizontal => "outline/ellipsis-horizontal.svg",
    ExclamationCircle => "outline/exclamation-circle.svg",
    Folder => "outline/folder.svg",
    MagnifyingGlass => "outline/magnifying-glass.svg",
    PaperClip => "outline/paper-clip.svg",
    Pencil => "outline/pencil.svg",
    PencilSquare => "outline/pencil-square.svg",
    Plus => "outline/plus.svg",
    ShieldCheck => "outline/shield-check.svg",
    ShieldExclamation => "outline/shield-exclamation.svg",
    Stop => "solid/stop.svg",
    XMark => "outline/x-mark.svg",
}

/// `icon` at `size` logical pixels, tinted `color`.
pub fn icon<'a>(icon: Icon, size: f32, color: Color) -> Svg<'a> {
    svg(svg::Handle::from_memory(icon.bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
}

/// The running spinner: `arrow-path` turned by `angle`.
pub fn spinner<'a>(size: f32, color: Color, angle: Radians) -> Svg<'a> {
    icon(Icon::ArrowPath, size, color).rotation(Rotation::Floating(angle))
}

/// SVG bytes of the zerostack mark (keeps its own colors).
pub const LOGO: &[u8] = include_bytes!("../../../assets/desktop/icons/logo.svg");

/// The zerostack mark at `size` logical pixels.
pub fn logo<'a>(size: f32) -> Svg<'a> {
    svg(svg::Handle::from_memory(LOGO)).width(size).height(size)
}
