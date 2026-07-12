//! OPML import/export: the headless onboarding path (`--import`/`--export`).

use super::App;
use crate::app::events::StateEvent;
use crate::feed::{OpmlExporter, OpmlImporter};
use anyhow::{Context, Result};
use std::path::Path;

impl App {
    /// Import subscriptions from an OPML file
    ///
    /// Reads an OPML file and subscribes to all podcast feeds found within.
    /// Existing subscriptions with duplicate RSS URLs will be skipped.
    ///
    /// Returns the number of subscriptions successfully imported.
    pub async fn import_opml(&mut self, path: &Path) -> Result<usize> {
        tracing::info!("Importing OPML from: {}", path.display());

        // Parse OPML file (sync I/O in tokio::task::spawn_blocking)
        let path_buf = path.to_path_buf();
        let subscriptions =
            tokio::task::spawn_blocking(move || OpmlImporter::import_from_file(&path_buf))
                .await
                .context("OPML import task panicked")??;

        let total = subscriptions.len();
        let mut imported = 0;

        // Insert each subscription
        for sub in subscriptions {
            let subscription_id = sub.id;
            match self.state.db.insert_subscription(&sub).await {
                Ok(_) => {
                    tracing::debug!("Imported subscription: {}", sub.title);
                    imported += 1;

                    // Emit event
                    self.state
                        .event_bus
                        .publish(StateEvent::SubscriptionAdded { subscription_id });
                }
                Err(e) => {
                    // Log but continue - might be duplicate RSS URL
                    tracing::warn!("Failed to import {}: {}", sub.title, e);
                }
            }
        }

        // Reload subscriptions in UI
        self.state.load_subscriptions().await?;

        tracing::info!("Imported {}/{} subscriptions", imported, total);
        Ok(imported)
    }

    /// Export all subscriptions to an OPML file
    ///
    /// Writes all current subscriptions to an OPML file that can be imported
    /// by other podcast clients or re-imported later.
    pub async fn export_opml(&self, path: &Path) -> Result<()> {
        tracing::info!("Exporting OPML to: {}", path.display());

        // Get all subscriptions from database
        let subscriptions = self.state.db.get_all_subscriptions().await?;
        let count = subscriptions.len();

        // Write OPML file (sync I/O in tokio::task::spawn_blocking)
        let path_buf = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            OpmlExporter::export_to_file(&subscriptions, &path_buf)
        })
        .await
        .context("OPML export task panicked")??;

        tracing::info!("Exported {} subscriptions", count);
        Ok(())
    }
}
