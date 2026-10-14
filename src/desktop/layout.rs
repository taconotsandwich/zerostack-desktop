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
pub(super) const CONTENT_WIDTH: f32 = 800.0;
pub(super) const PANEL_WIDTH: f32 = 520.0;
pub(super) const MENU_WIDTH: f32 = 208.0;
pub(super) const USAGE_WIDTH: f32 = 240.0;
pub(super) const EDITOR_HEIGHT: f32 = 56.0;
pub(super) const MESSAGE_WIDTH: f32 = 560.0;
pub(super) const MODEL_ROW_HEIGHT: f32 = 60.0;
pub(super) const COMMAND_COLUMN: f32 = 144.0;
pub(super) const PICKER_HEIGHT: f32 = 180.0;
pub(super) const REVIEW_WIDTH: f32 = 440.0;
pub(super) const REVIEW_SPLIT_WIDTH: f32 = 1200.0;

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub content_width: f32,
    pub gutter: f32,
    pub panel_height: f32,
    size: Size,
}

impl Layout {
    /// Leave space for the model and icon controls even with the sidebar open.
    pub fn prompt_width(self) -> f32 {
        ((self.content_width - 2.0 * self.gutter) * 0.2).clamp(80.0, 120.0)
    }

    pub fn model_width(self, busy: bool) -> f32 {
        let icons = if busy { 4.0 } else { 3.0 };
        (self.content_width
            - 2.0 * self.gutter
            - 2.0 * LG
            - icons * ICON_TARGET
            - (icons + 1.0) * XS
            - self.prompt_width())
        .max(0.0)
    }

    pub fn modal_body_height(self, footer_height: f32) -> f32 {
        (self.panel_height - CONTROL_HEIGHT - 2.0 * LG - footer_height).max(0.0)
    }

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

    #[test]
    fn composer_keeps_room_for_model_text_at_minimum_window_size() {
        for sidebar in [false, true] {
            for width in [680.0, 1040.0, 1440.0] {
                let layout = Layout::new(Size::new(width, 480.0), sidebar);
                let model_width = layout.model_width(true);
                // Includes Stop while a turn runs, in addition to Send.
                assert!(model_width >= 80.0, "model gets {model_width}px at {width}");
            }
        }
    }

    #[test]
    fn model_actions_remain_visible_when_favorite_fields_are_open() {
        let layout = Layout::new(Size::new(680.0, 480.0), true);
        for footer in [CONTROL_HEIGHT, 2.0 * CONTROL_HEIGHT + SM] {
            let body = layout.modal_body_height(footer);
            assert!(body >= 200.0);
            assert!(body + CONTROL_HEIGHT + 2.0 * LG + footer + 2.0 * XL <= 480.0);
        }
    }
}
