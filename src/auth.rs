use crate::event::WorkerMsg;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

const AUTH_BASE: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0";
const XBOX_AUTH: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE: &str = "https://api.minecraftservices.com/minecraft/profile";
const SCOPES: &str = "XboxLive.signin offline_access";
const KEYRING_SERVICE: &str = "TinuxLauncher";
const KEYRING_USER: &str = "ms-refresh";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
}

// Our Azure client ID, registered to tinuxongit and approved for
// Minecraft/Xbox authentication (see https://aka.ms/AppRegInfo).
const BAKED_CLIENT_ID: Option<&str> = Some("164bca05-6a7e-4142-990e-540a7aae3f18"); // tinux-launcher

pub fn client_id() -> Option<String> {
    if let Ok(v) = std::env::var("TINUX_MS_CLIENT_ID") {
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

/// Run the OAuth 2.0 Device Code Flow against Microsoft's consumer endpoint.
///
/// Emits a `WorkerMsg::AuthDeviceCode` containing the user_code + verification
/// URI as soon as Microsoft hands them to us, then polls /token until the
/// user completes the sign-in (or the code expires / they cancel).
pub async fn interactive_login(tx: UnboundedSender<WorkerMsg>) -> Result<Account> {
    let client_id = client_id().ok_or_else(|| anyhow!("no client id configured"))?;
    let http = reqwest::Client::builder()
        .user_agent("tinux-launcher/0.1")
        .build()?;

    let device = request_device_code(&http, &client_id).await?;

    // Tell the UI to show the user code + open the verification URL in a browser.
    let _ = tx.send(WorkerMsg::AuthDeviceCode {
        user_code: device.user_code.clone(),
        verification_uri: device.verification_uri.clone(),
        expires_in: device.expires_in,
    });
    // Best-effort: also pop the browser so the user doesn't have to copy the URL.
    let _ = webbrowser::open(&device.verification_uri);

    let ms = poll_for_token(&http, &client_id, &device).await?;
    let account = ms_to_account(&http, &ms).await?;

    if let Some(rt) = &account.refresh_token {
        let _ = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .and_then(|e| e.set_password(rt));
    }

    Ok(account)
}

pub async fn set_skin_url(
    http: &reqwest::Client,
    token: &str,
    model: &str,
    url: &str,
) -> Result<()> {
    let resp = http
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(token)
        .json(&serde_json::json!({ "variant": model, "url": url }))
        .send()
        .await?;
    if !resp.status().is_success() {
        let s = resp.status();
        let t = resp.text().await.unwrap_or_default();
        bail!("set skin HTTP {s}: {t}");
    }
    Ok(())
}

/// Upload a local PNG file as the active Minecraft skin via Mojang's
/// multipart endpoint.
pub async fn upload_skin_file(
    http: &reqwest::Client,
    token: &str,
    model: &str,
    file_bytes: Vec<u8>,
    filename: &str,
) -> Result<()> {
    let part = reqwest::multipart::Part::bytes(file_bytes)
        .file_name(filename.to_string())
        .mime_str("image/png")
        .context("setting mime for skin upload")?;
    let form = reqwest::multipart::Form::new()
        .text("variant", model.to_string())
        .part("file", part);
    let resp = http
        .post("https://api.minecraftservices.com/minecraft/profile/skins")
        .bearer_auth(token)
        .multipart(form)
        .send()
        .await?;
    if !resp.status().is_success() {
        let s = resp.status();
        let t = resp.text().await.unwrap_or_default();
        bail!("upload skin HTTP {s}: {t}");
    }
    Ok(())
}

pub async fn reset_skin(http: &reqwest::Client, token: &str) -> Result<()> {
    let resp = http
        .delete("https://api.minecraftservices.com/minecraft/profile/skins/active")
        .bearer_auth(token)
        .send()
        .await?;
    if !resp.status().is_success() {
        let s = resp.status();
        let t = resp.text().await.unwrap_or_default();
        bail!("reset skin HTTP {s}: {t}");
    }
    Ok(())
}

pub fn logout() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
}

/// Try to silently restore a previously-signed-in session by trading the
/// refresh token in the keyring for a fresh access token, then re-running
/// the Xbox + Mojang chain. Returns Ok(account) on success, Err otherwise
/// (no saved token, token expired, network failure, etc.). Callers should
/// just discard the error and leave the user in offline mode.
pub async fn try_refresh_session() -> Result<Account> {
    let client_id = client_id().ok_or_else(|| anyhow!("no client id configured"))?;
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .context("opening keyring entry")?;
    let refresh_token = entry.get_password().context("no saved session")?;
    if refresh_token.trim().is_empty() {
        bail!("saved refresh token is empty");
    }

    let http = reqwest::Client::builder()
        .user_agent("tinux-launcher/0.1")
        .build()?;
    let resp = http
        .post(format!("{AUTH_BASE}/token"))
        .form(&[
            ("client_id", client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("scope", SCOPES),
        ])
        .send()
        .await
        .context("refreshing MS token")?;
    if !resp.status().is_success() {
        // Token is dead — drop it so we don't keep trying every startup.
        let _ = entry.delete_credential();
        let s = resp.status();
        let t = resp.text().await.unwrap_or_default();
        bail!("refresh failed HTTP {s}: {t}");
    }
    let ms: MsToken = resp.json().await?;
    let account = ms_to_account(&http, &ms).await?;
    // Microsoft rotates refresh tokens, so persist the new one if we got it.
    if let Some(rt) = &account.refresh_token {
        let _ = entry.set_password(rt);
    }
    Ok(account)
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

async fn request_device_code(
    http: &reqwest::Client,
    client_id: &str,
) -> Result<DeviceCodeResponse> {
    let resp = http
        .post(format!("{AUTH_BASE}/devicecode"))
        .form(&[("client_id", client_id), ("scope", SCOPES)])
        .send()
        .await
        .context("requesting device code")?;
    if !resp.status().is_success() {
        let s = resp.status();
        let t = resp.text().await.unwrap_or_default();
        bail!("device code request HTTP {s}: {t}");
    }
    Ok(resp.json::<DeviceCodeResponse>().await?)
}

#[derive(Deserialize)]
struct MsToken {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct TokenError {
    error: String,
    #[serde(default)]
    error_description: String,
}

async fn poll_for_token(
    http: &reqwest::Client,
    client_id: &str,
    device: &DeviceCodeResponse,
) -> Result<MsToken> {
    let started = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(device.expires_in.min(900));
    let mut interval = std::time::Duration::from_secs(device.interval.max(1));
    loop {
        if started.elapsed() >= timeout {
            bail!("Sign-in timed out — you didn't finish in time");
        }
        tokio::time::sleep(interval).await;

        let resp = http
            .post(format!("{AUTH_BASE}/token"))
            .form(&[
                ("client_id", client_id),
                ("grant_type", DEVICE_GRANT),
                ("device_code", device.device_code.as_str()),
            ])
            .send()
            .await
            .context("polling /token")?;
        let status = resp.status();
        if status.is_success() {
            return Ok(resp.json::<MsToken>().await?);
        }
        // 400 / 401 with a JSON error body is the normal "not yet" path.
        let body_text = resp.text().await.unwrap_or_default();
        match serde_json::from_str::<TokenError>(&body_text) {
            Ok(err) => match err.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += std::time::Duration::from_secs(5);
                    continue;
                }
                "expired_token" => bail!("Sign-in code expired — try again"),
                "access_denied" => bail!("Sign-in was cancelled"),
                other => bail!("Sign-in failed ({other}): {}", err.error_description),
            },
            Err(_) => bail!("Sign-in failed ({status}): {body_text}"),
        }
    }
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
        if status == reqwest::StatusCode::FORBIDDEN && body.contains("Invalid app registration") {
            bail!(
                "Minecraft rejected this Microsoft app registration. The Azure client ID must be approved for Minecraft/Xbox authentication; see https://aka.ms/AppRegInfo."
            );
        }
        bail!("Minecraft login HTTP {status}: {body}");
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
