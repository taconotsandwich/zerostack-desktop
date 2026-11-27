//! Building blocks the views share: text, buttons, rows, chips, the session
//! status slot and the context ring.
//!
//! Every builder is stateless and takes its messages from the caller, so the
//! views stay plain functions of app state. Sizes follow the canvas boards.

use std::f32::consts::{FRAC_PI_2, TAU};

use iced::widget::canvas::{self, LineCap, Path, Stroke, path::Arc};
use iced::widget::text::{IntoFragment, LineHeight, Wrapping};
use iced::widget::{Button, Text, button, canvas as draw, container, row, text};
use iced::{Alignment, Color, Element, Font, Length, Padding, Radians, Rectangle, Renderer, Theme};
use iced::{Point, mouse};

use super::icon::{self, Icon};
use super::style;
use super::theme::{self, color, metric};

// ── Text ───────────────────────────────────────────────────────────────

/// Inter at a canvas size: `(font size, line height)` from `theme::text`.
pub fn label<'a>(content: impl IntoFragment<'a>, size: (f32, f32), color: Color) -> Text<'a> {
    text(content)
        .size(size.0)
        .line_height(LineHeight::Absolute(size.1.into()))
        .color(color)
}

/// JetBrains Mono at a canvas size.
pub fn mono<'a>(content: impl IntoFragment<'a>, size: (f32, f32), color: Color) -> Text<'a> {
    label(content, size, color).font(theme::MONO)
}

/// Inline path or command in an activity row: gold mono at caption size.
pub fn code<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    mono(content, theme::text::CAPTION, color::GOLD)
}

/// Keyboard hint or meta line: muted caption ("⌘N", "Enter", "2h").
pub fn hint<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    label(content, theme::text::CAPTION, color::MUTED)
}

/// One line that clips instead of wrapping (titles, paths).
fn single_line(text: Text<'_>) -> Text<'_> {
    text.wrapping(Wrapping::None).width(Length::Fill)
}

// ── Buttons ────────────────────────────────────────────────────────────

/// Square muted icon button: `box_size` square, `glyph` at `glyph_size`.
/// Sidebar toggle 28/16, folder plus 22/14, settings 32/18, header menu 28/18.
/// Swap the style for a bordered one with `.style(style::outline)`.
pub fn icon_button<'a, M: Clone + 'a>(
    glyph: Icon,
    glyph_size: f32,
    box_size: f32,
    on_press: M,
) -> Button<'a, M> {
    button(container(icon::icon(glyph, glyph_size, color::MUTED)).center(box_size))
        .width(box_size)
        .height(box_size)
        .padding(0)
        .style(style::ghost)
        .on_press(on_press)
}

/// Round composer action: accent Send (`ArrowUp`) or ink Stop (`Stop`).
/// `None` disables it.
pub fn round_button<'a, M: Clone + 'a>(
    glyph: Icon,
    fill: Color,
    on_press: Option<M>,
) -> Button<'a, M> {
    button(container(icon::icon(glyph, 16.0, color::BG0)).center(32.0))
        .width(32.0)
        .height(32.0)
        .padding(0)
        .style(style::round(fill))
        .on_press_maybe(on_press)
}

/// Borderless composer menu button with a trailing chevron: permission mode
/// ("Standard") and model.
pub fn dropdown<'a, M: Clone + 'a>(
    leading: Option<Icon>,
    title: impl IntoFragment<'a>,
    on_press: Option<M>,
) -> Button<'a, M> {
    let mut content = row![].spacing(6).align_y(Alignment::Center);
    if let Some(glyph) = leading {
        content = content.push(icon::icon(glyph, 16.0, color::MUTED));
    }
    let content = content
        .push(label(title, theme::text::SMALL, color::MUTED))
        .push(icon::icon(Icon::ChevronDown, 12.0, color::MUTED));
    button(content.height(Length::Fill))
        .height(32.0)
        .padding(Padding::ZERO.left(10).right(8))
        .style(style::ghost)
        .on_press_maybe(on_press)
}

/// Bordered chip under the home composer (the folder). Ends with a chevron.
pub fn chip<'a, M: Clone + 'a>(
    leading: Option<Icon>,
    title: Text<'a>,
    on_press: Option<M>,
) -> Button<'a, M> {
    let mut content = row![].spacing(7).align_y(Alignment::Center);
    if let Some(glyph) = leading {
        content = content.push(icon::icon(glyph, 14.0, color::MUTED));
    }
    let content = content
        .push(title)
        .push(icon::icon(Icon::ChevronDown, 12.0, color::MUTED));
    button(content.height(Length::Fill))
        .height(30.0)
        .padding([0, 10])
        .style(style::outline)
        .on_press_maybe(on_press)
}

// ── Rows ───────────────────────────────────────────────────────────────

/// Sidebar action row: "New session ⌘N", "Search ⌘K". `font` sets the title
/// weight (the primary action is medium).
pub fn nav_row<'a, M: Clone + 'a>(
    glyph: Icon,
    title: &'a str,
    shortcut: &'a str,
    font: Font,
    on_press: M,
) -> Button<'a, M> {
    let content = row![
        icon::icon(glyph, 16.0, color::MUTED),
        label(title, theme::text::UI, color::INK)
            .font(font)
            .width(Length::Fill),
        hint(shortcut),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .height(Length::Fill);
    button(content)
        .width(Length::Fill)
        .height(metric::ROW_HEIGHT)
        .padding([0, 8])
        .style(style::row(false))
        .on_press(on_press)
}

/// Folder heading over its sessions, with a "new session here" button.
pub fn group_header<'a, M: Clone + 'a>(name: &'a str, on_new: M) -> Element<'a, M> {
    row![
        icon::icon(Icon::Folder, 14.0, color::MUTED),
        single_line(label(name, theme::text::CAPTION, color::MUTED).font(theme::SANS_MEDIUM)),
        icon_button(Icon::Plus, 14.0, 22.0, on_new),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .height(28.0)
    .padding([0, 8])
    .into()
}

/// What a saved session is doing, shown at the end of its sidebar row.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// A turn is streaming; the spinner turns by the given angle.
    Running(Radians),
    /// Waiting on a permission answer.
    NeedsYou,
    /// Idle; shows when it last changed ("now", "2h", "Mon").
    Idle(String),
}

/// Session row under a folder heading. `selected` marks the open session.
pub fn session_row<'a, M: Clone + 'a>(
    title: &'a str,
    status: &Status,
    selected: bool,
    on_press: M,
) -> Button<'a, M> {
    let (ink, font) = if selected {
        (color::INK, theme::SANS_MEDIUM)
    } else {
        (color::BODY, theme::SANS)
    };
    let content = row![
        single_line(label(title, theme::text::UI, ink).font(font)),
        status_slot(status),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .height(Length::Fill);
    button(content)
        .width(Length::Fill)
        .height(metric::ROW_HEIGHT)
        .padding(Padding::ZERO.left(28).right(8))
        .style(style::row(selected))
        .on_press(on_press)
}

fn status_slot<'a, M: 'a>(status: &Status) -> Element<'a, M> {
    match status {
        Status::Running(angle) => icon::spinner(16.0, color::ACCENT, *angle).into(),
        Status::NeedsYou => icon::icon(Icon::ExclamationCircle, 16.0, color::AMBER).into(),
        Status::Idle(when) => hint(when.clone()).into(),
    }
}

/// How an activity row opens: chevron right or down, or the spinner while
/// its tool runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lead {
    Collapsed,
    Expanded,
    Running(Radians),
}

/// Transcript activity row ("Thought for 6s", "Edited src/x.rs +2 −1"):
/// a lead glyph then `parts` in muted small text. Clickable when `on_press`
/// toggles its details.
pub fn activity<'a, M: Clone + 'a>(
    lead: Lead,
    parts: impl IntoIterator<Item = Element<'a, M>>,
    on_press: Option<M>,
) -> Element<'a, M> {
    let glyph: Element<'a, M> = match lead {
        Lead::Collapsed => icon::icon(Icon::ChevronRight, 14.0, color::MUTED).into(),
        Lead::Expanded => icon::icon(Icon::ChevronDown, 14.0, color::MUTED).into(),
        Lead::Running(angle) => icon::spinner(14.0, color::ACCENT, angle).into(),
    };
    let content = row![glyph]
        .extend(parts)
        .spacing(6)
        .align_y(Alignment::Center)
        .height(metric::ACTIVITY_HEIGHT);
    match on_press {
        Some(message) => button(content)
            .padding(0)
            .style(style::bare)
            .on_press(message)
            .into(),
        None => content.into(),
    }
}

/// Muted small text for activity row parts.
pub fn activity_text<'a>(content: impl IntoFragment<'a>) -> Text<'a> {
    label(content, theme::text::SMALL, color::MUTED)
}

// ── Indicators ─────────────────────────────────────────────────────────

/// The ring's SVG geometry on the canvas: an 18-unit box drawn at 16px.
const RING_BOX: f32 = 18.0;
const RING_RADIUS: f32 = 7.0;
const RING_STROKE: f32 = 2.5;
const RING_SIZE: f32 = 16.0;

/// Context window ring: an edge track with an accent arc for the share of
/// the window in use, clockwise from 12 o'clock. `used` is clamped to 0..=1.
pub fn context_ring<'a, M: 'a>(used: f32) -> Element<'a, M> {
    draw(Ring {
        used: used.clamp(0.0, 1.0),
    })
    .width(RING_SIZE)
    .height(RING_SIZE)
    .into()
}

struct Ring {
    used: f32,
}

impl<M> canvas::Program<M> for Ring {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let scale = bounds.width.min(bounds.height) / RING_BOX;
        let center: Point = frame.center();
        let radius = RING_RADIUS * scale;
        let stroke = Stroke::default().with_width(RING_STROKE * scale);

        frame.stroke(
            &Path::circle(center, radius),
            stroke.with_color(color::EDGE),
        );
        if self.used > 0.0 {
            let start = -FRAC_PI_2;
            let arc = Path::new(|path| {
                path.arc(Arc {
                    center,
                    radius,
                    start_angle: Radians(start),
                    end_angle: Radians(start + TAU * self.used),
                });
            });
            frame.stroke(
                &arc,
                stroke
                    .with_color(color::ACCENT)
                    .with_line_cap(LineCap::Round),
            );
        }
        vec![frame.into_geometry()]
    }
}
