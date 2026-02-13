use std::{env, path::PathBuf, sync::LazyLock};

use directories::ProjectDirs;
use tracing_error::ErrorLayer;
use tracing_subscriber::{
    EnvFilter, Layer,
    fmt::{self},
    layer::SubscriberExt,
    util::SubscriberInitExt,
};

use crate::{app::cli::LogLevel, errors::AppError};

pub static PROJECT_NAME: LazyLock<String> =
    LazyLock::new(|| env!("CARGO_CRATE_NAME").to_uppercase().to_string());
pub static DATA_FOLDER: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    env::var(format!("{}_DATA", &*PROJECT_NAME))
        .ok()
        .map(PathBuf::from)
});

pub static LOG_ENV: LazyLock<String> = LazyLock::new(|| format!("{}_LOG_LEVEL", &*PROJECT_NAME));
pub static LOG_FILE: LazyLock<String> = LazyLock::new(|| format!("{}.log", env!("CARGO_PKG_NAME")));

pub(crate) fn get_data_dir() -> PathBuf {
    if let Some(s) = DATA_FOLDER.clone() {
        s
    } else if let Some(proj_dirs) = project_directory() {
        proj_dirs.data_local_dir().to_path_buf()
    } else {
        PathBuf::from(".").join(".data")
    }
}

pub(crate) fn project_directory() -> Option<ProjectDirs> {
    ProjectDirs::from("com", "jayanaxhf", env!("CARGO_PKG_NAME"))
}

pub fn init(cfg: LoggingConfig) -> Result<(), AppError> {
    //TODO: Add proper directory for logs
    let directory = get_data_dir();
    std::fs::create_dir_all(directory.clone())?;
    let log_path = directory.join(&*LOG_FILE);
    let log_file = std::fs::File::create(log_path)?;

    let env_filter = EnvFilter::builder()
        .with_default_directive(cfg.level.try_into().map_err(anyhow::Error::from)?);
    // If the `RUST_LOG` environment variable is set, use that as the default, otherwise use the
    // value of the `LOG_ENV` environment variable. If the `LOG_ENV` environment variable contains
    // errors, then this will return an error.
    let env_filter = env_filter
        .try_from_env()
        .or_else(|_| env_filter.with_env_var(LOG_ENV.clone()).from_env())
        .map_err(Into::<anyhow::Error>::into)?;
    let file_subscriber = fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .with_writer(log_file)
        .with_ansi(false)
        .with_target(true)
        .with_filter(env_filter);
    tracing_subscriber::registry()
        .with(file_subscriber)
        .with(ErrorLayer::default())
        .try_init()?;

    Ok(())
}

#[derive(Clone, Debug)]
pub struct LoggingConfig {
    level: LogLevel,
}

impl LoggingConfig {
    pub fn new(level: LogLevel) -> Self {
        Self { level }
    }
}
