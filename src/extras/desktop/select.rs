//! Selectable message text. The text itself is `iced_selection`'s; this
//! wrapper keeps a press on it from counting as taken by a widget.

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree, Widget};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector, mouse};

/// Message text that can be selected. The text captures the press that
/// starts a selection; this lets the press through as untaken, so clicking
/// a message still closes an open menu, as a click anywhere else does.
pub(crate) struct Selectable<'a, Message> {
    content: Element<'a, Message>,
}

pub(crate) fn selectable<'a, Message>(
    content: impl Into<Element<'a, Message>>,
) -> Selectable<'a, Message> {
    Selectable {
        content: content.into(),
    }
}

impl<Message> Widget<Message, Theme, Renderer> for Selectable<'_, Message> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
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
        let content = self.content.as_widget_mut();
        let tree = &mut tree.children[0];
        if !matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_))) {
            content.update(
                tree, event, layout, cursor, renderer, clipboard, shell, viewport,
            );
            return;
        }
        // Everything the press did but the capture.
        let mut messages = Vec::new();
        let mut inner = Shell::new(&mut messages);
        content.update(
            tree, event, layout, cursor, renderer, clipboard, &mut inner, viewport,
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
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<Selectable<'a, Message>> for Element<'a, Message> {
    fn from(selectable: Selectable<'a, Message>) -> Self {
        Element::new(selectable)
    }
}
