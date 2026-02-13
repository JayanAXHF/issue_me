use std::{fmt::Display, str::FromStr};

use clap::Parser;
use tracing_subscriber::filter::{self, Directive};

#[derive(Parser)]
#[clap(author, version, about, long_about = None)]
pub struct Cli {
    #[clap(flatten)]
    pub args: Args,
}

#[derive(clap::Args, Clone)]
pub struct Args {
    #[clap(required_unless_present = "print_log_dir")]
    pub owner: Option<String>,
    #[clap(required_unless_present = "print_log_dir")]
    pub repo: Option<String>,
    #[clap(long, short, default_value_t = LogLevel::Info)]
    pub log_level: LogLevel,
    #[clap(long, short)]
    pub print_log_dir: bool,
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
