//! Configuration loaded from the environment (and an optional `.env` in the working dir).

use anyhow::{Context, Result, bail};
use chrono_tz::Tz;
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// General settings. Spotify credentials live in [`SpotifyCreds`] so that
/// offline commands (stats, import) work without them.
#[derive(Debug, Clone)]
pub struct Config {
    pub timezone: Tz,
    pub max_history_size: usize,
    pub database_file: PathBuf,
    pub data_file: PathBuf,
    pub readme_file: PathBuf,
    pub sync_interval: Duration,
    pub token_cache_file: PathBuf,
    pub git_push: bool,
}

#[derive(Debug, Clone)]
pub struct SpotifyCreds {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

/// Load `.env` from the current directory only (no parent-directory search).
/// Existing environment variables take precedence.
pub fn load_dotenv() -> Result<()> {
    match dotenvy::from_path(Path::new(".env")) {
        Ok(()) => Ok(()),
        Err(e) if e.not_found() => Ok(()),
        Err(e) => Err(e).context("failed to parse .env"),
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let timezone_name = var_or("TIMEZONE", "Europe/London");
        let timezone: Tz = timezone_name
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid TIMEZONE: {timezone_name}"))?;
        let max_history = var_or("MAX_HISTORY_SIZE", "200");
        Ok(Self {
            timezone,
            max_history_size: max_history
                .parse()
                .with_context(|| format!("invalid MAX_HISTORY_SIZE: {max_history}"))?,
            database_file: var_or("DATABASE_FILE", "spotify_tracker.db").into(),
            data_file: var_or("DATA_FILE", "spotify_data.json").into(),
            readme_file: var_or("README_FILE", "README.md").into(),
            sync_interval: parse_interval(&var_or("SYNC_INTERVAL", "30m"))?,
            token_cache_file: var_or("TOKEN_CACHE_FILE", ".spotify_token.json").into(),
            git_push: parse_bool("GIT_PUSH", &var_or("GIT_PUSH", "true"))?,
        })
    }
}

impl SpotifyCreds {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            client_id: required("SPOTIPY_CLIENT_ID")?,
            client_secret: required("SPOTIPY_CLIENT_SECRET")?,
            redirect_uri: required("SPOTIPY_REDIRECT_URI")?,
        })
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn required(name: &str) -> Result<String> {
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => bail!(
            "missing required environment variable {name} (set it in .env or the environment)"
        ),
    }
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => bail!("invalid {name}: {value} (expected true or false)"),
    }
}

/// Parse an interval like `30m`, `45s` or `2h`.
pub fn parse_interval(s: &str) -> Result<Duration> {
    let s = s.trim();
    let invalid = || anyhow::anyhow!("invalid SYNC_INTERVAL: {s:?} (expected e.g. 30m, 45s, 2h)");
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: u64 = num.parse().map_err(|_| invalid())?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        _ => return Err(invalid()),
    };
    if secs == 0 {
        return Err(invalid());
    }
    Ok(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_intervals() {
        assert_eq!(parse_interval("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_interval("45s").unwrap(), Duration::from_secs(45));
        assert_eq!(parse_interval("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_interval(" 5m ").unwrap(), Duration::from_secs(300));
    }

    #[test]
    fn rejects_bad_intervals() {
        for bad in ["", "m", "30", "30x", "-5m", "0m", "1.5h", "ms"] {
            assert!(parse_interval(bad).is_err(), "{bad:?} should fail");
        }
    }

    #[test]
    fn parses_bools() {
        assert!(parse_bool("X", "TRUE").unwrap());
        assert!(!parse_bool("X", "false").unwrap());
        assert!(parse_bool("X", "maybe").is_err());
    }
}
