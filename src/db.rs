//! SQLite storage. Plays are stored by UTC instant; all local-time values are
//! derived at query time (see `stats.rs`), so changing TIMEZONE is lossless.

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, Transaction, params};
use std::path::Path;
use std::time::Duration;

const SCHEMA_VERSION: i64 = 1;

// `played_at_utc` is UNIQUE, which already gives it an index.
const SCHEMA_V1: &str = "
CREATE TABLE plays (
    id            INTEGER PRIMARY KEY,
    played_at_utc TEXT NOT NULL UNIQUE,
    track_id      TEXT NULL,
    song_name     TEXT NOT NULL,
    album         TEXT NOT NULL,
    duration_ms   INTEGER NULL,
    created_at    TEXT DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE play_artists (
    play_id  INTEGER NOT NULL REFERENCES plays(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    artist   TEXT NOT NULL,
    PRIMARY KEY (play_id, position)
);
CREATE INDEX idx_play_artists_artist ON play_artists(artist);
CREATE INDEX idx_plays_song_name ON plays(song_name);
CREATE INDEX idx_plays_album ON plays(album);
";

/// A play to be inserted.
#[derive(Debug, Clone)]
pub struct NewPlay {
    pub played_at: DateTime<Utc>,
    pub track_id: Option<String>,
    pub song_name: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub artists: Vec<String>,
}

/// A play read back from the database.
#[derive(Debug, Clone)]
pub struct StoredPlay {
    pub played_at: DateTime<Utc>,
    pub song_name: String,
    pub album: String,
    pub artists: Vec<String>,
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("failed to open database {}", path.display()))?;
    init(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<Connection> {
    init(Connection::open_in_memory()?)
}

fn init(mut conn: Connection) -> Result<Connection> {
    conn.busy_timeout(Duration::from_secs(10))?;
    conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON;")?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute_batch("CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);")?;
    let current: i64 = tx.query_row(
        "SELECT IFNULL(MAX(version), 0) FROM schema_version",
        [],
        |r| r.get(0),
    )?;
    if current < 1 {
        tx.execute_batch(SCHEMA_V1)?;
    }
    if current < SCHEMA_VERSION {
        tx.execute("DELETE FROM schema_version", [])?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [SCHEMA_VERSION],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Canonical storage format: RFC3339 UTC, truncated to whole seconds. Second
/// precision matches the legacy data so imported and fetched plays dedupe.
pub fn format_utc(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn parse_utc(s: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)
        .with_context(|| format!("bad played_at_utc in database: {s}"))?
        .with_timezone(&Utc))
}

/// Insert a play unless one already exists at the same instant. Returns true if inserted.
pub fn insert_play(tx: &Transaction, play: &NewPlay) -> Result<bool> {
    let mut insert = tx.prepare_cached(
        "INSERT OR IGNORE INTO plays (played_at_utc, track_id, song_name, album, duration_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?;
    let changed = insert.execute(params![
        format_utc(&play.played_at),
        play.track_id,
        play.song_name,
        play.album,
        play.duration_ms
    ])?;
    if changed == 0 {
        return Ok(false);
    }
    let play_id = tx.last_insert_rowid();
    let mut artist = tx.prepare_cached(
        "INSERT INTO play_artists (play_id, position, artist) VALUES (?1, ?2, ?3)",
    )?;
    for (position, name) in play.artists.iter().enumerate() {
        artist.execute(params![play_id, position as i64, name])?;
    }
    Ok(true)
}

/// Insert a batch of plays in a single transaction. Returns the number inserted.
pub fn insert_plays(conn: &mut Connection, plays: &[NewPlay]) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut inserted = 0;
    for play in plays {
        if insert_play(&tx, play)? {
            inserted += 1;
        }
    }
    tx.commit()?;
    Ok(inserted)
}

pub fn total_plays(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM plays", [], |r| r.get(0))?)
}

/// The newest `limit` plays, newest first.
pub fn recent_plays(conn: &Connection, limit: usize) -> Result<Vec<StoredPlay>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, played_at_utc, song_name, album FROM plays
         ORDER BY played_at_utc DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get(2)?,
            r.get(3)?,
        ))
    })?;
    let mut plays = Vec::new();
    for row in rows {
        let (id, played_at, song_name, album) = row?;
        plays.push(StoredPlay {
            played_at: parse_utc(&played_at)?,
            song_name,
            album,
            artists: artists_for(conn, id)?,
        });
    }
    Ok(plays)
}

fn artists_for(conn: &Connection, play_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare_cached("SELECT artist FROM play_artists WHERE play_id = ?1 ORDER BY position")?;
    let artists = stmt
        .query_map([play_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(artists)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn play(secs: i64, millis: u32) -> NewPlay {
        NewPlay {
            played_at: Utc.timestamp_opt(secs, millis * 1_000_000).unwrap(),
            track_id: Some("abc".into()),
            song_name: "Song".into(),
            album: "Album".into(),
            duration_ms: Some(200_000),
            artists: vec!["A".into(), "B".into()],
        }
    }

    #[test]
    fn dedupes_on_second_precision_and_keeps_artist_order() {
        let mut conn = open_in_memory().unwrap();
        let n = insert_plays(
            &mut conn,
            &[play(1_700_000_000, 123), play(1_700_000_000, 999)],
        )
        .unwrap();
        assert_eq!(n, 1);
        assert_eq!(total_plays(&conn).unwrap(), 1);
        let recent = recent_plays(&conn, 10).unwrap();
        assert_eq!(recent[0].artists, vec!["A", "B"]);
        assert_eq!(format_utc(&recent[0].played_at), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn migration_is_idempotent() {
        let mut conn = open_in_memory().unwrap();
        migrate(&mut conn).unwrap();
        let v: i64 = conn
            .query_row("SELECT version FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }
}
