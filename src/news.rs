use anyhow::{Context, Result};
use serde::Deserialize;

const NEWS_URL: &str = "https://launchercontent.mojang.com/v2/news.json";
const PATCH_URL: &str = "https://launchercontent.mojang.com/v2/javaPatchNotes.json";
const CONTENT_BASE: &str = "https://launchercontent.mojang.com/v2/";

#[derive(Debug, Clone)]
pub struct NewsEntry {
    pub title: String,
    pub date: String,
    pub kind: String,
    pub short_text: String,
    pub read_more_link: String,
    pub content_path: String,
    pub article_body: String,
}

impl NewsEntry {
    pub fn date_short(&self) -> &str {
        self.date.get(..10).unwrap_or(&self.date)
    }
}

#[derive(Debug, Deserialize)]
struct RawNewsResponse {
    entries: Vec<RawNewsEntry>,
}

#[derive(Debug, Deserialize)]
struct RawNewsEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    text: String,
    #[serde(default, rename = "readMoreLink")]
    read_more_link: String,
    #[serde(default, rename = "articleBody")]
    article_body: String,
}

#[derive(Debug, Deserialize)]
struct RawPatchResponse {
    entries: Vec<RawPatchEntry>,
}

#[derive(Debug, Deserialize)]
struct RawPatchEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default, rename = "contentPath")]
    content_path: String,
}

pub async fn fetch(client: &reqwest::Client) -> Result<Vec<NewsEntry>> {
    let (news, patches) = tokio::join!(
        fetch_news(client),
        fetch_patches(client),
    );
    let mut all: Vec<NewsEntry> = Vec::new();
    if let Ok(mut n) = news {
        all.append(&mut n);
    }
    if let Ok(mut p) = patches {
        all.append(&mut p);
    }
    all.sort_by(|a, b| b.date.cmp(&a.date));
    Ok(all)
}

async fn fetch_news(client: &reqwest::Client) -> Result<Vec<NewsEntry>> {
    let resp: RawNewsResponse = client
        .get(NEWS_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parsing news.json")?;
    Ok(resp
        .entries
        .into_iter()
        .map(|e| NewsEntry {
            title: e.title,
            date: e.date,
            kind: e.category,
            short_text: e.text,
            read_more_link: e.read_more_link,
            content_path: String::new(),
            article_body: e.article_body,
        })
        .collect())
}

async fn fetch_patches(client: &reqwest::Client) -> Result<Vec<NewsEntry>> {
    let resp: RawPatchResponse = client
        .get(PATCH_URL)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parsing patch notes")?;
    Ok(resp
        .entries
        .into_iter()
        .map(|e| NewsEntry {
            title: e.title,
            date: e.date,
            kind: e.kind,
            short_text: String::new(),
            read_more_link: String::new(),
            content_path: e.content_path,
            article_body: String::new(),
        })
        .collect())
}

#[derive(Debug, Clone)]
pub struct Article {
    pub title: String,
    pub date: String,
    pub kind: String,
    pub source_url: String,
    pub blocks: Vec<Block>,
    pub read_more_link: String,
}

#[derive(Debug, Clone)]
pub enum Block {
    Heading(u8, String),
    Paragraph(String),
    Bullet(String),
}

#[derive(Debug, Deserialize)]
struct RawArticle {
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    body: String,
}

pub async fn fetch_article(client: &reqwest::Client, entry: NewsEntry) -> Result<Article> {
    if !entry.content_path.is_empty() {
        let path = entry.content_path.trim_start_matches('/');
        let url = format!("{CONTENT_BASE}{path}");
        let raw: RawArticle = client
            .get(&url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
            .context("parsing article body")?;
        return Ok(Article {
            title: if !raw.title.is_empty() { raw.title } else { entry.title.clone() },
            date: if !raw.date.is_empty() { raw.date } else { entry.date.clone() },
            kind: if !raw.kind.is_empty() { raw.kind } else { entry.kind.clone() },
            source_url: "https://www.minecraft.net/en-us/articles".into(),
            blocks: parse_html(&raw.body),
            read_more_link: String::new(),
        });
    }

    if !entry.article_body.is_empty() {
        return Ok(Article {
            title: entry.title.clone(),
            date: entry.date.clone(),
            kind: entry.kind.clone(),
            source_url: "https://www.minecraft.net/en-us/articles".into(),
            blocks: parse_html(&entry.article_body),
            read_more_link: entry.read_more_link,
        });
    }

    let mut blocks = Vec::new();
    if !entry.short_text.is_empty() {
        blocks.push(Block::Paragraph(entry.short_text.clone()));
    }
    blocks.push(Block::Paragraph(
        "The full text isn't available inside the launcher. \
         Use the link below to read it on minecraft.net."
            .into(),
    ));
    Ok(Article {
        title: entry.title,
        date: entry.date,
        kind: entry.kind,
        source_url: "https://www.minecraft.net/en-us/articles".into(),
        blocks,
        read_more_link: entry.read_more_link,
    })
}

enum Tok {
    Open(String),
    Close(String),
    Text(String),
}

fn tokenize(html: &str) -> Vec<Tok> {
    let mut tokens = Vec::new();
    let mut buf = String::new();
    let mut chars = html.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<' {
            if !buf.is_empty() {
                tokens.push(Tok::Text(decode_entities(&buf)));
                buf.clear();
            }
            let mut tag = String::new();
            for c2 in chars.by_ref() {
                if c2 == '>' {
                    break;
                }
                tag.push(c2);
            }
            let trimmed = tag.trim();
            if let Some(rest) = trimmed.strip_prefix('/') {
                let name = rest.split_whitespace().next().unwrap_or("").to_lowercase();
                tokens.push(Tok::Close(name));
            } else {
                let bare = trimmed.trim_end_matches('/').trim();
                let name = bare.split_whitespace().next().unwrap_or("").to_lowercase();
                tokens.push(Tok::Open(name));
            }
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        tokens.push(Tok::Text(decode_entities(&buf)));
    }
    tokens
}

fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let mut entity = String::new();
        let mut closed = false;
        for c2 in chars.by_ref() {
            if c2 == ';' {
                closed = true;
                break;
            }
            if entity.len() > 12 || c2.is_whitespace() {
                out.push('&');
                out.push_str(&entity);
                out.push(c2);
                break;
            }
            entity.push(c2);
        }
        if !closed {
            continue;
        }
        let replacement = match entity.as_str() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" | "#39" => "'",
            "nbsp" => " ",
            "rsquo" | "lsquo" => "'",
            "rdquo" | "ldquo" => "\"",
            "mdash" => "—",
            "ndash" => "–",
            "hellip" => "…",
            "trade" => "™",
            "copy" => "©",
            "reg" => "®",
            _ => "",
        };
        out.push_str(replacement);
    }
    out
}

fn heading_level(tag: &str) -> Option<u8> {
    if let Some(rest) = tag.strip_prefix('h') {
        if let Ok(n) = rest.parse::<u8>() {
            if (1..=6).contains(&n) {
                return Some(n);
            }
        }
    }
    None
}

fn parse_html(html: &str) -> Vec<Block> {
    let tokens = tokenize(html);
    let mut blocks = Vec::new();
    let mut buf = String::new();
    let mut kind = BlockState::Paragraph;

    let flush = |kind: &BlockState, buf: &mut String, blocks: &mut Vec<Block>| {
        let collapsed = collapse_ws(buf);
        if collapsed.is_empty() {
            buf.clear();
            return;
        }
        match kind {
            BlockState::Paragraph => blocks.push(Block::Paragraph(collapsed)),
            BlockState::Heading(n) => blocks.push(Block::Heading(*n, collapsed)),
            BlockState::Bullet => blocks.push(Block::Bullet(collapsed)),
        }
        buf.clear();
    };

    for tok in tokens {
        match tok {
            Tok::Open(name) => {
                if name == "p" {
                    flush(&kind, &mut buf, &mut blocks);
                    kind = BlockState::Paragraph;
                } else if let Some(n) = heading_level(&name) {
                    flush(&kind, &mut buf, &mut blocks);
                    kind = BlockState::Heading(n);
                } else if name == "li" {
                    flush(&kind, &mut buf, &mut blocks);
                    kind = BlockState::Bullet;
                } else if name == "br" {
                    buf.push(' ');
                }
            }
            Tok::Close(name) => {
                if name == "p"
                    || name == "li"
                    || heading_level(&name).is_some()
                    || name == "ul"
                    || name == "ol"
                {
                    flush(&kind, &mut buf, &mut blocks);
                    kind = BlockState::Paragraph;
                }
            }
            Tok::Text(t) => buf.push_str(&t),
        }
    }
    flush(&kind, &mut buf, &mut blocks);
    blocks
}

#[derive(Debug, Clone, Copy)]
enum BlockState {
    Paragraph,
    Heading(u8),
    Bullet,
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}
