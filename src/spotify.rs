//! Spotify Web API access via rspotify (authorization-code flow, cached token).

use crate::config::{Config, SpotifyCreds};
use crate::db::NewPlay;
use anyhow::{Context, Result, anyhow, bail};
use rspotify::http::HttpError;
use rspotify::prelude::*;
use rspotify::{AuthCodeSpotify, ClientError, Credentials, OAuth, Token, scopes};
use std::io::{self, BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const FETCH_LIMIT: u32 = 50;

fn client(cfg: &Config, creds: &SpotifyCreds) -> AuthCodeSpotify {
    let oauth = OAuth {
        redirect_uri: creds.redirect_uri.clone(),
        scopes: scopes!("user-read-recently-played"),
        ..Default::default()
    };
    let config = rspotify::Config {
        cache_path: cfg.token_cache_file.clone(),
        token_cached: true,
        token_refreshing: true,
        ..Default::default()
    };
    AuthCodeSpotify::with_config(
        Credentials::new(&creds.client_id, &creds.client_secret),
        oauth,
        config,
    )
}

/// Interactive, headless-friendly login: print the auth URL, read the
/// redirect URL from stdin, exchange the code and cache the token.
pub async fn login(cfg: &Config, creds: &SpotifyCreds) -> Result<()> {
    let sp = client(cfg, creds);
    let url = sp.get_authorize_url(false)?;
    println!("1. Open this URL in any browser and approve access:\n\n{url}\n");
    println!(
        "2. You will be redirected to {} (the page may fail to load; that's fine).",
        creds.redirect_uri
    );
    print!("3. Paste the full URL from the address bar here: ");
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin().lock().read_line(&mut line)?;
    let code = sp
        .parse_response_code(line.trim())
        .ok_or_else(|| anyhow!("no valid `code` (or mismatched `state`) in the pasted URL"))?;
    sp.request_token(&code)
        .await
        .context("failed to exchange authorization code")?;
    std::fs::set_permissions(
        &cfg.token_cache_file,
        std::fs::Permissions::from_mode(0o600),
    )
    .with_context(|| format!("failed to chmod {}", cfg.token_cache_file.display()))?;
    println!("Token saved to {}", cfg.token_cache_file.display());
    Ok(())
}

/// Build a client from the cached token. Expired tokens are refreshed
/// automatically by rspotify on the first request (and re-cached).
pub async fn authorized_client(cfg: &Config, creds: &SpotifyCreds) -> Result<AuthCodeSpotify> {
    if !cfg.token_cache_file.exists() {
        bail!(
            "no token cache at {}; run `spotify-tracker login` first",
            cfg.token_cache_file.display()
        );
    }
    let sp = client(cfg, creds);
    let token: Token = sp
        .read_token_cache(true)
        .await
        .context("failed to read token cache")?
        .ok_or_else(|| anyhow!("cached token lacks required scope; run `spotify-tracker login`"))?;
    *sp.get_token().lock().await.unwrap() = Some(token);
    Ok(sp)
}

pub async fn fetch_recent(sp: &AuthCodeSpotify) -> Result<Vec<NewPlay>> {
    let page = sp
        .current_user_recently_played(Some(FETCH_LIMIT), None)
        .await
        .context("failed to fetch recently played tracks")?;
    Ok(page
        .items
        .into_iter()
        .map(|h| NewPlay {
            played_at: h.played_at,
            track_id: h.track.id.map(|id| id.id().to_string()),
            song_name: h.track.name,
            album: h.track.album.name,
            duration_ms: Some(h.track.duration.num_milliseconds()),
            artists: h.track.artists.into_iter().map(|a| a.name).collect(),
        })
        .collect())
}

/// If `err` is a Spotify 429, how long to wait (from `Retry-After`, default 60s).
pub fn retry_after(err: &anyhow::Error) -> Option<Duration> {
    let ClientError::Http(http) = err.downcast_ref::<ClientError>()? else {
        return None;
    };
    let HttpError::StatusCode(resp) = http.as_ref() else {
        return None;
    };
    if resp.status().as_u16() != 429 {
        return None;
    }
    let secs = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(60);
    Some(Duration::from_secs(secs.max(1)))
}
