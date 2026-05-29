use anyhow::{anyhow, bail, Context, Result};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkinView {
    Front,
    Right,
    Back,
    Left,
}

impl SkinView {
    pub fn next(self) -> Self {
        match self {
            SkinView::Front => SkinView::Right,
            SkinView::Right => SkinView::Back,
            SkinView::Back => SkinView::Left,
            SkinView::Left => SkinView::Front,
        }
    }
    pub fn prev(self) -> Self {
        match self {
            SkinView::Front => SkinView::Left,
            SkinView::Right => SkinView::Front,
            SkinView::Back => SkinView::Right,
            SkinView::Left => SkinView::Back,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SkinPreview {
    src: Vec<[u8; 4]>,
    src_w: u32,
    src_h: u32,
}

/// Decoded cape texture. Rendered in its own widget (separate from the skin),
/// since users want to see the cape on its own panel rather than overlaid.
#[derive(Debug, Clone)]
pub struct CapePixels {
    pixels: Vec<[u8; 4]>,
    pub w: u32,
    pub h: u32,
}

impl CapePixels {
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.w || y >= self.h {
            return [0, 0, 0, 0];
        }
        self.pixels[(y * self.w + x) as usize]
    }
}

impl SkinPreview {
    pub fn rows(&self) -> u16 {
        (PREVIEW_H as u16) / 2
    }
    pub fn cols(&self) -> u16 {
        PREVIEW_W as u16
    }

    fn src_px(&self, x: u32, y: u32) -> [u8; 4] {
        if x >= self.src_w || y >= self.src_h {
            return [0, 0, 0, 0];
        }
        self.src[(y * self.src_w + x) as usize]
    }

    fn compose(&self, view: SkinView, cape: Option<&CapePixels>) -> Vec<[u8; 4]> {
        let mut out = vec![[0u8; 4]; (PREVIEW_W * PREVIEW_H) as usize];
        let put = |out: &mut [[u8; 4]], x: u32, y: u32, c: [u8; 4]| {
            if x < PREVIEW_W && y < PREVIEW_H && c[3] >= 128 {
                out[(y * PREVIEW_W + x) as usize] = [c[0], c[1], c[2], 255];
            }
        };
        let blit = |out: &mut [[u8; 4]],
                    sx: u32,
                    sy: u32,
                    sw: u32,
                    sh: u32,
                    dx: u32,
                    dy: u32,
                    mirror: bool| {
            for j in 0..sh {
                for i in 0..sw {
                    let src_i = if mirror { sw - 1 - i } else { i };
                    put(out, dx + i, dy + j, self.src_px(sx + src_i, sy + j));
                }
            }
        };
        // Cape outside face on a vanilla cape texture: U=1, V=1, W=10, H=16.
        let cape_blit = |out: &mut [[u8; 4]],
                         cape: &CapePixels,
                         sx: u32,
                         sw: u32,
                         dx: u32,
                         dy: u32| {
            const CAPE_SY: u32 = 1;
            const CAPE_H: u32 = 16;
            for j in 0..CAPE_H {
                for i in 0..sw {
                    let px = cape.px(sx + i, CAPE_SY + j);
                    put(out, dx + i, dy + j, px);
                }
            }
        };

        // (sx, sy, sw, sh, dx, dy, mirror)
        let base: &[(u32, u32, u32, u32, u32, u32, bool)] = match view {
            SkinView::Front => &[
                (8, 8, 8, 8, 4, 0, false),       // head
                (20, 20, 8, 12, 4, 8, false),    // body
                (44, 20, 4, 12, 0, 8, false),    // right arm
                (36, 52, 4, 12, 12, 8, false),   // left arm
                (4, 20, 4, 12, 4, 20, false),    // right leg
                (20, 52, 4, 12, 8, 20, false),   // left leg
            ],
            SkinView::Back => &[
                (24, 8, 8, 8, 4, 0, false),      // head back
                (32, 20, 8, 12, 4, 8, false),    // body back
                (52, 20, 4, 12, 12, 8, false),   // right arm back (now on viewer right)
                (44, 52, 4, 12, 0, 8, false),    // left arm back
                (12, 20, 4, 12, 8, 20, false),   // right leg back
                (28, 52, 4, 12, 4, 20, false),   // left leg back
            ],
            SkinView::Right => &[
                (0, 8, 8, 8, 4, 0, false),       // head right side
                (16, 20, 4, 12, 6, 8, false),    // body right side
                (40, 20, 4, 12, 6, 8, false),    // right arm right side (overlaps body)
                (0, 20, 4, 12, 6, 20, false),    // right leg right side
            ],
            SkinView::Left => &[
                (16, 8, 8, 8, 4, 0, false),      // head left side
                (28, 20, 4, 12, 6, 8, false),    // body left side
                (40, 52, 4, 12, 6, 8, false),    // left arm left side
                (16, 52, 4, 12, 6, 20, false),   // left leg left side
            ],
        };
        let overlay: &[(u32, u32, u32, u32, u32, u32, bool)] = match view {
            SkinView::Front => &[
                (40, 8, 8, 8, 4, 0, false),
                (20, 36, 8, 12, 4, 8, false),
                (44, 36, 4, 12, 0, 8, false),
                (52, 52, 4, 12, 12, 8, false),
                (4, 36, 4, 12, 4, 20, false),
                (4, 52, 4, 12, 8, 20, false),
            ],
            SkinView::Back => &[
                (56, 8, 8, 8, 4, 0, false),
                (32, 36, 8, 12, 4, 8, false),
                (52, 36, 4, 12, 12, 8, false),
                (60, 52, 4, 12, 0, 8, false),
                (12, 36, 4, 12, 8, 20, false),
                (12, 52, 4, 12, 4, 20, false),
            ],
            SkinView::Right => &[
                (32, 8, 8, 8, 4, 0, false),
                (16, 36, 4, 12, 6, 8, false),
                (40, 36, 4, 12, 6, 8, false),
                (0, 36, 4, 12, 6, 20, false),
            ],
            SkinView::Left => &[
                (48, 8, 8, 8, 4, 0, false),
                (28, 36, 4, 12, 6, 8, false),
                (56, 52, 4, 12, 6, 8, false),
                (0, 52, 4, 12, 6, 20, false),
            ],
        };
        // Cape drawn behind the body for non-back views — for the side views
        // a 1-col strip peeks out at the player's back edge; for Front it's
        // entirely covered but we still draw the sliver so arms occlude it
        // correctly if a custom skin leaves gaps.
        if let Some(cape) = cape {
            match view {
                SkinView::Front => {
                    cape_blit(&mut out, cape, 1, 10, 3, 8);
                }
                SkinView::Right => {
                    // In MC's body right-surface texture, screen-left of the
                    // rendered body is the player's back. Cape strip sits at
                    // dx=5, just left of the body (dx=6..10).
                    cape_blit(&mut out, cape, 1, 1, 5, 8);
                }
                SkinView::Left => {
                    // Mirror of Right: screen-right of the rendered body is
                    // the player's back. Cape strip at dx=10.
                    cape_blit(&mut out, cape, 10, 1, 10, 8);
                }
                SkinView::Back => {}
            }
        }
        for &(sx, sy, sw, sh, dx, dy, mir) in base {
            blit(&mut out, sx, sy, sw, sh, dx, dy, mir);
        }
        for &(sx, sy, sw, sh, dx, dy, mir) in overlay {
            blit(&mut out, sx, sy, sw, sh, dx, dy, mir);
        }
        // Back view: cape covers the body, drawn on top.
        if let (Some(cape), SkinView::Back) = (cape, view) {
            cape_blit(&mut out, cape, 1, 10, 3, 8);
        }
        out
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

pub async fn fetch_cape(client: &reqwest::Client, url: &str) -> Result<CapePixels> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    decode_cape(&bytes)
}

fn decode_cape(bytes: &[u8]) -> Result<CapePixels> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().context("cape png read_info")?;
    let mut raw = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut raw).context("cape png next_frame")?;
    let w = info.width;
    let h = info.height;
    let stride = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        other => anyhow::bail!("unsupported cape color type: {other:?}"),
    };
    let mut pixels = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) as usize) * stride;
            let r = raw[i];
            let g = raw[i + 1];
            let b = raw[i + 2];
            let a = if stride == 4 { raw[i + 3] } else { 255 };
            pixels.push([r, g, b, a]);
        }
    }
    Ok(CapePixels { pixels, w, h })
}

#[derive(Debug, Deserialize)]
struct UuidLookup {
    id: String,
}

/// Resolve a launcher-skin input string to a concrete texture URL.
/// Inputs starting with `http://`/`https://` are returned as-is. Anything else is
/// treated as a Minecraft username and looked up via Mojang's public APIs.
pub async fn resolve_skin_url(client: &reqwest::Client, input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("empty input");
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(trimmed.to_string());
    }
    if !trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || trimmed.len() > 16 {
        bail!("not a URL and not a valid Minecraft username");
    }
    let url = format!("https://api.mojang.com/users/profiles/minecraft/{trimmed}");
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("Minecraft user '{trimmed}' not found");
    }
    if !status.is_success() {
        bail!("username lookup HTTP {status}");
    }
    let lookup: UuidLookup = resp.json().await.context("username lookup body")?;
    current_skin_url(client, &lookup.id).await
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

    let mut src = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) as usize) * stride;
            let r = raw[i];
            let g = raw[i + 1];
            let b = raw[i + 2];
            let a = if stride == 4 { raw[i + 3] } else { 255 };
            src.push([r, g, b, a]);
        }
    }
    Ok(SkinPreview {
        src,
        src_w: w,
        src_h: h,
    })
}

pub struct SkinPreviewWidget<'a> {
    pub preview: &'a SkinPreview,
    pub view: SkinView,
    pub cape: Option<&'a CapePixels>,
}

impl<'a> Widget for SkinPreviewWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let pixels = self.preview.compose(self.view, self.cape);
        let px = |x: u32, y: u32| -> [u8; 4] {
            if x >= PREVIEW_W || y >= PREVIEW_H {
                return [0, 0, 0, 0];
            }
            pixels[(y * PREVIEW_W + x) as usize]
        };
        let max_cx = (PREVIEW_W as u16).min(area.width);
        let max_cy = ((PREVIEW_H / 2) as u16).min(area.height);
        for cy in 0..max_cy {
            for cx in 0..max_cx {
                let top = px(cx as u32, (cy * 2) as u32);
                let bot = px(cx as u32, (cy * 2 + 1) as u32);
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
