//! AppState: the content library - loading subscriptions/episodes, search,
//! subscribe, queueing, downloads, refresh, and per-item actions.

#[allow(unused_imports)]
use super::*;

impl AppState {
    // Note: update() method removed - state is now event-driven
    // State fields are updated via events published by AudioPlayer, DownloadManager, etc.

    pub async fn load_subscriptions(&mut self) -> Result<()> {
        self.subscriptions = self.db.get_all_subscriptions().await?;
        // Keep the left-pane cursor in range after the list changes.
        self.subscription_index = self
            .subscription_index
            .min(self.subscriptions.len().saturating_sub(1));
        tracing::debug!("Loaded {} subscriptions", self.subscriptions.len());
        Ok(())
    }

    pub async fn load_episodes_for_subscription(
        &mut self,
        subscription_id: uuid::Uuid,
    ) -> Result<()> {
        self.episodes = self
            .db
            .get_episodes_for_subscription(subscription_id)
            .await?;
        // Keep the right-pane cursor in range after the list changes.
        self.episode_index = self
            .episode_index
            .min(self.episodes.len().saturating_sub(1));
        tracing::debug!("Loaded {} episodes", self.episodes.len());
        Ok(())
    }

    /// In the two-pane library, sync the right pane to the highlighted feed: load
    /// its episodes and mark it current. Cheap (a local indexed query), so it is
    /// safe to call on every left-pane cursor move for a live preview. A no-op
    /// when already showing that feed; clears the pane when the list is empty.
    pub async fn preview_selected_subscription(&mut self) -> Result<()> {
        if let Some(sub) = self.subscriptions.get(self.subscription_index).cloned() {
            if self.current_subscription.as_ref().map(|s| s.id) != Some(sub.id) {
                self.current_subscription = Some(sub.clone());
                self.load_episodes_for_subscription(sub.id).await?;
                // A different feed: start its episode pane at the newest episode.
                self.episode_index = 0;
            }
        } else {
            self.current_subscription = None;
            self.episodes.clear();
            self.episode_index = 0;
        }
        Ok(())
    }

    pub async fn select_item(&mut self) -> Result<()> {
        match self.current_view {
            View::Subscriptions => {
                if let Some(subscription) = self.subscriptions.get(self.subscription_index) {
                    self.current_subscription = Some(subscription.clone());
                    self.load_episodes_for_subscription(subscription.id).await?;
                    // Drill into the feed: focus the episode pane at the top.
                    self.focus_episodes();
                }
            }
            View::Episodes => {
                if let Some(episode) = self.episodes.get(self.episode_index) {
                    self.play_episode(episode.clone()).await?;
                }
            }
            View::Queue => {
                // Enter on a queue item jumps to it and plays.
                if let Some(episode) = self.queue_items.get(self.selected_index).cloned() {
                    self.play_episode(episode).await?;
                }
            }
            View::Search => {
                // Enter on a result (only meaningful with focus on the results
                // list; the input gate handles Enter while typing). In a normal
                // search this subscribes; in a feed-recovery picker it re-points
                // the target feed to the chosen result (via a confirm prompt).
                if self.search_focus == SearchFocus::Results
                    && let Some(result) = self.search_results.get(self.selected_index).cloned()
                {
                    if self.feed_fix_target.is_some() {
                        self.prompt_repoint_to(&result);
                    } else {
                        let title = result.title.clone();
                        self.subscribe_from_search_result(&result).await?;
                        self.set_status(format!("Subscribed to {}", title));
                    }
                }
            }
            View::Settings => {}
        }
        Ok(())
    }

    /// Search for podcasts using the iTunes Search API
    ///
    /// Updates `self.search_results` with the results.
    /// Run an iTunes search in a background task, delivering results via a
    /// `SearchCompleted` event (or `SearchFailed`). Non-blocking, so typing
    /// Enter never freezes the UI on the network call.
    pub fn start_search(&self, query: String) {
        tracing::info!("Searching for podcasts: {}", query);
        let search = self.podcast_search.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            match search.search(&query).await {
                Ok(results) => {
                    tracing::info!("Found {} results", results.len());
                    event_bus.publish(StateEvent::SearchCompleted { results });
                }
                Err(e) => {
                    event_bus.publish(StateEvent::SearchFailed {
                        error: e.to_string(),
                    });
                }
            }
        });
    }

    /// Subscribe to a podcast from a search result
    ///
    /// Creates a new subscription from the search result and adds it to the database.
    pub async fn subscribe_from_search_result(&mut self, result: &SearchResult) -> Result<()> {
        tracing::info!("Subscribing to: {}", result.title);

        // Create subscription from search result
        let mut subscription = Subscription::new(result.title.clone(), result.feed_url.clone());
        subscription.author = Some(result.artist.clone());
        subscription.artwork_url = result.artwork_url.clone();
        subscription.description = result.description.clone();

        // Insert into database
        self.db.insert_subscription(&subscription).await?;

        // Emit event
        self.event_bus
            .publish(crate::app::events::StateEvent::SubscriptionAdded {
                subscription_id: subscription.id,
            });

        // Reload subscriptions
        self.load_subscriptions().await?;

        // Fetch the new feed's episodes now (off the loop), so drilling in shows
        // content instead of an empty list. Without this, the feed stays empty
        // until the next scheduled refresh (up to a full interval away).
        let refresher = self.feed_refresher.clone();
        tokio::spawn(async move {
            if let Err(e) = refresher.refresh_one(subscription).await {
                tracing::warn!("Initial refresh after subscribe failed: {}", e);
            }
        });

        tracing::info!("Successfully subscribed to: {}", result.title);
        Ok(())
    }

    /// Remove the selected episode from the queue (Queue view only).
    pub async fn remove_selected_from_queue(&mut self) -> Result<bool> {
        if self.current_view == View::Queue
            && let Some(episode) = self.queue_items.get(self.selected_index).cloned()
        {
            self.queue_manager.remove_episode(episode.id).await?;
            tracing::info!("Removed '{}' from queue", episode.title);
            return Ok(true);
        }
        Ok(false)
    }

    // Item action methods

    /// Returns whether an episode was actually added (false if not in Episodes
    /// or the list is empty), so the caller only reports success on real work.
    pub async fn add_selected_to_queue(&mut self) -> Result<bool> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.episode_index)
        {
            self.queue_manager.add_episode(episode.id).await?;
            tracing::info!("Added '{}' to queue", episode.title);
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn download_selected_episode(&mut self) -> Result<bool> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.episode_index).cloned()
        {
            tracing::info!("Downloading episode: {}", episode.title);
            // Spawn download task to not block UI
            let download_manager = self.download_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = download_manager.download_episode(&episode).await {
                    tracing::error!("Download failed: {}", e);
                }
            });
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_selected_download(&mut self) -> Result<bool> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.episode_index)
            && episode.is_downloaded()
        {
            self.delete_download(episode).await?;
            // Reload episodes to update UI
            if let Some(sub) = &self.current_subscription {
                self.load_episodes_for_subscription(sub.id).await?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Unsubscribe from the selected feed (Subscriptions view). Cascade-deletes
    /// its episodes. Returns whether it acted.
    pub async fn unsubscribe_selected(&mut self) -> Result<bool> {
        if self.current_view == View::Subscriptions
            && let Some(sub) = self.subscriptions.get(self.subscription_index).cloned()
        {
            self.db.delete_subscription(sub.id).await?;
            self.event_bus
                .publish(crate::app::events::StateEvent::SubscriptionRemoved {
                    subscription_id: sub.id,
                });
            self.set_status(format!("Unsubscribed from {}", sub.title));
            self.load_subscriptions().await?;
            // Refresh the right pane against the row now under the cursor
            // (load_subscriptions already clamped subscription_index).
            self.preview_selected_subscription().await?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Refresh the selected feed in a background task (the refresher publishes
    /// FeedRefresh* events; the UI reloads on completion). Never blocks the loop.
    pub fn refresh_selected_subscription(&mut self) {
        if self.current_view == View::Subscriptions
            && let Some(subscription) = self.subscriptions.get(self.subscription_index).cloned()
        {
            tracing::info!("Refreshing subscription: {}", subscription.title);
            self.set_status(format!("Refreshing {}...", subscription.title));
            let refresher = self.feed_refresher.clone();
            tokio::spawn(async move {
                if let Err(e) = refresher.refresh_one(subscription).await {
                    tracing::error!("Refresh failed: {}", e);
                }
            });
        }
    }

    /// Refresh every feed in a background task (concurrency-bounded inside the
    /// refresher). Non-blocking; each feed's completion drives a UI reload.
    pub fn refresh_all_subscriptions(&mut self) {
        tracing::info!("Refreshing all subscriptions");
        self.set_status("Refreshing all feeds...".to_string());
        let refresher = self.feed_refresher.clone();
        let subscriptions = self.subscriptions.clone();
        tokio::spawn(async move {
            if let Err(e) = refresher.refresh_all(subscriptions).await {
                tracing::error!("Refresh all failed: {}", e);
            }
        });
    }

    /// Search (by title) for an up-to-date feed URL for the selected
    /// subscription, off the event loop. Most useful on a failing feed whose URL
    /// moved (a retired custom domain, a migrated host): iTunes usually still
    /// indexes the show under its title with the live feed URL. Publishes one of:
    /// `FeedFixFound` (a confident, differently-URLed title match -> confirm
    /// prompt), `FeedFixCandidates` (no confident match but results exist -> a
    /// picker), or `FeedFixNotFound` (nothing). Non-destructive: any switch waits
    /// on the user (a confirm, or a pick then confirm).
    pub fn find_feed_fix(&mut self) {
        if self.current_view != View::Subscriptions {
            return;
        }
        let Some(sub) = self.subscriptions.get(self.subscription_index).cloned() else {
            return;
        };

        self.set_status(format!(
            "Searching for an updated feed for {}...",
            sub.title
        ));
        let search = self.podcast_search.clone();
        let event_bus = self.event_bus.clone();
        tokio::spawn(async move {
            match search.search(&sub.title).await {
                Ok(results) => {
                    // A *confident* auto-suggestion needs a title that relates to
                    // the subscription (exact, or one title contains the other)
                    // and a different, non-empty URL - iTunes' top hit for an
                    // unindexed show is often an unrelated podcast, so we don't
                    // want to auto-offer that. When there's no confident match we
                    // still hand back the raw results as a picker (the user judges
                    // from the metadata), rather than a flat "none".
                    let needle = sub.title.to_lowercase();
                    let matched = results
                        .iter()
                        .find(|r| {
                            let hay = r.title.to_lowercase();
                            (hay == needle || hay.contains(&needle) || needle.contains(&hay))
                                && r.feed_url != sub.rss_url
                                && !r.feed_url.is_empty()
                        })
                        .cloned();

                    if let Some(r) = matched {
                        event_bus.publish(StateEvent::FeedFixFound {
                            subscription_id: sub.id,
                            podcast_title: r.title,
                            artist: r.artist,
                            new_url: r.feed_url,
                        });
                    } else if results.is_empty() {
                        event_bus.publish(StateEvent::FeedFixNotFound {
                            subscription_id: sub.id,
                        });
                    } else {
                        event_bus.publish(StateEvent::FeedFixCandidates {
                            subscription_id: sub.id,
                            results,
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("Feed-fix search failed for '{}': {}", sub.title, e);
                    event_bus.publish(StateEvent::FeedFixNotFound {
                        subscription_id: sub.id,
                    });
                }
            }
        });
    }

    /// Prompt to re-point the feed-fix target subscription to the given feed
    /// (chosen from the picker). Non-destructive: sets up the confirm, does not
    /// switch until confirmed.
    pub fn prompt_repoint_to(&mut self, result: &SearchResult) {
        let Some(target) = self.feed_fix_target else {
            return;
        };
        self.pending_action = Some(PendingAction::RepointFeed {
            subscription_id: target,
            new_url: result.feed_url.clone(),
        });
        self.modal = Modal::Confirm {
            message: format!(
                "Switch this feed to:\n{} by {}\n{}?",
                result.title, result.artist, result.feed_url
            ),
            action: "repoint-feed".to_string(),
        };
    }

    /// Execute the action awaiting confirmation (from a `Modal::Confirm`), if any.
    /// Currently just feed re-pointing: swap the subscription's URL, clear its
    /// error, and refresh it against the new feed. Clears the pending action and
    /// closes the modal either way.
    pub async fn apply_pending_action(&mut self) -> Result<()> {
        let action = self.pending_action.take();
        self.close_modal();

        match action {
            Some(PendingAction::RepointFeed {
                subscription_id,
                new_url,
            }) => {
                if let Err(e) = self
                    .db
                    .update_subscription_rss_url(subscription_id, &new_url)
                    .await
                {
                    self.show_error(format!("Could not switch feed: {e}"));
                    return Ok(());
                }
                // The re-point is committed: leave the recovery picker and return
                // to the library so the fixed feed is in view.
                self.feed_fix_target = None;
                self.set_view(View::Subscriptions);
                self.set_status("Feed URL updated; refreshing...".to_string());
                self.load_subscriptions().await?;
                self.preview_selected_subscription().await?;
                // Refresh against the new URL now, off the loop, so the fix is
                // visible without waiting for the next scheduled pass.
                if let Some(sub) = self.db.get_subscription(subscription_id).await? {
                    let refresher = self.feed_refresher.clone();
                    tokio::spawn(async move {
                        if let Err(e) = refresher.refresh_one(sub).await {
                            tracing::warn!("Refresh after feed re-point failed: {}", e);
                        }
                    });
                }
            }
            None => {}
        }
        Ok(())
    }

    /// Cycle the selected subscription's auto-queue setting through
    /// off -> add-to-bottom -> add-to-top -> off. New episodes from an on feed
    /// are auto-added to the queue at publish time (see the refresher hook).
    pub async fn cycle_selected_auto_queue(&mut self) -> Result<()> {
        if self.current_view == View::Subscriptions
            && let Some(sub) = self.subscriptions.get(self.subscription_index).cloned()
        {
            let (auto_queue, to_top, label) = match (sub.auto_queue, sub.auto_queue_to_top) {
                (false, _) => (true, false, "auto-queue: bottom"),
                (true, false) => (true, true, "auto-queue: top"),
                (true, true) => (false, false, "auto-queue: off"),
            };
            self.db
                .update_subscription_auto_queue(sub.id, auto_queue, to_top)
                .await?;
            self.set_status(format!("{} - {}", sub.title, label));
            self.load_subscriptions().await?;
        }
        Ok(())
    }

    pub async fn toggle_played_status(&mut self) -> Result<bool> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.episode_index)
        {
            let episode_id = episode.id;
            let new_status = !episode.played;
            self.db.mark_episode_played(episode_id, new_status).await?;
            tracing::info!(
                "Marked episode as {}",
                if new_status { "played" } else { "unplayed" }
            );

            // Emit event
            if new_status {
                self.event_bus
                    .publish(crate::app::events::StateEvent::EpisodeMarkedPlayed { episode_id });
            } else {
                self.event_bus
                    .publish(crate::app::events::StateEvent::EpisodeMarkedUnplayed { episode_id });
            }

            // Reload episodes to update UI
            if let Some(sub) = &self.current_subscription {
                self.load_episodes_for_subscription(sub.id).await?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    // Queue management

    pub async fn load_queue(&mut self) -> Result<()> {
        // Load queue items from database
        let queue_data = self.db.get_queue().await?;
        let mut episodes = Vec::new();

        for item in queue_data {
            if let Some(episode) = self.db.get_episode(item.episode_id).await? {
                episodes.push(episode);
            }
        }

        self.queue_items = episodes;
        Ok(())
    }

    /// Download an episode
    ///
    /// Downloads the episode audio to the configured download directory.
    /// Progress can be tracked via `get_download_progress()`.
    pub async fn download_episode(&self, episode: &Episode) -> Result<()> {
        tracing::info!("Starting download for: {}", episode.title);
        self.download_manager.download_episode(episode).await?;
        Ok(())
    }

    /// Get the download progress for a specific episode
    ///
    /// Returns `None` if the episode is not currently downloading.
    pub async fn get_download_progress(
        &self,
        episode_id: uuid::Uuid,
    ) -> Option<Arc<crate::download::DownloadProgress>> {
        self.download_manager
            .get_download_progress(episode_id)
            .await
    }

    /// Get all active downloads
    ///
    /// Returns a list of all episodes currently being downloaded with their progress.
    pub async fn get_active_downloads(&self) -> Vec<Arc<crate::download::DownloadProgress>> {
        self.download_manager.get_active_downloads().await
    }

    /// Cancel an active download
    ///
    /// Stops the download and cleans up any partial files.
    pub async fn cancel_download(&self, episode_id: uuid::Uuid) -> Result<()> {
        tracing::info!("Cancelling download for episode: {}", episode_id);
        self.download_manager.cancel_download(episode_id).await?;
        Ok(())
    }

    /// Delete a downloaded episode
    ///
    /// Removes the downloaded file from disk and updates the database.
    pub async fn delete_download(&self, episode: &Episode) -> Result<()> {
        tracing::info!("Deleting download for: {}", episode.title);
        self.download_manager.delete_download(episode).await?;
        Ok(())
    }
}
