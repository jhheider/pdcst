use anyhow::Result;
use std::path::Path;
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

pub fn setup_logging(log_dir: &Path, debug: bool) -> Result<()> {
    // Ensure log directory exists
    std::fs::create_dir_all(log_dir)?;

    // Create daily rotating file appender
    let file_appender = tracing_appender::rolling::daily(log_dir, "podcast-tui.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    // Determine log level
    let default_level = if debug { "debug" } else { "info" };
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("podcast_tui={}", default_level)));

    // Setup subscriber with file output
    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false) // No color codes in log files
                .with_target(true)
                .with_thread_ids(true),
        )
        .init();

    // Keep the guard alive for the duration of the program
    // In a real application, you'd want to store this somewhere
    std::mem::forget(_guard);

    Ok(())
}
