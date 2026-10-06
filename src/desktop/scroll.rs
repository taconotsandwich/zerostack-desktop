//! Whether the conversation sits at its end. iced reports scrolling only
//! while content overflows, so a transcript that shrinks to fit (a folded
//! tool group) never reports and would keep a stale `Latest` pill; measuring
//! after such changes keeps the follow state honest.

use iced::advanced::widget::operation::{Outcome, Scrollable};
use iced::advanced::widget::{self, Id, Operation};
use iced::{Rectangle, Task, Vector};

use super::app::Message;

pub(super) const CONVERSATION: &str = "conversation";

/// Distance from the end that still counts as following the output.
pub(super) const END_SLACK: f32 = 48.0;

/// Measure the conversation once the next layout is in place.
pub(super) fn measure() -> Task<Message> {
    widget::operate(AtEnd {
        target: Id::new(CONVERSATION),
        at_end: None,
    })
    .map(Message::Scrolled)
}

struct AtEnd {
    target: Id,
    at_end: Option<bool>,
}

impl Operation<bool> for AtEnd {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
        if self.at_end.is_none() {
            operate(self);
        }
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&self.target) {
            let hidden_below = content_bounds.height - bounds.height - translation.y;
            self.at_end = Some(hidden_below < END_SLACK);
        }
    }

    fn finish(&self) -> Outcome<bool> {
        self.at_end.map_or(Outcome::None, Outcome::Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn measure(content: f32, offset: f32) -> Outcome<bool> {
        let mut op = AtEnd {
            target: Id::new(CONVERSATION),
            at_end: None,
        };
        use widget::operation::scrollable::{AbsoluteOffset, RelativeOffset};
        struct Fixed;
        impl Scrollable for Fixed {
            fn snap_to(&mut self, _: RelativeOffset<Option<f32>>) {}
            fn scroll_to(&mut self, _: AbsoluteOffset<Option<f32>>) {}
            fn scroll_by(&mut self, _: AbsoluteOffset, _: Rectangle, _: Rectangle) {}
        }
        op.scrollable(
            Some(&Id::new(CONVERSATION)),
            Rectangle::new(iced::Point::ORIGIN, iced::Size::new(800.0, 600.0)),
            Rectangle::new(iced::Point::ORIGIN, iced::Size::new(800.0, content)),
            Vector::new(0.0, offset),
            &mut Fixed,
        );
        op.finish()
    }

    #[test]
    fn content_that_fits_or_is_scrolled_to_its_end_counts_as_the_end() {
        assert!(matches!(measure(300.0, 0.0), Outcome::Some(true)));
        assert!(matches!(measure(2000.0, 1400.0), Outcome::Some(true)));
        assert!(matches!(measure(2000.0, 1360.0), Outcome::Some(true)));
        assert!(matches!(measure(2000.0, 900.0), Outcome::Some(false)));
    }
}
