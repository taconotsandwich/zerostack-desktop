//! Keyboard support for shared controls. iced 0.14 buttons only handle pointers.
//! Labels are also exposed to widget operations; iced has no native accessibility
//! tree yet, so this does not claim platform screen-reader support.
use std::borrow::Cow;

use iced::advanced::widget::operation::Focusable;
use iced::advanced::widget::{Operation, Tree, operation, tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer, widget};
use iced::widget::button;
use iced::{
    Border, Color, Element, Event, Length, Padding, Rectangle, Renderer, Size, Task, Theme, Vector,
    keyboard,
};

use super::{app::Message, style};

fn anchored(message: Message, bounds: Rectangle, keyboard: bool) -> Message {
    match message {
        Message::Show(panel) => Message::ShowAt(
            panel,
            iced::Point::new(bounds.x, bounds.y + bounds.height),
            keyboard,
        ),
        message => message,
    }
}

pub(super) const DIALOG: &str = "desktop-dialog";

pub(super) fn menu_position(anchor: iced::Point, window: Size) -> iced::Point {
    iced::Point::new(
        anchor.x.clamp(8.0, (window.width - 296.0).max(8.0)),
        anchor.y.clamp(8.0, (window.height - 256.0).max(8.0)),
    )
}

pub(super) fn focus_next(backwards: bool, dialog: bool) -> Task<Message> {
    let operation: Box<dyn Operation<Message>> = if backwards {
        Box::new(operation::focusable::focus_previous())
    } else {
        Box::new(operation::focusable::focus_next())
    };
    if dialog {
        widget::operate(operation::scope(widget::Id::new(DIALOG), operation))
    } else {
        widget::operate(operation)
    }
}

/// A button builder that retains its name and keyboard action.
pub(super) struct Action<'a> {
    button: button::Button<'a, Message>,
    label: Cow<'a, str>,
    message: Option<Message>,
}
impl<'a> Action<'a> {
    pub(super) fn new(
        button: button::Button<'a, Message>,
        label: Cow<'a, str>,
        message: Option<Message>,
    ) -> Self {
        Self {
            button,
            label,
            message,
        }
    }
    pub(super) fn width(mut self, width: impl Into<Length>) -> Self {
        self.button = self.button.width(width);
        self
    }
    pub(super) fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.button = self.button.padding(padding);
        self
    }
    pub(super) fn style(
        mut self,
        style: impl Fn(&Theme, button::Status) -> button::Style + 'a,
    ) -> Self {
        self.button = self.button.style(style);
        self
    }
}
impl<'a> From<Action<'a>> for Element<'a, Message> {
    fn from(action: Action<'a>) -> Self {
        named(action.button.into(), action.label, action.message, false)
    }
}

pub(super) fn named<'a>(
    content: Element<'a, Message>,
    label: Cow<'a, str>,
    message: Option<Message>,
    hint_on_focus: bool,
) -> Element<'a, Message> {
    Element::new(Control {
        content,
        label: Some(label),
        message,
        hint_on_focus,
        inert: false,
    })
}

/// Keep the base widget tree intact while a modal temporarily blocks its input.
pub(super) fn layer(content: Element<'_, Message>, inert: bool) -> Element<'_, Message> {
    Element::new(Control {
        content,
        label: None,
        message: None,
        hint_on_focus: false,
        inert,
    })
}

struct Control<'a> {
    content: Element<'a, Message>,
    label: Option<Cow<'a, str>>,
    message: Option<Message>,
    hint_on_focus: bool,
    inert: bool,
}
#[derive(Default)]
struct State {
    focused: bool,
    label: String,
}
impl Focusable for State {
    fn is_focused(&self) -> bool {
        self.focused
    }
    fn focus(&mut self) {
        self.focused = true;
    }
    fn unfocus(&mut self) {
        self.focused = false;
    }
}
impl State {
    fn activates(&self, event: &Event, enabled: bool) -> bool {
        self.focused
            && enabled
            && matches!(event,
            Event::Keyboard(keyboard::Event::KeyPressed { key: keyboard::Key::Named(keyboard::key::Named::Enter | keyboard::key::Named::Space), modifiers, repeat: false, .. })
                if modifiers.is_empty())
    }
}
impl Widget<Message, Theme, Renderer> for Control<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }
    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State {
            label: self.label.as_deref().unwrap_or_default().into(),
            ..State::default()
        })
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }
    fn diff(&self, tree: &mut Tree) {
        let state = tree.state.downcast_mut::<State>();
        let label = self.label.as_deref().unwrap_or_default();
        if state.label != label || self.message.is_none() {
            state.focused = false;
            state.label = label.into();
        }
        tree.diff_children(&[&self.content]);
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        if self.inert {
            return;
        }
        if let Some(label) = &self.label {
            operation.text(None, layout.bounds(), label);
            if self.message.is_some() {
                operation.focusable(None, layout.bounds(), tree.state.downcast_mut::<State>());
            }
        }
        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.inert {
            return;
        }
        let state = tree.state.downcast_mut::<State>();
        if matches!(
            event,
            Event::Mouse(mouse::Event::ButtonPressed(_)) | Event::Touch(_)
        ) {
            state.unfocus();
        }
        if !shell.is_event_captured() && state.activates(event, self.message.is_some()) {
            shell.publish(anchored(
                self.message.clone().unwrap(),
                layout.bounds(),
                true,
            ));
            shell.capture_event();
            return;
        }
        // Let iced's tooltip show the same label on keyboard focus and hover.
        let cursor = if state.focused
            && self.hint_on_focus
            && matches!(
                event,
                Event::Window(iced::window::Event::RedrawRequested(_))
            ) {
            mouse::Cursor::Available(layout.bounds().center())
        } else {
            cursor
        };
        if shell.is_event_captured() {
            return;
        }
        let mut messages = Vec::new();
        let mut child_shell = Shell::new(&mut messages);
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            &mut child_shell,
            viewport,
        );
        shell.merge(child_shell, |message| {
            anchored(message, layout.bounds(), false)
        });
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        appearance: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            appearance,
            layout,
            cursor,
            viewport,
        );
        if !self.inert && tree.state.downcast_ref::<State>().focused {
            use iced::advanced::Renderer as _;
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: Border {
                        color: style::ACCENT,
                        width: 1.5,
                        radius: style::CONTROL_RADIUS.into(),
                    },
                    ..Default::default()
                },
                Color::TRANSPARENT,
            );
        }
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.inert {
            return mouse::Interaction::None;
        }
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        if self.inert {
            return None;
        }
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn press(key: keyboard::key::Named, modifiers: keyboard::Modifiers, repeat: bool) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key),
            modified_key: keyboard::Key::Named(key),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat,
        })
    }
    #[test]
    fn menu_activation_uses_its_control_bounds_for_pointer_and_keyboard() {
        let bounds = Rectangle::new(iced::Point::new(40.0, 100.0), Size::new(32.0, 32.0));
        for keyboard in [false, true] {
            assert!(
                matches!(anchored(Message::Show(super::super::app::Panel::Attachments), bounds, keyboard),
                Message::ShowAt(super::super::app::Panel::Attachments, position, by_keyboard)
                    if position == iced::Point::new(40.0, 132.0) && by_keyboard == keyboard)
            );
        }
        assert!(matches!(
            anchored(Message::Stop, bounds, false),
            Message::Stop
        ));
    }

    #[test]
    fn menus_stay_inside_small_windows_and_near_their_trigger() {
        assert_eq!(
            menu_position(iced::Point::new(680.0, 480.0), Size::new(680.0, 480.0)),
            iced::Point::new(384.0, 224.0)
        );
        assert_eq!(
            menu_position(iced::Point::new(120.0, 80.0), Size::new(1040.0, 760.0)),
            iced::Point::new(120.0, 80.0)
        );
        assert_eq!(
            menu_position(iced::Point::ORIGIN, Size::new(100.0, 100.0)),
            iced::Point::new(8.0, 8.0)
        );
    }

    #[test]
    fn keyboard_activation_requires_focus_enabled_and_a_plain_nonrepeating_key() {
        let mut state = State::default();
        let enter = press(
            keyboard::key::Named::Enter,
            keyboard::Modifiers::empty(),
            false,
        );
        assert!(!state.activates(&enter, true));
        state.focus();
        assert!(state.activates(&enter, true));
        assert!(!state.activates(&enter, false));
        assert!(state.activates(
            &press(
                keyboard::key::Named::Space,
                keyboard::Modifiers::empty(),
                false
            ),
            true
        ));
        assert!(!state.activates(
            &press(
                keyboard::key::Named::Enter,
                keyboard::Modifiers::CTRL,
                false
            ),
            true
        ));
        assert!(!state.activates(
            &press(
                keyboard::key::Named::Enter,
                keyboard::Modifiers::empty(),
                true
            ),
            true
        ));
        state.unfocus();
        assert!(!state.activates(&enter, true));
    }
}
