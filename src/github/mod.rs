use crate::errors::AppError;

pub struct GithubClient {
    inner: octocrab::Octocrab,
}

impl std::ops::Deref for GithubClient {
    type Target = octocrab::Octocrab;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl GithubClient {
    pub fn new(token: Option<String>) -> Result<Self, AppError> {
        let mut builder = octocrab::Octocrab::builder();
        if let Some(token) = token {
            builder = builder.personal_token(token);
        }
        let inner = builder.build()?;
        Ok(Self { inner })
    }

    pub fn inner(&self) -> &octocrab::Octocrab {
        &self.inner
    }
}
