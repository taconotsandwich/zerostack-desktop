mod app;
mod commands;
mod layout;
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
        .window(iced::window::Settings {
            size: (1040.0, 760.0).into(),
            min_size: Some((680.0, 480.0).into()),
            maximized: true,
            ..Default::default()
        })
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
        for flag in [
            "--print",
            "--setup",
            "--tutor",
            "--print-config",
            "--resume",
        ] {
            assert!(Cli::try_parse_from(["zerostack", "--desktop", flag]).is_err());
        }
        #[cfg(feature = "acp")]
        assert!(Cli::try_parse_from(["zerostack", "--desktop", "--acp"]).is_err());
        #[cfg(feature = "loop")]
        assert!(Cli::try_parse_from(["zerostack", "--desktop", "--loop"]).is_err());
    }
}
