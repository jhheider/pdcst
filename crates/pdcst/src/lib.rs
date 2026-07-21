pub mod app;
pub mod audio;
pub mod config;
pub mod download;
pub mod feed;
pub mod models;
pub mod queue;
pub mod retention;
pub mod search;
pub mod storage;
pub mod ui;
pub mod utils;

pub use app::App;
pub use models::Config;

/// Install ring as the process-wide rustls CryptoProvider. Idempotent and cheap
/// after the first call. reqwest uses rustls with no baked-in provider (to force
/// ring over aws-lc), so a provider must be installed before any client is
/// built; call this at every construction site, including in tests, which
/// never run `main()`.
pub fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
