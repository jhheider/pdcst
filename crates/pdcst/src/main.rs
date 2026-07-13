use anyhow::Result;
use clap::Parser;
use pdcst::Config;
use pdcst::app::App;
use pdcst::utils::logging;

#[derive(Parser, Debug)]
#[command(name = "pdcst")]
#[command(version)]
#[command(about = "A terminal-based podcast player", long_about = None)]
struct Cli {
    /// Path to custom configuration file
    #[arg(short, long)]
    config: Option<std::path::PathBuf>,

    /// Print the effective configuration (and its file path), then exit
    #[arg(long)]
    print_config: bool,

    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    /// Import subscriptions from an OPML file, then exit (skips duplicates)
    #[arg(long, value_name = "FILE")]
    import: Option<std::path::PathBuf>,

    /// Export subscriptions to an OPML file, then exit
    #[arg(long, value_name = "FILE")]
    export: Option<std::path::PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // Install ring as the rustls provider before any TLS client is built.
    pdcst::ensure_crypto_provider();

    let cli = Cli::parse();

    // Load configuration. `--config` is an explicit override; otherwise the
    // default path is read (and a commented template written there on first run).
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(Config::default_config_path);
    let config = if let Some(config_path) = cli.config.as_ref() {
        Config::load_from_file(config_path)?
    } else {
        Config::load_default()?
    };

    // Print the resolved config and exit, so the tunables are inspectable without
    // reading the file (and to show where that file lives).
    if cli.print_config {
        println!("# config file: {}", config_path.display());
        print!("{}", toml::to_string_pretty(&config)?);
        return Ok(());
    }

    // Setup logging
    logging::setup_logging(&config.log_dir, cli.debug)?;
    tracing::info!("Starting pdcst v{}", env!("CARGO_PKG_VERSION"));

    // Batch OPML operations run headlessly and exit, so the TUI never starts.
    if let Some(path) = cli.import.as_ref() {
        let mut app = App::new(config).await?;
        let imported = app.import_opml(path).await?;
        println!(
            "Imported {imported} subscription(s) from {}",
            path.display()
        );
        println!("Run `pdcst` to browse them.");
        return Ok(());
    }
    if let Some(path) = cli.export.as_ref() {
        let app = App::new(config).await?;
        app.export_opml(path).await?;
        println!("Exported subscriptions to {}", path.display());
        return Ok(());
    }

    // Create and run application
    let mut app = App::new(config).await?;
    app.run().await?;

    tracing::info!("Shutting down pdcst");
    Ok(())
}
