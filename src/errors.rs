use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("not implemented")]
    NotImplemented,
    #[error(transparent)]
    Octocrab(#[from] octocrab::Error),
    #[error(transparent)]
    Keyring(#[from] keyring::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error(transparent)]
    Inquire(#[from] inquire::error::InquireError),
    #[error("error sending message on channel")]
    TokioMpsc,
    #[error(transparent)]
    InitLoggingError(#[from] tracing_subscriber::util::TryInitError),
    #[error("error setting global {0}")]
    ErrorSettingGlobal(&'static str),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;
