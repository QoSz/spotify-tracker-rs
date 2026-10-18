//! Import from the legacy Python tracker's `play_counter.db`.

use crate::db::{self, NewPlay};
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Tz;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;

/// Timestamp format used by the legacy tracker (local time, no offset).
pub const LEGACY_FORMAT: &str = "%d/%m/%Y - %H:%M:%S";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: usize,
    pub skipped: usize,
    pub invalid: usize,
}

/// Parse a legacy local timestamp in `tz` and convert it to UTC. During the
/// autumn DST overlap the earlier instant is chosen; times inside the spring
/// gap cannot be produced by a UTC->local conversion, so they are rejected.
pub fn parse_legacy(s: &str, tz: Tz) -> Result<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(s.trim(), LEGACY_FORMAT)
        .with_context(|| format!("bad legacy timestamp {s:?}"))?;
    let local = tz
        .from_local_datetime(&naive)
        .earliest()
        .ok_or_else(|| anyhow!("timestamp {s:?} does not exist in {tz}"))?;
    Ok(local.with_timezone(&Utc))
}

pub fn run(conn: &mut Connection, from: &Path, source_tz: Tz) -> Result<ImportReport> {
    let legacy = Connection::open_with_flags(from, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open legacy database {}", from.display()))?;
    let mut stmt = legacy
        .prepare("SELECT song_name, artists, album, played_at FROM tracks ORDER BY id")
        .context("legacy database has no readable `tracks` table")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;

    let mut report = ImportReport::default();
    let tx = conn.transaction()?;
    for row in rows {
        let (song_name, artists_json, album, played_at) = row?;
        let parsed = parse_legacy(&played_at, source_tz).and_then(|utc| {
            let artists: Vec<String> = serde_json::from_str(&artists_json)
                .with_context(|| format!("bad artists JSON {artists_json:?}"))?;
            Ok((utc, artists))
        });
        let (played_at, artists) = match parsed {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("skipping invalid legacy row: {e:#}");
                report.invalid += 1;
                continue;
            }
        };
        let play = NewPlay {
            played_at,
            track_id: None,
            song_name,
            album,
            duration_ms: None,
            artists,
        };
        if db::insert_play(&tx, &play)? {
            report.imported += 1;
        } else {
            report.skipped += 1;
        }
    }
    tx.commit()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::Europe::London;

    #[test]
    fn winter_time_is_utc() {
        let utc = parse_legacy("15/01/2026 - 12:00:00", London).unwrap();
        assert_eq!(db::format_utc(&utc), "2026-01-15T12:00:00Z");
    }

    #[test]
    fn summer_time_is_utc_plus_one() {
        let utc = parse_legacy("15/07/2025 - 12:34:56", London).unwrap();
        assert_eq!(db::format_utc(&utc), "2025-07-15T11:34:56Z");
    }

    #[test]
    fn autumn_overlap_picks_earlier_instant() {
        // 2025-10-26 01:30 happens twice in London; first occurrence is BST.
        let utc = parse_legacy("26/10/2025 - 01:30:00", London).unwrap();
        assert_eq!(db::format_utc(&utc), "2025-10-26T00:30:00Z");
    }

    #[test]
    fn spring_gap_and_garbage_are_rejected() {
        assert!(parse_legacy("30/03/2025 - 01:30:00", London).is_err());
        assert!(parse_legacy("2025-03-30 01:30:00", London).is_err());
    }
}
