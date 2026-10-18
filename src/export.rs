//! The rolling `DATA_FILE` JSON (same field names as the Python tracker).

use crate::config::Config;
use crate::db::{self, StoredPlay};
use crate::import::LEGACY_FORMAT;
use anyhow::{Context, Result};
use chrono_tz::Tz;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Serialize)]
struct Export<'a> {
    total_plays: i64,
    tracks: Vec<ExportTrack<'a>>,
}

#[derive(Serialize)]
struct ExportTrack<'a> {
    song_name: &'a str,
    artists: &'a [String],
    album: &'a str,
    played_at: String,
}

/// Render JSON for `plays` (already newest first) with 4-space indentation.
pub fn render(total_plays: i64, plays: &[StoredPlay], tz: Tz) -> Result<String> {
    let export = Export {
        total_plays,
        tracks: plays
            .iter()
            .map(|p| ExportTrack {
                song_name: &p.song_name,
                artists: &p.artists,
                album: &p.album,
                played_at: p
                    .played_at
                    .with_timezone(&tz)
                    .format(LEGACY_FORMAT)
                    .to_string(),
            })
            .collect(),
    };
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    export.serialize(&mut ser)?;
    buf.push(b'\n');
    Ok(String::from_utf8(buf)?)
}

pub fn write(conn: &Connection, cfg: &Config) -> Result<()> {
    let plays = db::recent_plays(conn, cfg.max_history_size)?;
    let json = render(db::total_plays(conn)?, &plays, cfg.timezone)?;
    std::fs::write(&cfg.data_file, json)
        .with_context(|| format!("failed to write {}", cfg.data_file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use chrono_tz::Europe::London;

    #[test]
    fn field_names_order_and_local_format() {
        let play = StoredPlay {
            played_at: DateTime::parse_from_rfc3339("2026-07-17T19:19:11Z")
                .unwrap()
                .with_timezone(&Utc),
            song_name: "Señor".into(),
            artists: vec!["A".into(), "B".into()],
            album: "Alb".into(),
        };
        let json = render(42, &[play], London).unwrap();
        let pos = |k: &str| json.find(&format!("\"{k}\"")).unwrap();
        assert!(pos("total_plays") < pos("tracks"));
        assert!(pos("song_name") < pos("artists"));
        assert!(pos("artists") < pos("album"));
        assert!(pos("album") < pos("played_at"));
        assert!(json.contains("\"played_at\": \"17/07/2026 - 20:19:11\""));
        assert!(json.contains("Señor"), "non-ASCII must not be escaped");
        assert!(json.contains("\n    \"total_plays\": 42"));
    }
}
