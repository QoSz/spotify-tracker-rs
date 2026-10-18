//! Runs the real binary: `import` from a 3-row legacy DB into a fresh DB.

use rusqlite::Connection;
use std::path::Path;
use std::process::Command;

fn build_legacy(path: &Path) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(
        "CREATE TABLE play_count (id INTEGER PRIMARY KEY, count INTEGER);
         CREATE TABLE tracks (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             song_name TEXT NOT NULL, artists TEXT NOT NULL, album TEXT NOT NULL,
             played_at TEXT NOT NULL UNIQUE, played_at_date TEXT NOT NULL,
             played_at_hour INTEGER NOT NULL, played_at_weekday TEXT NOT NULL,
             created_at TEXT DEFAULT CURRENT_TIMESTAMP);
         INSERT INTO tracks (song_name, artists, album, played_at, played_at_date, played_at_hour, played_at_weekday) VALUES
           ('Mwaki', '[\"Zerb\", \"Sofiya Nzau\"]', 'SURRENDER', '04/03/2025 - 10:14:27', '2025-03-04', 10, 'Tuesday'),
           ('Fountains (with Tems)', '[\"Drake\", \"Tems\"]', 'Certified Lover Boy', '15/07/2025 - 12:00:00', '2025-07-15', 12, 'Tuesday'),
           ('No Introduction', '[\"Central Cee\"]', 'CAN''T RUSH GREATNESS', '17/07/2026 - 20:19:11', '2026-07-17', 20, 'Friday');",
    )
    .unwrap();
}

fn import(dir: &Path, legacy: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_spotify-tracker"))
        .current_dir(dir)
        .env("DATABASE_FILE", dir.join("new.db"))
        .env_remove("TIMEZONE")
        .arg("import")
        .arg("--from")
        .arg(legacy)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn import_is_correct_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join("play_counter.db");
    build_legacy(&legacy);

    let first = import(dir.path(), &legacy);
    assert!(first.contains("imported 3, skipped 0"), "{first}");
    let second = import(dir.path(), &legacy);
    assert!(second.contains("imported 0, skipped 3"), "{second}");

    let conn = Connection::open(dir.path().join("new.db")).unwrap();
    let rows: Vec<(String, String, Option<i64>, Option<String>)> = conn
        .prepare("SELECT played_at_utc, song_name, duration_ms, track_id FROM plays ORDER BY played_at_utc")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].0, "2025-03-04T10:14:27Z"); // GMT
    assert_eq!(rows[1].0, "2025-07-15T11:00:00Z"); // BST
    assert_eq!(rows[2].0, "2026-07-17T19:19:11Z"); // BST
    assert!(rows.iter().all(|r| r.2.is_none() && r.3.is_none()));

    let artists: Vec<String> = conn
        .prepare(
            "SELECT a.artist FROM play_artists a JOIN plays p ON p.id = a.play_id
             WHERE p.song_name = 'Mwaki' ORDER BY a.position",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(artists, vec!["Zerb", "Sofiya Nzau"]);
}
