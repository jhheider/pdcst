use crate::models::Subscription;
use anyhow::{Context, Result};
use opml::{OPML, Outline};
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
        if let Some(xml_url) = &outline.xml_url
            && !xml_url.is_empty()
        {
            let title = outline.text.clone();
            let mut sub = Subscription::new(title, xml_url.clone());

            if let Some(html_url) = &outline.html_url {
                sub.website_url = Some(html_url.clone());
            }

            subscriptions.push(sub);
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_subscription(title: &str, rss_url: &str) -> Subscription {
        let mut sub = Subscription::new(title.to_string(), rss_url.to_string());
        sub.website_url = Some(format!(
            "https://{}.com",
            title.to_lowercase().replace(' ', "")
        ));
        sub
    }

    #[test]
    fn test_export_to_string() {
        let subs = vec![
            create_test_subscription("Test Podcast 1", "https://example.com/feed1.xml"),
            create_test_subscription("Test Podcast 2", "https://example.com/feed2.xml"),
        ];

        let result = OpmlExporter::export_to_string(&subs);
        assert!(result.is_ok());

        let opml_str = result.unwrap();
        assert!(opml_str.contains("Test Podcast 1"));
        assert!(opml_str.contains("https://example.com/feed1.xml"));
        assert!(opml_str.contains("Test Podcast 2"));
        assert!(opml_str.contains("https://example.com/feed2.xml"));
    }

    #[test]
    fn test_import_from_string() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Test Podcast" type="rss" xmlUrl="https://example.com/feed.xml" htmlUrl="https://example.com"/>
  </body>
</opml>"#;

        let result = OpmlImporter::import_from_string(opml_content);
        assert!(result.is_ok());

        let subs = result.unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].title, "Test Podcast");
        assert_eq!(subs[0].rss_url, "https://example.com/feed.xml");
        assert_eq!(subs[0].website_url, Some("https://example.com".to_string()));
    }

    #[test]
    fn test_import_nested_outlines() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Technology">
      <outline text="Podcast 1" type="rss" xmlUrl="https://example.com/feed1.xml"/>
      <outline text="Podcast 2" type="rss" xmlUrl="https://example.com/feed2.xml"/>
    </outline>
    <outline text="Podcast 3" type="rss" xmlUrl="https://example.com/feed3.xml"/>
  </body>
</opml>"#;

        let result = OpmlImporter::import_from_string(opml_content);
        assert!(result.is_ok());

        let subs = result.unwrap();
        assert_eq!(subs.len(), 3);
    }

    #[test]
    fn test_round_trip() {
        let original_subs = vec![
            create_test_subscription("Podcast A", "https://example.com/a.xml"),
            create_test_subscription("Podcast B", "https://example.com/b.xml"),
            create_test_subscription("Podcast C", "https://example.com/c.xml"),
        ];

        // Export to string
        let opml_str = OpmlExporter::export_to_string(&original_subs).unwrap();

        // Import back
        let imported_subs = OpmlImporter::import_from_string(&opml_str).unwrap();

        // Verify
        assert_eq!(imported_subs.len(), original_subs.len());
        for (original, imported) in original_subs.iter().zip(imported_subs.iter()) {
            assert_eq!(imported.title, original.title);
            assert_eq!(imported.rss_url, original.rss_url);
            assert_eq!(imported.website_url, original.website_url);
        }
    }

    #[test]
    fn test_export_and_import_file() {
        let temp_dir = TempDir::new().unwrap();
        let opml_path = temp_dir.path().join("test.opml");

        let subs = vec![
            create_test_subscription("File Test 1", "https://example.com/f1.xml"),
            create_test_subscription("File Test 2", "https://example.com/f2.xml"),
        ];

        // Export to file
        OpmlExporter::export_to_file(&subs, &opml_path).unwrap();
        assert!(opml_path.exists());

        // Import from file
        let imported = OpmlImporter::import_from_file(&opml_path).unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported[0].title, "File Test 1");
        assert_eq!(imported[1].title, "File Test 2");
    }

    #[test]
    fn test_empty_xml_url_ignored() {
        let opml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <body>
    <outline text="Valid Podcast" type="rss" xmlUrl="https://example.com/feed.xml"/>
    <outline text="Category" type="link"/>
    <outline text="Empty URL" type="rss" xmlUrl=""/>
  </body>
</opml>"#;

        let result = OpmlImporter::import_from_string(opml_content);
        assert!(result.is_ok());

        let subs = result.unwrap();
        // Only the valid podcast should be imported
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].title, "Valid Podcast");
    }
}
