use anyhow::Result;
use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub fn init(log_dir: &Path) -> Result<WorkerGuard> {
    let file_appender = tracing_appender::rolling::daily(log_dir, "tinux.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env("TINUX_LOG")
        .unwrap_or_else(|_| EnvFilter::new("info,tinux_launcher=debug"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(true),
        )
        .init();

    Ok(guard)
}
