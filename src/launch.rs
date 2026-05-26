use crate::auth::Account;
use crate::download::{AssetLayout, InstallPlan};
use crate::java::JavaInstall;
use crate::paths::Paths;
use crate::version::{self, ArgEntry, ArgValue, VersionDetails};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

pub const CLASSPATH_SEP: &str = if cfg!(windows) { ";" } else { ":" };

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub min_ram_mb: u32,
    pub max_ram_mb: u32,
    pub username: String,
    pub uuid: String,
    pub access_token: String,
    pub user_type: String, // "msa" for Microsoft, "legacy" for offline
}

impl LaunchOptions {
    pub fn offline(name: impl Into<String>) -> Self {
        Self {
            min_ram_mb: 512,
            max_ram_mb: 2048,
            username: name.into(),
            uuid: "00000000-0000-0000-0000-000000000000".into(),
            access_token: "0".into(),
            user_type: "legacy".into(),
        }
    }

    pub fn from_account(acc: &Account) -> Self {
        Self {
            min_ram_mb: 512,
            max_ram_mb: 2048,
            username: acc.username.clone(),
            uuid: acc.uuid.clone(),
            access_token: acc.access_token.clone(),
            user_type: "msa".into(),
        }
    }
}

pub async fn launch(
    java: &JavaInstall,
    paths: &Paths,
    plan: &InstallPlan,
    details: &VersionDetails,
    opts: &LaunchOptions,
    log: UnboundedSender<String>,
) -> Result<i32> {
    let natives = paths.natives_dir(&plan.version_id);
    crate::download::extract_natives(&plan.natives_jars, &natives).await?;

    let game_dir = paths.instances.join(&plan.version_id);
    std::fs::create_dir_all(&game_dir)?;

    let cp = build_classpath(&plan.classpath, &plan.client_jar);

    let assets_root = match plan.asset_legacy_or_resources {
        AssetLayout::Modern => paths.assets.clone(),
        AssetLayout::Legacy | AssetLayout::PreVirtual => game_dir.join("resources"),
    };
    if matches!(
        plan.asset_legacy_or_resources,
        AssetLayout::Legacy | AssetLayout::PreVirtual
    ) {
        // For ancient versions we'd materialize assets here; we leave it as a
        // best-effort empty dir for now.
        std::fs::create_dir_all(&assets_root)?;
    }

    let mut cmd = Command::new(java.launch_path());
    cmd.current_dir(&game_dir);
    // Sever the child from our terminal: pipe stdin to nothing, and on Windows
    // detach from our console so native libs can't WriteConsole past the pipes.
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    cmd.arg(format!("-Xms{}M", opts.min_ram_mb));
    cmd.arg(format!("-Xmx{}M", opts.max_ram_mb));
    cmd.arg(format!("-Djava.library.path={}", natives.display()));
    cmd.arg("-Dminecraft.launcher.brand=revo-launcher");
    cmd.arg("-Dminecraft.launcher.version=0.1.0");

    let ctx = ArgContext {
        auth_player_name: &opts.username,
        version_name: &plan.version_id,
        game_dir: &game_dir,
        assets_root: &assets_root,
        assets_index_name: &plan.asset_index_id,
        auth_uuid: &opts.uuid,
        auth_access_token: &opts.access_token,
        clientid: "revo-launcher",
        auth_xuid: "",
        user_type: &opts.user_type,
        version_type: details.kind.as_deref().unwrap_or("release"),
        natives_directory: &natives,
        launcher_name: "revo-launcher",
        launcher_version: "0.1.0",
        classpath: &cp,
    };

    if let Some(args) = &details.arguments {
        for e in &args.jvm {
            push_arg(&mut cmd, e, &ctx);
        }
    } else {
        // pre-1.13 default JVM args
        cmd.arg("-cp").arg(&cp);
    }

    cmd.arg(&plan.main_class);

    if let Some(args) = &details.arguments {
        for e in &args.game {
            push_arg(&mut cmd, e, &ctx);
        }
    } else if let Some(legacy) = &details.minecraft_arguments {
        for tok in legacy.split_whitespace() {
            cmd.arg(substitute(tok, &ctx));
        }
    }

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let _ = log.send(format!("Launching Minecraft {}", plan.version_id));
    let mut child = cmd.spawn().context("spawning Java")?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let log_out = log.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = log_out.send(line);
        }
    });
    let log_err = log.clone();
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            let _ = log_err.send(format!("[err] {line}"));
        }
    });

    let status = child.wait().await?;
    Ok(status.code().unwrap_or(-1))
}

fn build_classpath(libs: &[PathBuf], client_jar: &Path) -> String {
    let mut parts: Vec<String> = libs.iter().map(|p| p.display().to_string()).collect();
    parts.push(client_jar.display().to_string());
    parts.join(CLASSPATH_SEP)
}

struct ArgContext<'a> {
    auth_player_name: &'a str,
    version_name: &'a str,
    game_dir: &'a Path,
    assets_root: &'a Path,
    assets_index_name: &'a str,
    auth_uuid: &'a str,
    auth_access_token: &'a str,
    clientid: &'a str,
    auth_xuid: &'a str,
    user_type: &'a str,
    version_type: &'a str,
    natives_directory: &'a Path,
    launcher_name: &'a str,
    launcher_version: &'a str,
    classpath: &'a str,
}

fn push_arg(cmd: &mut Command, entry: &ArgEntry, ctx: &ArgContext<'_>) {
    match entry {
        ArgEntry::Simple(s) => {
            cmd.arg(substitute(s, ctx));
        }
        ArgEntry::Conditional { rules, value } => {
            if !version::rules_allow(rules) {
                return;
            }
            match value {
                ArgValue::One(s) => {
                    cmd.arg(substitute(s, ctx));
                }
                ArgValue::Many(v) => {
                    for s in v {
                        cmd.arg(substitute(s, ctx));
                    }
                }
            }
        }
    }
}

fn substitute(s: &str, ctx: &ArgContext<'_>) -> String {
    let mut out = s.to_string();
    let replacements: [(&str, String); 14] = [
        ("${auth_player_name}", ctx.auth_player_name.into()),
        ("${version_name}", ctx.version_name.into()),
        ("${game_directory}", ctx.game_dir.display().to_string()),
        ("${assets_root}", ctx.assets_root.display().to_string()),
        ("${game_assets}", ctx.assets_root.display().to_string()),
        ("${assets_index_name}", ctx.assets_index_name.into()),
        ("${auth_uuid}", ctx.auth_uuid.into()),
        ("${auth_access_token}", ctx.auth_access_token.into()),
        ("${clientid}", ctx.clientid.into()),
        ("${auth_xuid}", ctx.auth_xuid.into()),
        ("${user_type}", ctx.user_type.into()),
        ("${version_type}", ctx.version_type.into()),
        ("${natives_directory}", ctx.natives_directory.display().to_string()),
        ("${classpath}", ctx.classpath.into()),
    ];
    for (k, v) in replacements {
        out = out.replace(k, &v);
    }
    out = out.replace("${launcher_name}", ctx.launcher_name);
    out = out.replace("${launcher_version}", ctx.launcher_version);
    // Legacy session token used by old versions: same as access token here.
    out = out.replace("${auth_session}", ctx.auth_access_token);
    // user_properties was removed in 1.8+, default to {}
    out = out.replace("${user_properties}", "{}");
    out
}
