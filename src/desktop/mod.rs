mod app;
mod commands;
mod panels;
mod style;
mod view;
mod worker;

use app::App;

use crate::cli::Cli;

pub(crate) fn run(cli: Cli) -> anyhow::Result<()> {
    iced::application(move || App::new(cli.clone()), App::update, App::view)
        .title("zerostack")
        .theme(style::theme())
        .subscription(App::subscription)
        .window_size((1040.0, 760.0))
        .exit_on_close_request(false)
        .run()?;
    Ok(())
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
