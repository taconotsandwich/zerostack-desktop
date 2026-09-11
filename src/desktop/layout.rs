use iced::{Point, Size};

pub(super) const XS: u16 = 4;
pub(super) const SM: u16 = 8;
pub(super) const MD: u16 = 12;
pub(super) const LG: u16 = 16;
pub(super) const XL: u16 = 24;
pub(super) const ICON_TARGET: u16 = 32;
pub(super) const CONTROL_HEIGHT: u16 = 36;
pub(super) const HEADER_HEIGHT: u16 = 56;
pub(super) const SIDEBAR_WIDTH: u16 = 248;
pub(super) const CONTENT_WIDTH: f32 = 880.0;
pub(super) const PANEL_WIDTH: f32 = 520.0;
pub(super) const MENU_WIDTH: f32 = 208.0;
pub(super) const USAGE_WIDTH: u16 = 240;
pub(super) const EDITOR_HEIGHT: u16 = 64;
pub(super) const MESSAGE_WIDTH: u16 = 620;
pub(super) const COMMAND_COLUMN: u16 = 144;
pub(super) const PICKER_HEIGHT: f32 = 180.0;

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub content_width: f32,
    pub gutter: u16,
    pub panel_height: f32,
    size: Size,
}

impl Layout {
    pub fn new(size: Size, sidebar: bool) -> Self {
        let main_width = (size.width - if sidebar { SIDEBAR_WIDTH as f32 } else { 0.0 }).max(0.0);
        Self {
            content_width: main_width.min(CONTENT_WIDTH),
            gutter: if main_width < 600.0 { LG } else { XL },
            panel_height: (size.height - 4.0 * XL as f32).clamp(0.0, 560.0),
            size,
        }
    }

    pub fn menu_position(self, cursor: Point, rows: usize) -> Point {
        let height = rows as f32 * CONTROL_HEIGHT as f32 + 2.0 * XS as f32;
        Point::new(
            (MD as f32).min((self.size.width - MENU_WIDTH).max(0.0)),
            cursor
                .y
                .clamp(0.0, (self.size.height - height - SM as f32).max(0.0)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_fits_with_sidebar_at_narrow_and_wide_sizes() {
        let narrow = Layout::new(Size::new(680.0, 480.0), true);
        assert_eq!(narrow.content_width, 432.0);
        assert_eq!(narrow.gutter, LG);
        let wide = Layout::new(Size::new(1440.0, 900.0), true);
        assert_eq!(wide.content_width, CONTENT_WIDTH);
        assert_eq!(wide.gutter, XL);
        assert!(Layout::new(Size::new(680.0, 480.0), false).content_width > narrow.content_width);
    }

    #[test]
    fn menus_and_panels_stay_inside_short_windows() {
        let layout = Layout::new(Size::new(680.0, 480.0), true);
        let position = layout.menu_position(Point::new(600.0, 475.0), 4);
        assert!(position.y + 4.0 * CONTROL_HEIGHT as f32 + 2.0 * XS as f32 <= 480.0);
        assert!(position.x + MENU_WIDTH <= 680.0);
        assert!(layout.panel_height + 4.0 * XL as f32 <= 480.0);
    }
}
