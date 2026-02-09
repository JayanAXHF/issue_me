use tracing_error::ErrorLayer;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::errors::AppError;

pub fn init() -> Result<(), AppError> {
    //TODO: Add proper directory for logs
    let log_file = std::fs::File::create("issue_me.log")?;
    let file_subscriber = fmt::layer()
        .with_file(true)
        .with_line_number(true)
        .with_writer(log_file)
        .with_target(true)
        .with_ansi(false);
    tracing_subscriber::registry()
        .with(file_subscriber)
        .with(ErrorLayer::default())
        .try_init()?;
    Ok(())
}
