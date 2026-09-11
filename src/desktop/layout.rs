use iced::{Point, Size};

pub(super) const XS: f32 = 4.0;
pub(super) const SM: f32 = 8.0;
pub(super) const MD: f32 = 12.0;
pub(super) const LG: f32 = 16.0;
pub(super) const XL: f32 = 24.0;
pub(super) const ICON_TARGET: f32 = 32.0;
pub(super) const CONTROL_HEIGHT: f32 = 36.0;
pub(super) const HEADER_HEIGHT: f32 = 56.0;
pub(super) const SIDEBAR_WIDTH: f32 = 248.0;
pub(super) const CONTENT_WIDTH: f32 = 880.0;
pub(super) const PANEL_WIDTH: f32 = 520.0;
pub(super) const MENU_WIDTH: f32 = 208.0;
pub(super) const USAGE_WIDTH: f32 = 240.0;
pub(super) const EDITOR_HEIGHT: f32 = 64.0;
pub(super) const MESSAGE_WIDTH: f32 = 620.0;
pub(super) const COMMAND_COLUMN: f32 = 144.0;
pub(super) const PICKER_HEIGHT: f32 = 180.0;

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub content_width: f32,
    pub gutter: f32,
    pub panel_height: f32,
    size: Size,
}

impl Layout {
    pub fn new(size: Size, sidebar: bool) -> Self {
        let main_width = (size.width - if sidebar { SIDEBAR_WIDTH } else { 0.0 }).max(0.0);
        Self {
            content_width: main_width.min(CONTENT_WIDTH),
            gutter: if main_width < 600.0 { LG } else { XL },
            panel_height: (size.height - 4.0 * XL).clamp(0.0, 560.0),
            size,
        }
    }

    pub fn menu_position(self, cursor: Point, rows: usize) -> Point {
        let height = rows as f32 * CONTROL_HEIGHT + 2.0 * XS;
        Point::new(
            (MD).min((self.size.width - MENU_WIDTH).max(0.0)),
            cursor
                .y
                .clamp(0.0, (self.size.height - height - SM).max(0.0)),
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
        assert!(position.y + 4.0 * CONTROL_HEIGHT + 2.0 * XS <= 480.0);
        assert!(position.x + MENU_WIDTH <= 680.0);
        assert!(layout.panel_height + 4.0 * XL <= 480.0);
    }
}
