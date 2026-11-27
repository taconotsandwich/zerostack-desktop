//! Home: the first prompt of a new session, centered under the mark.

use iced::widget::{Space, column, container, row, space, stack};
use iced::{Alignment, Element, Length, Padding};

use super::{composer, main_bar};
use crate::extras::desktop::app::{App, Message, tilde};
use crate::extras::desktop::icon::{self, Icon};
use crate::extras::desktop::theme::{self, color, text};
use crate::extras::desktop::widgets::{chip, hint, label, mono};

/// Column width on Home.
const WIDTH: f32 = 680.0;
/// Height kept under the composer: the chips, and the menu that opens over
/// them. Fixed, so opening a menu never moves the composer.
const BELOW: f32 = 360.0;
/// Space above the block, so it sits where the canvas centers it with a
/// 140px bottom inset and a 40px chips row.
const TOP: f32 = BELOW - 180.0;

pub(super) fn view(app: &App) -> Element<'_, Message> {
    let heading = row![
        icon::logo(36.0),
        label("What are we building?", (28.0, 36.0), color::INK).font(theme::SANS_MEDIUM),
    ]
    .spacing(14)
    .align_y(Alignment::Center);

    let home_dir = dirs::home_dir();
    let mut chips = row![chip(
        Some(Icon::Folder),
        label(
            tilde(&app.folder, home_dir.as_deref()),
            text::SMALL,
            color::INK
        ),
        Some(Message::OpenFolder),
    )]
    .spacing(6)
    .align_y(Alignment::Center);
    // zerostack reads the branch but never switches it, so the branch is
    // shown as text, not as a chip that looks like it opens something.
    if let Some(branch) = &app.info.branch {
        chips = chips
            .push(container(mono(branch.as_str(), text::CAPTION, color::MUTED)).padding([0, 6]));
    }
    let chips = chips
        .push(space::horizontal())
        .push(hint("⇧Tab mode · / commands · ! shell"));
    let chips = container(chips).padding(Padding::from([0, 4]).top(10));

    let mut below = stack![container(chips).height(BELOW).width(Length::Fill)];
    if let Some(menu) = composer::menu(app) {
        below = below.push(column![Space::new().height(8.0), menu]);
    }

    let block = column![heading, column![composer::view(app, true), below]]
        .spacing(28)
        .align_x(Alignment::Center)
        .max_width(WIDTH);

    column![
        main_bar(app, Space::new()),
        container(block)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(Padding::from([0, 24]).top(TOP))
            .center_x(Length::Fill)
            .align_y(Alignment::Center),
    ]
    .into()
}
