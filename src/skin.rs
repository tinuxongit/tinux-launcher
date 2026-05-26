use anyhow::{anyhow, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::Widget,
};
use serde::Deserialize;

const PREVIEW_W: u32 = 16;
const PREVIEW_H: u32 = 32;

#[derive(Debug, Clone)]
pub struct SkinPreview {
    pixels: Vec<[u8; 4]>,
}

impl SkinPreview {
    pub fn rows(&self) -> u16 {
        (PREVIEW_H as u16) / 2
    }
    pub fn cols(&self) -> u16 {
        PREVIEW_W as u16
    }

    fn px(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= PREVIEW_W || y >= PREVIEW_H {
            return [0, 0, 0, 0];
        }
        self.pixels[(y * PREVIEW_W + x) as usize]
    }
}

#[derive(Deserialize)]
struct Profile {
    properties: Vec<Property>,
}

#[derive(Deserialize)]
struct Property {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct Textures {
    textures: TexturesInner,
}

#[derive(Deserialize)]
struct TexturesInner {
    #[serde(rename = "SKIN")]
    skin: Option<TextureRef>,
}

#[derive(Deserialize)]
struct TextureRef {
    url: String,
}

/// Look up the public skin URL for a Microsoft / Minecraft account UUID.
pub async fn current_skin_url(client: &reqwest::Client, uuid: &str) -> Result<String> {
    let stripped: String = uuid.chars().filter(|c| *c != '-').collect();
    let url = format!("https://sessionserver.mojang.com/session/minecraft/profile/{stripped}");
    let prof: Profile = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parsing session profile")?;
    let prop = prof
        .properties
        .into_iter()
        .find(|p| p.name == "textures")
        .ok_or_else(|| anyhow!("no textures property"))?;
    let decoded = STANDARD.decode(prop.value).context("base64 textures")?;
    let tex: Textures = serde_json::from_slice(&decoded).context("textures json")?;
    tex.textures
        .skin
        .map(|s| s.url)
        .ok_or_else(|| anyhow!("no SKIN texture"))
}

pub async fn fetch_preview(client: &reqwest::Client, url: &str) -> Result<SkinPreview> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    decode_and_compose(&bytes)
}

fn decode_and_compose(bytes: &[u8]) -> Result<SkinPreview> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().context("png read_info")?;
    let mut raw = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).context("png next_frame")?;
    let w = info.width;
    let h = info.height;
    let stride = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => anyhow::bail!("unsupported skin color type: {other:?}"),
    };

    let pixel = |x: u32, y: u32| -> [u8; 4] {
        if x >= w || y >= h {
            return [0, 0, 0, 0];
        }
        let i = ((y * w + x) as usize) * stride;
        let r = raw[i];
        let g = raw[i + 1];
        let b = raw[i + 2];
        let a = if stride == 4 { raw[i + 3] } else { 255 };
        [r, g, b, a]
    };

    let mut out = vec![[0u8, 0, 0, 0]; (PREVIEW_W * PREVIEW_H) as usize];
    let put = |out: &mut [[u8; 4]], x: u32, y: u32, c: [u8; 4]| {
        if x < PREVIEW_W && y < PREVIEW_H {
            let idx = (y * PREVIEW_W + x) as usize;
            if c[3] >= 128 {
                out[idx] = [c[0], c[1], c[2], 255];
            } else if out[idx][3] == 0 && c[3] > 0 {
                out[idx] = [c[0], c[1], c[2], 255];
            }
        }
    };

    // (src_x, src_y, src_w, src_h, dst_x, dst_y)
    // Base layer
    let base = [
        (8, 8, 8, 8, 4, 0),      // head
        (20, 20, 8, 12, 4, 8),   // body
        (44, 20, 4, 12, 0, 8),   // right arm (viewer left)
        (36, 52, 4, 12, 12, 8),  // left arm (viewer right) — 1.8+
        (4, 20, 4, 12, 4, 20),   // right leg
        (20, 52, 4, 12, 8, 20),  // left leg — 1.8+
    ];
    let overlay = [
        (40, 8, 8, 8, 4, 0),     // head overlay (hat)
        (20, 36, 8, 12, 4, 8),   // body overlay (jacket)
        (44, 36, 4, 12, 0, 8),   // right arm overlay
        (52, 52, 4, 12, 12, 8),  // left arm overlay
        (4, 36, 4, 12, 4, 20),   // right leg overlay
        (4, 52, 4, 12, 8, 20),   // left leg overlay
    ];

    for &(sx, sy, sw, sh, dx, dy) in base.iter() {
        for j in 0..sh {
            for i in 0..sw {
                put(&mut out, dx + i, dy + j, pixel(sx + i, sy + j));
            }
        }
    }
    for &(sx, sy, sw, sh, dx, dy) in overlay.iter() {
        for j in 0..sh {
            for i in 0..sw {
                let c = pixel(sx + i, sy + j);
                if c[3] > 0 {
                    put(&mut out, dx + i, dy + j, c);
                }
            }
        }
    }

    Ok(SkinPreview { pixels: out })
}

pub struct SkinPreviewWidget<'a> {
    pub preview: &'a SkinPreview,
}

impl<'a> Widget for SkinPreviewWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let max_cx = (self.preview.cols()).min(area.width);
        let max_cy = (self.preview.rows()).min(area.height);
        for cy in 0..max_cy {
            for cx in 0..max_cx {
                let top = self.preview.px(cx as u32, (cy * 2) as u32);
                let bot = self.preview.px(cx as u32, (cy * 2 + 1) as u32);
                let cell = &mut buf[(area.x + cx, area.y + cy)];
                match (top[3] > 0, bot[3] > 0) {
                    (true, true) => {
                        cell.set_symbol("▀");
                        cell.set_fg(Color::Rgb(top[0], top[1], top[2]));
                        cell.set_bg(Color::Rgb(bot[0], bot[1], bot[2]));
                    }
                    (true, false) => {
                        cell.set_symbol("▀");
                        cell.set_fg(Color::Rgb(top[0], top[1], top[2]));
                    }
                    (false, true) => {
                        cell.set_symbol("▄");
                        cell.set_fg(Color::Rgb(bot[0], bot[1], bot[2]));
                    }
                    (false, false) => {}
                }
            }
        }
    }
}
