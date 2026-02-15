use anyhow::anyhow;
use inquire::Password;

use crate::app::cli::Cli;
use crate::auth::AuthProvider;
use crate::errors::AppError;
use crate::github::GithubClient;
use crate::logging::LoggingConfig;
use crate::{logging, ui};
use std::sync::OnceLock;

pub struct App {
    pub owner: String,
    pub repo: String,
}

pub static GITHUB_CLIENT: OnceLock<GithubClient> = OnceLock::new();

impl App {
    pub async fn new(cli: Cli) -> Result<Self, AppError> {
        logging::init(LoggingConfig::new(cli.args.log_level))?;
        let auth = crate::auth::keyring::KeyringAuth::new("gitv")?;
        let token = match auth.get_token().ok() {
            Some(token) => token,
            None => Self::handle_no_token(&auth)?,
        };
        let github = GithubClient::new(Some(token))?;
        let _ = GITHUB_CLIENT.set(github);
        Ok(Self {
            owner: cli.args.owner.unwrap_or_default(),
            repo: cli.args.repo.unwrap_or_default(),
        })
    }

    pub async fn run(&mut self) -> Result<(), AppError> {
        use crate::ui::AppState;
        let current_user = GITHUB_CLIENT
            .get()
            .ok_or_else(|| AppError::Other(anyhow!("github client is not initialized")))?
            .inner()
            .current()
            .user()
            .await?
            .login;

        let ap = AppState::new(self.repo.clone(), self.owner.clone(), current_user);
        ui::run(ap).await
    }

    pub fn handle_no_token(auth: &impl AuthProvider) -> Result<String, AppError> {
        let prompt = Password::new("No token found. Please enter your github token")
            .with_display_toggle_enabled()
            .without_confirmation()
            .with_display_mode(inquire::PasswordDisplayMode::Masked);
        let token = prompt.prompt()?;
        auth.set_token(&token)?;
        Ok(token)
    }
}

pub mod cli;
