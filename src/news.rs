use anyhow::{Context, Result};
use serde::Deserialize;

const NEWS_URL: &str = "https://launchercontent.mojang.com/news.json";

#[derive(Debug, Clone, Deserialize)]
pub struct NewsEntry {
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub date: String,
    #[serde(default, rename = "readMoreLink")]
    pub read_more_link: String,
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
        .context("parsing news.json")?;
    Ok(resp.entries)
}
