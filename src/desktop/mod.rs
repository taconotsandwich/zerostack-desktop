mod worker;

use std::sync::Arc;

use iced::widget::{button, column, container, scrollable, text, text_input};
use iced::{Element, Fill, Task, Theme};

use crate::cli::Cli;

pub(crate) fn run(cli: Cli) -> anyhow::Result<()> {
    iced::application(move || App::new(cli.clone()), App::update, App::view)
        .title("zerostack")
        .theme(Theme::Dark)
        .window_size((1040.0, 760.0))
        .run()?;
    Ok(())
}

struct App {
    worker: worker::Worker,
    snapshot: Option<Arc<worker::Snapshot>>,
    draft: String,
    error: String,
    busy: bool,
}

#[derive(Debug, Clone)]
enum Message {
    Ready(worker::Reply),
    Edit(String),
    Send,
}

impl App {
    fn new(cli: Cli) -> (Self, Task<Message>) {
        let (worker, ready) = worker::Worker::start(cli);
        (
            Self {
                worker,
                snapshot: None,
                draft: String::new(),
                error: String::new(),
                busy: true,
            },
            Task::perform(worker::receive(ready), Message::Ready),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Ready(result) => {
                self.busy = false;
                match result {
                    Ok(snapshot) => self.snapshot = Some(snapshot),
                    Err(error) => self.error = error,
                }
            }
            Message::Edit(value) => self.draft = value,
            Message::Send if !self.busy && !self.draft.trim().is_empty() => {
                self.busy = true;
                return Task::perform(
                    self.worker
                        .clone()
                        .request(worker::Operation::Run(std::mem::take(&mut self.draft))),
                    Message::Ready,
                );
            }
            Message::Send => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let mut messages = column![].spacing(16);
        if let Some(snapshot) = &self.snapshot {
            for message in &snapshot.session.messages {
                messages = messages.push(text(message.content.as_str()));
            }
        }
        container(
            column![
                scrollable(messages).height(Fill),
                text(if self.busy { "Working…" } else { &self.error }),
                text_input("Ask anything", &self.draft)
                    .on_input(Message::Edit)
                    .on_submit(Message::Send),
                button("Send").on_press_maybe((!self.busy).then_some(Message::Send)),
            ]
            .spacing(12),
        )
        .padding(24)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::cli::Cli;

    #[test]
    fn desktop_is_opt_in_and_excludes_terminal_setup() {
        assert!(!Cli::try_parse_from(["zerostack"]).unwrap().desktop);
        assert!(
            Cli::try_parse_from(["zerostack", "--desktop"])
                .unwrap()
                .desktop
        );
        for flag in ["--print", "--setup", "--tutor", "--print-config"] {
            assert!(Cli::try_parse_from(["zerostack", "--desktop", flag]).is_err());
        }
    }
}
