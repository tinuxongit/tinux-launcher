//! Microsoft OAuth -> Xbox Live -> XSTS -> Minecraft Services token chain.
//!
//! Requires a Microsoft Entra (Azure AD) application registration with personal-account
//! support and a redirect URI of `http://localhost:<port>/callback`. Set the client id
//! via the `REVO_MS_CLIENT_ID` environment variable.

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

// We bind to 127.0.0.1 to listen for the callback (a real IP, since tokio's TcpListener
// won't resolve names) but advertise "localhost" in the redirect URI. Azure treats
// http://localhost specially for public clients — it accepts any port, so we don't
// have to hard-code one in the app registration.
const BIND_HOST: &str = "127.0.0.1";
const REDIRECT_HOST: &str = "localhost";
const AUTH_BASE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const XBOX_AUTH: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const SCOPES: &str = "XboxLive.signin offline_access";
const KEYRING_SERVICE: &str = "RevoLauncher";
const KEYRING_USER: &str = "ms-refresh";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

/// Paste your Azure app's Application (client) ID here and recompile to ship Revo with
/// sign-in working out of the box for every user — this is exactly how Modrinth App,
/// PrismLauncher, ATLauncher, etc. distribute their launchers. The id is **not** a
/// secret: it's the launcher's public identity, paired with PKCE for safety. Users
/// still sign in with their own Microsoft accounts; this id just tells Microsoft
/// which app is asking.
///
/// Leave as `None` and Revo falls back to env var → config.json (developer mode).
const BAKED_CLIENT_ID: Option<&str> = Some("164bca05-6a7e-4142-990e-540a7aae3f18");

pub fn client_id() -> Option<String> {
    if let Ok(v) = std::env::var("REVO_MS_CLIENT_ID") {
        if !v.trim().is_empty() {
            return Some(v);
        }
    }
    if let Some(s) = crate::config::path()
        .and_then(|p| crate::config::Config::load(&p).ms_client_id)
        .filter(|s| !s.trim().is_empty())
    {
        return Some(s);
    }
    BAKED_CLIENT_ID.map(|s| s.to_string())
}

pub async fn interactive_login() -> Result<Account> {
    let client_id = client_id().ok_or_else(|| {
        anyhow!(
            "Microsoft client id missing. Paste your Azure app's Application (client) ID \
             into config.json (see the Accounts tab for the path) or set REVO_MS_CLIENT_ID."
        )
    })?;

    let listener = TcpListener::bind((BIND_HOST, 0u16)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://{REDIRECT_HOST}:{port}/callback");

    let (verifier, challenge) = pkce_pair();
    let state = random_state();

    let mut auth_url = Url::parse(&format!("{AUTH_BASE}/authorize"))?;
    auth_url
        .query_pairs_mut()
        .append_pair("client_id", &client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("response_mode", "query")
        .append_pair("scope", SCOPES)
        .append_pair("state", &state)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256")
        // Always show the account picker — otherwise Microsoft silently re-uses the
        // browser's existing session, which traps users on whichever MS account they
        // signed in with first (often not the one that owns Minecraft).
        .append_pair("prompt", "select_account");

    webbrowser::open(auth_url.as_str()).context("opening browser for sign-in")?;

    let code = wait_for_redirect(listener, &state).await?;

    let http = reqwest::Client::builder()
        .user_agent("revo-launcher/0.1")
        .build()?;

    let ms = exchange_code(&http, &client_id, &code, &redirect_uri, &verifier).await?;
    let account = ms_to_account(&http, &ms).await?;

    if let Some(rt) = &account.refresh_token {
        let _ = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|e| e.set_password(rt));
    }

    Ok(account)
}

#[allow(dead_code)]
pub async fn try_silent_login() -> Result<Account> {
    let client_id = client_id().context("REVO_MS_CLIENT_ID not set")?;
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    let refresh = entry.get_password().context("no stored refresh token")?;

    let http = reqwest::Client::builder()
        .user_agent("revo-launcher/0.1")
        .build()?;

    let ms = refresh_ms(&http, &client_id, &refresh).await?;
    let account = ms_to_account(&http, &ms).await?;

    if let Some(rt) = &account.refresh_token {
        let _ = entry.set_password(rt);
    }

    Ok(account)
}

pub fn logout() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
}

fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let digest = sha2_impl::sha256(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

mod sha2_impl {
    // Minimal SHA-256 implementation for PKCE S256.
    // Source: public-domain reference (FIPS 180-4).
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    const H0: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    pub fn sha256(msg: &[u8]) -> [u8; 32] {
        let bit_len = (msg.len() as u64) * 8;
        let mut padded = msg.to_vec();
        padded.push(0x80);
        while padded.len() % 64 != 56 {
            padded.push(0);
        }
        padded.extend_from_slice(&bit_len.to_be_bytes());

        let mut h = H0;
        for chunk in padded.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
                (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ (!e & g);
                let t1 = hh
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                hh = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            h[0] = h[0].wrapping_add(a);
            h[1] = h[1].wrapping_add(b);
            h[2] = h[2].wrapping_add(c);
            h[3] = h[3].wrapping_add(d);
            h[4] = h[4].wrapping_add(e);
            h[5] = h[5].wrapping_add(f);
            h[6] = h[6].wrapping_add(g);
            h[7] = h[7].wrapping_add(hh);
        }
        let mut out = [0u8; 32];
        for (i, w) in h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

async fn wait_for_redirect(listener: TcpListener, expected_state: &str) -> Result<String> {
    let (mut stream, _peer) = listener.accept().await?;
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");
    let full = format!("http://localhost{path}");
    let parsed = Url::parse(&full)?;
    let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();

    let body = if let Some(err) = pairs.get("error") {
        format!("Sign-in failed: {err}")
    } else {
        "Sign-in complete. You can close this tab.".to_string()
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.shutdown().await;

    let state = pairs.get("state").map(|s| s.as_str()).unwrap_or("");
    if state != expected_state {
        bail!("OAuth state mismatch");
    }
    let code = pairs.get("code").ok_or_else(|| anyhow!("no code in callback"))?;
    Ok(code.clone())
}

#[derive(Deserialize)]
struct MsToken {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

async fn exchange_code(
    http: &reqwest::Client,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<MsToken> {
    let res = http
        .post(format!("{AUTH_BASE}/token"))
        .form(&[
            ("client_id", client_id),
            ("scope", SCOPES),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier),
        ])
        .send()
        .await?
        .error_for_status()
        .context("exchanging OAuth code")?
        .json::<MsToken>()
        .await?;
    Ok(res)
}

#[allow(dead_code)]
async fn refresh_ms(http: &reqwest::Client, client_id: &str, refresh: &str) -> Result<MsToken> {
    let res = http
        .post(format!("{AUTH_BASE}/token"))
        .form(&[
            ("client_id", client_id),
            ("scope", SCOPES),
            ("refresh_token", refresh),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await?
        .error_for_status()
        .context("refreshing MS token")?
        .json::<MsToken>()
        .await?;
    Ok(res)
}

async fn ms_to_account(http: &reqwest::Client, ms: &MsToken) -> Result<Account> {
    let xbl = xbl_authenticate(http, &ms.access_token).await?;
    let xsts = xsts_authorize(http, &xbl.token).await?;
    let uhs = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| anyhow!("missing uhs"))?;
    let mc_token = mc_login(http, &uhs, &xsts.token).await?;
    let profile = mc_profile(http, &mc_token).await?;
    Ok(Account {
        username: profile.name,
        uuid: profile.id,
        access_token: mc_token,
        refresh_token: ms.refresh_token.clone(),
    })
}

#[derive(Deserialize)]
struct XblResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Deserialize)]
struct DisplayClaims {
    xui: Vec<XuiClaim>,
}

#[derive(Deserialize)]
struct XuiClaim {
    uhs: String,
}

async fn xbl_authenticate(http: &reqwest::Client, ms_token: &str) -> Result<XblResponse> {
    let body = serde_json::json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_token}")
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });
    let r = http
        .post(XBOX_AUTH)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?
        .error_for_status()
        .context("Xbox Live auth")?
        .json::<XblResponse>()
        .await?;
    Ok(r)
}

#[derive(Deserialize)]
struct XstsResponse {
    #[serde(rename = "Token")]
    token: String,
}

async fn xsts_authorize(http: &reqwest::Client, xbl_token: &str) -> Result<XstsResponse> {
    let body = serde_json::json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl_token]
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    });
    let resp = http
        .post(XSTS_AUTH)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let s = resp.status();
        let text = resp.text().await.unwrap_or_default();
        bail!("XSTS authorize failed ({s}): {text}");
    }
    Ok(resp.json::<XstsResponse>().await?)
}

#[derive(Deserialize)]
struct McLoginResp {
    access_token: String,
}

async fn mc_login(http: &reqwest::Client, uhs: &str, xsts_token: &str) -> Result<String> {
    let body = serde_json::json!({
        "identityToken": format!("XBL3.0 x={uhs};{xsts_token}")
    });
    let resp = http
        .post(MC_LOGIN)
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let hint = if status.as_u16() == 403 {
            "\n\nThis launcher's Azure App ID isn't approved by Mojang yet. Microsoft now \
             requires third-party launchers to whitelist their client id. Submit it at \
             https://aka.ms/mce-reviewappid (needs the Client ID + Tenant ID from the \
             Azure portal) and allow up to 24h after approval. Offline mode still works \
             in the meantime."
        } else {
            ""
        };
        bail!("Minecraft login HTTP {status}: {body}{hint}");
    }
    let r: McLoginResp = resp.json().await?;
    Ok(r.access_token)
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

async fn mc_profile(http: &reqwest::Client, mc_token: &str) -> Result<McProfile> {
    let r = http
        .get(MC_PROFILE)
        .bearer_auth(mc_token)
        .send()
        .await?
        .error_for_status()
        .context("fetching MC profile (does this account own Minecraft?)")?
        .json::<McProfile>()
        .await?;
    Ok(r)
}
