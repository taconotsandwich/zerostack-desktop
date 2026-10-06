use iced::widget::{column, container, row, rule, text};
use iced::{Element, Fill};

use super::app::Message;
use super::layout;
use super::reply::{self, Part};
use super::style;
use crate::ui::feed::BlockStyle;

/// A message the user sent: the passages it replied to as quote blocks above
/// its own words.
pub(super) fn user(content: &str) -> Element<'_, Message> {
    let color = style::role_color(BlockStyle::User);
    let parts = reply::parts(content).into_iter().map(|part| match part {
        Part::Quote(quote) => row![
            rule::vertical(2).style(|theme| rule::Style {
                color: style::LIFTED,
                ..rule::default(theme)
            }),
            text(quote).size(style::LABEL).color(style::MUTED),
        ]
        .spacing(layout::SM)
        .into(),
        Part::Text(body) => text(body).size(style::BODY).color(color).into(),
    });
    container(
        container(column(parts).spacing(layout::MD))
            .padding([layout::MD, layout::LG])
            .max_width(layout::MESSAGE_WIDTH)
            .style(|_| style::surface(style::RAISED, style::BUBBLE_RADIUS)),
    )
    .width(Fill)
    .align_x(iced::Right)
    .into()
}
