//! README generator. If the README contains the START/END markers only the
//! section between them is replaced; otherwise the whole file is written.

use crate::config::Config;
use crate::stats::{self, Period, Summary};
use anyhow::{Context, Result};
use chrono::Datelike;
use rusqlite::Connection;
use std::fs;

pub const START: &str = "<!-- STATS:START -->";
pub const END: &str = "<!-- STATS:END -->";

pub fn write(conn: &Connection, cfg: &Config) -> Result<()> {
    let plays = stats::local_plays(conn, cfg.timezone)?;
    let today = stats::today(cfg.timezone);
    let summary = stats::summary(conn, &plays, today)?;
    let top = stats::top_artists(conn, 5)?;
    let months = stats::monthly(&plays, today.year());
    let block = render_stats(&summary, &top, &months);

    let existing = fs::read_to_string(&cfg.readme_file).unwrap_or_default();
    let content = splice(&existing, &block).unwrap_or_else(|| {
        format!(
            "# Spotify Tracker\n\nTracking my Spotify listening history.\n\n{START}\n{block}{END}\n"
        )
    });
    fs::write(&cfg.readme_file, content)
        .with_context(|| format!("failed to write {}", cfg.readme_file.display()))
}

/// Replace the text between the markers, or None if they're missing.
fn splice(existing: &str, block: &str) -> Option<String> {
    let start = existing.find(START)? + START.len();
    let end = start + existing[start..].find(END)?;
    Some(format!(
        "{}\n{block}{}",
        &existing[..start],
        &existing[end..]
    ))
}

pub fn render_stats(s: &Summary, top_artists: &[(String, i64)], months: &[Period]) -> String {
    let top = if top_artists.is_empty() {
        "No data yet".to_string()
    } else {
        top_artists
            .iter()
            .enumerate()
            .map(|(i, (a, c))| format!("{}. {a} ({c} plays)", i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let peak = s
        .peak_hour()
        .map_or("n/a".to_string(), |(h, _)| format!("{h:02}:00"));
    let excluded = if s.timed_plays < s.total_plays {
        format!(
            "\n- {} older plays (imported from the legacy tracker) have no duration and are excluded",
            stats::thousands(s.total_plays - s.timed_plays)
        )
    } else {
        String::new()
    };
    let by_month = if months.is_empty() {
        "No data yet".to_string()
    } else {
        let rows: Vec<String> = months
            .iter()
            .map(|m| {
                format!(
                    "| {} | {} | {:.1} |",
                    m.label,
                    stats::thousands(m.plays),
                    stats::hours(m.listening_ms)
                )
            })
            .collect();
        format!(
            "| Month | Plays | Hours |\n|---|---:|---:|\n{}",
            rows.join("\n")
        )
    };

    format!(
        "## Lifetime Stats

**{total}** songs listened to

- **{artists}** unique artists
- **{albums}** unique albums
- **{songs}** unique songs
- **{avg:.1}** songs per day (average)

## Top Artists (All Time)

{top}

## Listening Streak

- Current: **{current}** days
- Longest: **{longest}** days

## Listening Patterns

- Peak hour: **{peak}**

## Listening Time

- **{hours:.1}** hours listened ({timed} plays with known duration){excluded}

## This Year by Month

{by_month}

---
*Automatically updated by spotify-tracker (Rust)*
",
        total = stats::thousands(s.total_plays),
        artists = s.unique_artists,
        albums = s.unique_albums,
        songs = s.unique_songs,
        avg = s.daily_average,
        current = s.current_streak,
        longest = s.longest_streak,
        hours = stats::hours(s.listening_ms),
        timed = stats::thousands(s.timed_plays),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_replaces_only_between_markers() {
        let existing = format!("# Usage\nkeep me\n{START}\nold stats\n{END}\nfooter\n");
        let out = splice(&existing, "new stats\n").unwrap();
        assert_eq!(
            out,
            format!("# Usage\nkeep me\n{START}\nnew stats\n{END}\nfooter\n")
        );
        assert!(splice("no markers", "x").is_none());
    }

    #[test]
    fn writes_full_readme_with_new_sections() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config {
            timezone: chrono_tz::Europe::London,
            max_history_size: 200,
            database_file: dir.path().join("t.db"),
            data_file: dir.path().join("d.json"),
            readme_file: dir.path().join("README.md"),
            sync_interval: std::time::Duration::from_secs(60),
            token_cache_file: dir.path().join("tok.json"),
            git_push: false,
        };
        let mut conn = crate::db::open_in_memory().unwrap();
        let play = crate::db::NewPlay {
            played_at: chrono::Utc::now(),
            track_id: Some("id".into()),
            song_name: "S".into(),
            album: "A".into(),
            duration_ms: Some(5_400_000),
            artists: vec!["Drake".into()],
        };
        crate::db::insert_plays(&mut conn, &[play]).unwrap();
        write(&conn, &cfg).unwrap();
        write(&conn, &cfg).unwrap(); // second run splices between markers
        let out = fs::read_to_string(&cfg.readme_file).unwrap();
        assert_eq!(out.matches(START).count(), 1);
        for needle in [
            "# Spotify Tracker",
            "**1** songs listened to",
            "1. Drake (1 plays)",
            "- Current: **1** days",
            "## Listening Time",
            "- **1.5** hours listened (1 plays with known duration)",
            "## This Year by Month",
            "| Month | Plays | Hours |",
            "*Automatically updated by spotify-tracker (Rust)*",
        ] {
            assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
        }
    }
}
