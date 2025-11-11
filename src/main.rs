use anyhow::Result;
use clap::Parser;
use podcast_tui::app::App;
use podcast_tui::utils::logging;
use podcast_tui::Config;

#[derive(Parser, Debug)]
#[command(name = "podcast-tui")]
#[command(about = "A terminal-based podcast player", long_about = None)]
struct Cli {
    /// Path to custom configuration file
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Load configuration
    let config = if let Some(config_path) = cli.config {
        Config::load_from_file(&config_path)?
    } else {
        Config::load_default()?
    };

    // Setup logging
    logging::setup_logging(&config.log_dir, cli.debug)?;
    tracing::info!("Starting podcast-tui v{}", env!("CARGO_PKG_VERSION"));

    // Create and run application
    let mut app = App::new(config).await?;
    app.run().await?;

    tracing::info!("Shutting down podcast-tui");
    Ok(())
}
