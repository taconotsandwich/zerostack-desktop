//! Selectable message text. The text itself is `iced_selection`'s; this
//! wrapper keeps a press on it from counting as taken by a widget, reports
//! what is selected, and floats a pill (the "Add to reply" button) over the
//! selection.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, clipboard, overlay, renderer};
use iced::keyboard::{self, key};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme, Vector, mouse};

/// Space between the pointer and the pill: about half a line, plus a gap,
/// so the pill clears the line the pointer let go on.
const CLEARANCE: f32 = 16.0;

/// Message text that can be selected. The text captures the press that
/// starts a selection; this lets the press through as untaken, so clicking
/// a message still closes an open menu, as a click anywhere else does.
pub(crate) struct Selectable<'a, Message> {
    content: Element<'a, Message>,
    on_pick: Option<Box<dyn Fn(Option<String>) -> Message + 'a>>,
    pill: Option<Element<'a, Message>>,
}

pub(crate) fn selectable<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> Selectable<'a, Message> {
    Selectable {
        content: content.into(),
        on_pick: None,
        pill: None,
    }
}

impl<'a, Message> Selectable<'a, Message> {
    /// Reports the selection when the mouse lets go of it, with its text,
    /// and with `None` once it is gone.
    pub(crate) fn on_pick(mut self, on_pick: impl Fn(Option<String>) -> Message + 'a) -> Self {
        self.on_pick = Some(Box::new(on_pick));
        self
    }

    /// Floats `pill` just above where the mouse let go of the selection.
    pub(crate) fn pill(mut self, pill: Option<impl Into<Element<'a, Message>>>) -> Self {
        self.pill = pill.map(Into::into);
        self
    }
}

/// The selection last reported, and where the mouse let go of it,
/// from the message's top left corner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Picked {
    pub text: String,
    pub at: Vector,
}

/// Records the selection the text holds now, `text`, let go at `at` if the
/// mouse just let go. Returns what to report: nothing while the selection
/// is unchanged.
pub(crate) fn settle(
    picked: &mut Option<Picked>,
    text: Option<String>,
    at: Option<Vector>,
) -> Option<Option<String>> {
    let old = picked.take();
    let text = match text {
        Some(text) => text,
        None => return old.map(|_| None),
    };
    let changed = old.as_ref().is_none_or(|old| old.text != text);
    *picked = Some(Picked {
        text: text.clone(),
        at: at.or(old.map(|old| old.at)).unwrap_or(Vector::ZERO),
    });
    changed.then_some(Some(text))
}

#[derive(Default)]
struct State {
    /// The keyboard modifiers held, to give back after a probe.
    modifiers: keyboard::Modifiers,
    /// A left press landed on this message and has not let go yet.
    pressed: bool,
    picked: Option<Picked>,
}

/// The text the content would copy now: its selection, if any. `iced_selection`
/// keeps the selection to itself, so this replays ⌘C into a clipboard that
/// only records; the real clipboard is never touched.
fn probe<Message>(
    content: &mut Element<'_, Message>,
    tree: &mut Tree,
    layout: Layout<'_>,
    cursor: mouse::Cursor,
    renderer: &Renderer,
    viewport: &Rectangle,
    modifiers: keyboard::Modifiers,
) -> Option<String> {
    let copy = keyboard::Event::KeyPressed {
        key: keyboard::Key::Character("c".into()),
        modified_key: keyboard::Key::Character("c".into()),
        physical_key: key::Physical::Code(key::Code::KeyC),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::COMMAND,
        text: None,
        repeat: false,
    };
    let mut recorder = Recorder(None);
    let mut ignored = Vec::new();
    let mut shell = Shell::new(&mut ignored);
    for event in [
        keyboard::Event::ModifiersChanged(keyboard::Modifiers::COMMAND),
        copy,
        keyboard::Event::ModifiersChanged(modifiers),
    ] {
        content.as_widget_mut().update(
            tree,
            &Event::Keyboard(event),
            layout,
            cursor,
            renderer,
            &mut recorder,
            &mut shell,
            viewport,
        );
    }
    recorder.0.filter(|text| !text.trim().is_empty())
}

/// A clipboard that keeps what is written to it and reads as empty.
struct Recorder(Option<String>);

impl Clipboard for Recorder {
    fn read(&self, _: clipboard::Kind) -> Option<String> {
        None
    }

    fn write(&mut self, _: clipboard::Kind, contents: String) {
        self.0 = Some(contents);
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Selectable<'_, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        let mut children = vec![Tree::new(&self.content)];
        children.extend(self.pill.as_ref().map(Tree::new));
        children
    }

    fn diff(&self, tree: &mut Tree) {
        match &self.pill {
            Some(pill) => tree.diff_children(&[&self.content, pill]),
            None => tree.diff_children(std::slice::from_ref(&self.content)),
        }
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
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

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
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
        let state = tree.state.downcast_mut::<State>();
        let content_tree = &mut tree.children[0];
        let bounds = layout.bounds();
        // Whether to look at the selection after the event, and where the
        // mouse let go of it.
        let mut look = false;
        let mut at = None;
        match event {
            Event::Mouse(mouse::Event::ButtonPressed(button)) => {
                // Everything the press did but the capture.
                let mut messages = Vec::new();
                let mut inner = Shell::new(&mut messages);
                self.content.as_widget_mut().update(
                    content_tree,
                    event,
                    layout,
                    cursor,
                    renderer,
                    clipboard,
                    &mut inner,
                    viewport,
                );
                let redraw = inner.redraw_request();
                let relayout = inner.is_layout_invalid();
                let rebuild = inner.are_widgets_invalid();
                for message in messages {
                    shell.publish(message);
                }
                shell.request_redraw_at(redraw);
                if relayout {
                    shell.invalidate_layout();
                }
                if rebuild {
                    shell.invalidate_widgets();
                }
                if *button == mouse::Button::Left {
                    state.pressed = cursor.is_over(bounds);
                    // A press clears the selection or starts another; a
                    // shift press grows it.
                    look = state.picked.is_some();
                }
            }
            _ => {
                self.content.as_widget_mut().update(
                    content_tree,
                    event,
                    layout,
                    cursor,
                    renderer,
                    clipboard,
                    shell,
                    viewport,
                );
                match event {
                    Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                        look = std::mem::take(&mut state.pressed);
                        at = cursor.land().position().map(|point| {
                            Vector::new(
                                (point.x - bounds.x).clamp(0.0, bounds.width),
                                (point.y - bounds.y).clamp(0.0, bounds.height),
                            )
                        });
                    }
                    Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                        state.modifiers = *modifiers;
                    }
                    // Keys can grow, shrink or clear the selection.
                    Event::Keyboard(keyboard::Event::KeyPressed { .. }) => {
                        look = state.picked.is_some();
                    }
                    _ => {}
                }
            }
        }
        if !look {
            return;
        }
        let text = probe(
            &mut self.content,
            content_tree,
            layout,
            cursor,
            renderer,
            viewport,
            state.modifiers,
        );
        if let (Some(report), Some(on_pick)) = (settle(&mut state.picked, text, at), &self.on_pick)
        {
            shell.publish(on_pick(report));
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
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        let picked = tree.state.downcast_ref::<State>().picked.as_ref();
        let (content_tree, rest) = tree.children.split_first_mut()?;
        let inner = self.content.as_widget_mut().overlay(
            content_tree,
            layout,
            renderer,
            viewport,
            translation,
        );
        let bounds = layout.bounds() + translation;
        let pill = match (self.pill.as_mut(), picked, rest.first_mut()) {
            (Some(pill), Some(picked), Some(tree)) => {
                let at = bounds.position() + picked.at;
                // Not once the selection scrolls out of view.
                viewport.contains(at).then(|| {
                    overlay::Element::new(Box::new(Pill {
                        pill,
                        tree,
                        at,
                        span: (bounds.x, bounds.x + bounds.width),
                        top: viewport.y,
                    }))
                })
            }
            _ => None,
        };
        match (inner, pill) {
            (Some(inner), Some(pill)) => {
                Some(overlay::Group::with_children(vec![inner, pill]).overlay())
            }
            (inner, pill) => inner.or(pill),
        }
    }
}

impl<'a, Message: 'a> From<Selectable<'a, Message>> for Element<'a, Message> {
    fn from(selectable: Selectable<'a, Message>) -> Self {
        Element::new(selectable)
    }
}

/// The pill, floating over everything, so it takes clicks before the
/// rows under it.
struct Pill<'a, 'b, Message> {
    pill: &'a mut Element<'b, Message>,
    tree: &'a mut Tree,
    /// Where the mouse let go, in window coordinates.
    at: Point,
    /// The message's left and right edges: the pill stays between them.
    span: (f32, f32),
    /// The top of the visible transcript: the pill drops below the pointer
    /// rather than rise past it.
    top: f32,
}

impl<Message> overlay::Overlay<Message, Theme, Renderer> for Pill<'_, '_, Message> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let node = self.pill.as_widget_mut().layout(
            self.tree,
            renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let size = node.size();
        let (left, right) = self.span;
        let x = (self.at.x - size.width / 2.0).clamp(left, (right - size.width).max(left));
        let above = self.at.y - CLEARANCE - size.height;
        let y = if above >= self.top {
            above
        } else {
            self.at.y + CLEARANCE
        };
        node.move_to(Point::new(x, y))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.pill.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        self.pill.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        if !cursor.is_over(layout.bounds()) {
            return mouse::Interaction::None;
        }
        self.pill.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }
}
