use clap::Parser;
use gitv::{
    app::{
        App,
        cli::{Cli, generate_man_pages},
    },
    auth::AuthProvider,
    errors::AppError,
    logging,
};

#[tokio::main]
async fn main() -> anyhow::Result<(), AppError> {
    let cli = Cli::parse();
    if cli.args.generate_man {
        let output_dir = generate_man_pages()?;
        println!("Man page generated at: {}", output_dir.display());
        return Ok(());
    }
    if cli.args.print_log_dir {
        println!("Log directory: {}", logging::get_data_dir().display());
        return Ok(());
    }
    if let Some(ref token) = cli.args.set_token {
        let auth = gitv::auth::keyring::KeyringAuth::new("gitv")?;
        auth.set_token(token)?;
        return Ok(());
    }

    let mut app = App::new(cli).await?;
    app.run().await
}
