use anyhow::{Context, Result};
use serde::Deserialize;

const NEWS_URL: &str = "https://launchercontent.mojang.com/v2/javaPatchNotes.json";

#[derive(Debug, Clone, Deserialize)]
pub struct NewsEntry {
    pub title: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub date: String,
    #[serde(default, rename = "type")]
    pub kind: String,
}

impl NewsEntry {
    pub fn date_short(&self) -> &str {
        self.date.get(..10).unwrap_or(&self.date)
    }

    pub fn link(&self) -> String {
        if self.version.is_empty() {
            "https://www.minecraft.net/en-us/articles".into()
        } else {
            format!("https://minecraft.wiki/w/Java_Edition_{}", self.version)
        }
    }
}

#[derive(Debug, Deserialize)]
struct NewsResponse {
    entries: Vec<NewsEntry>,
}

pub async fn fetch(client: &reqwest::Client) -> Result<Vec<NewsEntry>> {
    let resp: NewsResponse = client
        .get(NEWS_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parsing patch notes")?;
    Ok(resp.entries)
}
