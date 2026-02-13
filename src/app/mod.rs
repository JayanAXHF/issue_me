use clap::Parser;
use inquire::Password;

use crate::auth::AuthProvider;
use crate::errors::AppError;
use crate::github::GithubClient;
use crate::{logging, ui};
use std::sync::OnceLock;

pub struct App {
    pub owner: String,
    pub repo: String,
}

pub static GITHUB_CLIENT: OnceLock<GithubClient> = OnceLock::new();

impl App {
    pub async fn new() -> Result<Self, AppError> {
        let cli = cli::Cli::parse();
        let mut auth = crate::auth::keyring::KeyringAuth::new("issue_me")?;
        let token = match auth.get_token().ok() {
            Some(token) => token,
            None => Self::handle_no_token(&mut auth)?,
        };
        let github = GithubClient::new(Some(token))?;
        let _ = GITHUB_CLIENT.set(github);
        Ok(Self {
            owner: cli.args.owner,
            repo: cli.args.repo,
        })
    }

    pub async fn run(&mut self) -> Result<(), AppError> {
        use crate::ui::AppState;
        logging::init()?;
        let current_user = GITHUB_CLIENT
            .get()
            .unwrap()
            .inner()
            .current()
            .user()
            .await?
            .login;

        let ap = AppState::new(self.repo.clone(), self.owner.clone(), current_user);
        ui::run(ap).await
    }

    pub fn handle_no_token(auth: &mut impl AuthProvider) -> Result<String, AppError> {
        let prompt = Password::new("No token found. Please enter your github token")
            .with_display_toggle_enabled()
            .without_confirmation()
            .with_display_mode(inquire::PasswordDisplayMode::Masked);
        let token = prompt.prompt()?;
        auth.set_token(&token)?;
        Ok(token)
    }
}

pub mod actions;
pub mod cli;
pub mod commands;
pub mod state;
