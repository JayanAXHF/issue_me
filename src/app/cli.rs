use std::{env, fs};
use std::{fmt::Display, path::PathBuf, str::FromStr};

use anyhow::anyhow;
use clap::{CommandFactory, Parser};
use tracing_subscriber::filter::{self, Directive};

use crate::errors::AppError;
use crate::logging::{PROJECT_NAME, get_data_dir};

#[derive(Parser)]
#[clap(author, version = version(), about, long_about = None, styles = get_styles())]
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
    #[clap(required_unless_present_any = [ "print_log_dir", "set_token", "generate_man" ])]
    pub owner: Option<String>,
    /// GitHub repository name under `owner` (for example: `rust`).
    ///
    /// This is required unless `--print-log-dir` or `--set-token` is provided.
    #[clap(required_unless_present_any = [ "print_log_dir", "set_token", "generate_man" ])]
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
    /// Generate man pages using clap-mangen and exit.
    #[clap(long)]
    pub generate_man: bool,
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

pub const VERSION_MESSAGE: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "-",
    env!("VERGEN_GIT_DESCRIBE"),
    " (",
    env!("VERGEN_BUILD_DATE"),
    ")"
);

pub fn version() -> String {
    let author = clap::crate_authors!();

    // let current_exe_path = PathBuf::from(clap::crate_name!()).display().to_string();
    let data_dir_path = get_data_dir().display().to_string();

    format!(
        "\
{VERSION_MESSAGE}

Author: {author}

Data directory: {data_dir_path}"
    )
}

pub fn generate_man_pages() -> Result<PathBuf, AppError> {
    if cfg!(windows) {
        return Err(AppError::Other(anyhow!(
            "man page generation is not supported on Windows"
        )));
    }

    let cmd = Cli::command();

    let prefix = env::var("PREFIX").unwrap_or("/usr/local".to_string());

    let man1_dir = PathBuf::from(&prefix).join("share/man/man1");

    fs::create_dir_all(&man1_dir)?;

    let man1_file = format!("{}.1", &*PROJECT_NAME).to_lowercase();
    let mut man1_fd = fs::File::create(man1_dir.join(&man1_file))?;

    // Write them to the correct directories

    clap_mangen::Man::new(cmd).render(&mut man1_fd)?;
    println!("Installed manpages:");
    println!("  {}/share/man/man1/{}", prefix, man1_file);

    Ok(man1_dir.join(man1_file))
}
