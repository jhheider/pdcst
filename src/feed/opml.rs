use crate::models::Subscription;
use anyhow::{Context, Result};
use opml::{Outline, OPML};
use std::path::Path;

pub struct OpmlImporter;

impl OpmlImporter {
    pub fn import_from_file(path: &Path) -> Result<Vec<Subscription>> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read OPML file: {}", path.display()))?;

        Self::import_from_string(&content)
    }

    pub fn import_from_string(content: &str) -> Result<Vec<Subscription>> {
        let opml = OPML::from_str(content).context("Failed to parse OPML")?;

        let mut subscriptions = Vec::new();

        for outline in &opml.body.outlines {
            Self::extract_subscriptions(outline, &mut subscriptions);
        }

        tracing::info!("Imported {} subscriptions from OPML", subscriptions.len());
        Ok(subscriptions)
    }

    fn extract_subscriptions(outline: &Outline, subscriptions: &mut Vec<Subscription>) {
        // Check if this outline is a podcast feed
        if let Some(xml_url) = &outline.xml_url {
            if !xml_url.is_empty() {
                let title = outline.text.clone();
                let mut sub = Subscription::new(title, xml_url.clone());

                if let Some(html_url) = &outline.html_url {
                    sub.website_url = Some(html_url.clone());
                }

                subscriptions.push(sub);
            }
        }

        // Recursively process child outlines
        for child in &outline.outlines {
            Self::extract_subscriptions(child, subscriptions);
        }
    }
}

pub struct OpmlExporter;

impl OpmlExporter {
    pub fn export_to_file(subscriptions: &[Subscription], path: &Path) -> Result<()> {
        let opml_string = Self::export_to_string(subscriptions)?;

        std::fs::write(path, opml_string)
            .with_context(|| format!("Failed to write OPML file: {}", path.display()))?;

        tracing::info!("Exported {} subscriptions to OPML", subscriptions.len());
        Ok(())
    }

    pub fn export_to_string(subscriptions: &[Subscription]) -> Result<String> {
        let mut opml = OPML {
            version: "2.0".to_string(),
            head: None,
            body: opml::Body {
                outlines: Vec::new(),
            },
        };

        for sub in subscriptions {
            let outline = Outline {
                text: sub.title.clone(),
                r#type: Some("rss".to_string()),
                xml_url: Some(sub.rss_url.clone()),
                html_url: sub.website_url.clone(),
                title: Some(sub.title.clone()),
                ..Default::default()
            };

            opml.body.outlines.push(outline);
        }

        let mut buffer = Vec::new();
        opml.to_writer(&mut buffer)
            .context("Failed to serialize OPML")?;

        String::from_utf8(buffer).context("Failed to convert OPML to string")
    }
}
