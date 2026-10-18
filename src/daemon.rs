//! One-shot sync and the long-running daemon loop.

use crate::config::{Config, SpotifyCreds};
use crate::{db, export, git, readme, spotify};
use anyhow::Result;
use std::time::Duration;
use tokio::signal::unix::{SignalKind, signal};
use tracing::{error, info, warn};

const BACKOFF_BASE: Duration = Duration::from_secs(30);
const BACKOFF_MAX: Duration = Duration::from_secs(300);

/// Fetch recent plays, store new ones, and refresh outputs if anything changed.
/// Returns the number of new plays.
pub async fn sync_once(cfg: &Config, creds: &SpotifyCreds) -> Result<usize> {
    let sp = spotify::authorized_client(cfg, creds).await?;
    let plays = spotify::fetch_recent(&sp).await?;
    let mut conn = db::open(&cfg.database_file)?;
    let inserted = db::insert_plays(&mut conn, &plays)?;
    info!(
        fetched = plays.len(),
        new = inserted,
        total = db::total_plays(&conn)?,
        "sync complete"
    );
    if inserted == 0 {
        return Ok(0);
    }
    export::write(&conn, cfg)?;
    readme::write(&conn, cfg)?;
    publish(cfg);
    Ok(inserted)
}

/// Git failures are logged, never propagated.
fn publish(cfg: &Config) {
    if !cfg.git_push {
        return;
    }
    if !git::is_repo() {
        info!("working directory is not a git repository; skipping git");
        return;
    }
    if let Err(e) = git::commit_and_push(&[&cfg.data_file, &cfg.readme_file]) {
        error!("{e:#}");
    }
}

pub async fn run(cfg: &Config, creds: &SpotifyCreds) -> Result<()> {
    // Registered up front so a signal during a sync is remembered and handled
    // once the in-flight sync finishes.
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    info!(interval = ?cfg.sync_interval, "daemon started");

    let mut failures = 0u32;
    loop {
        let wait = match sync_once(cfg, creds).await {
            Ok(_) => {
                failures = 0;
                cfg.sync_interval
            }
            Err(e) => {
                error!("sync failed: {e:#}");
                if let Some(d) = spotify::retry_after(&e) {
                    warn!("rate limited by Spotify; honouring Retry-After");
                    d
                } else {
                    failures += 1;
                    backoff(failures).min(cfg.sync_interval)
                }
            }
        };
        info!("next sync in {}s", wait.as_secs());
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = sigterm.recv() => { info!("SIGTERM received, exiting"); break; }
            _ = sigint.recv() => { info!("SIGINT received, exiting"); break; }
        }
    }
    Ok(())
}

/// 30s, 60s, 120s, 240s, then capped at 300s.
fn backoff(failures: u32) -> Duration {
    let factor = 2u32.saturating_pow(failures.saturating_sub(1).min(16));
    (BACKOFF_BASE * factor).min(BACKOFF_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        let secs: Vec<u64> = (1..=7).map(|n| backoff(n).as_secs()).collect();
        assert_eq!(secs, vec![30, 60, 120, 240, 300, 300, 300]);
        assert_eq!(backoff(u32::MAX).as_secs(), 300);
    }
}
