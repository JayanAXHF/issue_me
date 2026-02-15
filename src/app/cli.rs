use std::{fmt::Display, str::FromStr};

use clap::Parser;
use tracing_subscriber::filter::{self, Directive};

#[derive(Parser)]
#[clap(author, version, about, long_about = None, styles = get_styles())]
pub struct Cli {
    /// Top-level CLI arguments controlling repository selection and runtime behavior.
    #[clap(flatten)]
    pub args: Args,
}

#[derive(clap::Args, Clone)]
pub struct Args {
    /// GitHub repository owner or organization (for example: `rust-lang`).
    ///
    /// This is required unless `--print-log-dir` or `--set-token` is provided.
    #[clap(required_unless_present_any = [ "print_log_dir", "set_token" ])]
    pub owner: Option<String>,
    /// GitHub repository name under `owner` (for example: `rust`).
    ///
    /// This is required unless `--print-log-dir` or `--set-token` is provided.
    #[clap(required_unless_present_any = [ "print_log_dir", "set_token" ])]
    pub repo: Option<String>,
    /// Global logging verbosity used by the application logger.
    ///
    /// Defaults to `info`.
    #[clap(long, short, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,
    /// Prints the directory where log files are written and exits.
    #[clap(long, short)]
    pub print_log_dir: bool,
    /// Stores/updates the GitHub token in the configured credential store.
    ///
    /// When provided, this command updates the saved token value.
    #[clap(long, short)]
    pub set_token: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    None,
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            LogLevel::Trace => "trace",
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::None => "none",
        };
        write!(f, "{s}")
    }
}

impl TryFrom<LogLevel> for Directive {
    type Error = filter::ParseError;
    fn try_from(value: LogLevel) -> Result<Self, Self::Error> {
        Directive::from_str(&value.to_string())
    }
}

// Source - https://stackoverflow.com/a/76916424
// Posted by Praveen Perera, modified by community. See post 'Timeline' for change history
// Retrieved 2026-02-15, License - CC BY-SA 4.0

pub fn get_styles() -> clap::builder::Styles {
    use clap::builder::styling::{AnsiColor, Color, Style};
    clap::builder::Styles::styled()
        .usage(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Cyan))),
        )
        .header(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Green))),
        )
        .literal(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Cyan))),
        )
        .invalid(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Red))),
        )
        .error(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Red))),
        )
        .valid(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::Cyan))),
        )
        .placeholder(
            Style::new()
                .bold()
                .fg_color(Some(Color::Ansi(AnsiColor::BrightBlue))),
        )
}
