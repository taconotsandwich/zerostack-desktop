use iced::widget::{container, text};
use iced::{Element, Fill, Task, Theme};

use crate::cli::Cli;

pub(crate) fn run(_cli: Cli) -> anyhow::Result<()> {
    iced::application(|| ((), Task::none()), update, view)
        .title("zerostack")
        .theme(Theme::Dark)
        .window_size((1040.0, 760.0))
        .run()?;
    Ok(())
}

fn update(_state: &mut (), _message: ()) {}

fn view(_state: &()) -> Element<'_, ()> {
    container(text("zerostack"))
        .center_x(Fill)
        .center_y(Fill)
        .into()
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
