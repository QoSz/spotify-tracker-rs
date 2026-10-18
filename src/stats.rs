//! Listening statistics. Counts by artist/album/song are pure SQL; anything
//! that depends on local time is computed here from `played_at_utc` + TIMEZONE.

use crate::db;
use anyhow::Result;
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use chrono_tz::Tz;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};

pub const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

/// Each play's song name plus its artists joined in order.
const SONGS: &str = "SELECT p.song_name,
        (SELECT group_concat(artist, ', ' ORDER BY position)
           FROM play_artists WHERE play_id = p.id) AS artists
   FROM plays p";

pub struct LocalPlay {
    pub at: DateTime<Tz>,
    pub duration_ms: Option<i64>,
}

pub struct Summary {
    pub total_plays: i64,
    pub unique_artists: i64,
    pub unique_albums: i64,
    pub unique_songs: i64,
    pub daily_average: f64,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub by_hour: [i64; 24],
    pub by_weekday: [i64; 7],
    pub listening_ms: i64,
    pub timed_plays: i64,
}

impl Summary {
    /// The hour with the most plays (earliest hour wins ties).
    pub fn peak_hour(&self) -> Option<(usize, i64)> {
        let mut best: Option<(usize, i64)> = None;
        for (hour, &count) in self.by_hour.iter().enumerate() {
            if count > 0 && best.is_none_or(|(_, c)| count > c) {
                best = Some((hour, count));
            }
        }
        best
    }
}

pub struct Period {
    pub label: String,
    pub plays: i64,
    pub listening_ms: i64,
}

pub fn today(tz: Tz) -> NaiveDate {
    Utc::now().with_timezone(&tz).date_naive()
}

pub fn local_plays(conn: &Connection, tz: Tz) -> Result<Vec<LocalPlay>> {
    let mut stmt =
        conn.prepare_cached("SELECT played_at_utc, duration_ms FROM plays ORDER BY played_at_utc")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?;
    let mut plays = Vec::new();
    for row in rows {
        let (utc, duration_ms) = row?;
        plays.push(LocalPlay {
            at: db::parse_utc(&utc)?.with_timezone(&tz),
            duration_ms,
        });
    }
    Ok(plays)
}

pub fn summary(conn: &Connection, plays: &[LocalPlay], today: NaiveDate) -> Result<Summary> {
    let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    let dates: BTreeSet<NaiveDate> = plays.iter().map(|p| p.at.date_naive()).collect();
    let (current_streak, longest_streak) = streaks(&dates, today);

    let mut by_hour = [0i64; 24];
    let mut by_weekday = [0i64; 7];
    for p in plays {
        by_hour[p.at.hour() as usize] += 1;
        by_weekday[p.at.weekday().num_days_from_monday() as usize] += 1;
    }
    let (listening_ms, timed_plays) = listening(plays);

    Ok(Summary {
        total_plays: db::total_plays(conn)?,
        unique_artists: count("SELECT COUNT(DISTINCT artist) FROM play_artists")?,
        unique_albums: count("SELECT COUNT(DISTINCT album) FROM plays")?,
        unique_songs: count(&format!(
            "SELECT COUNT(*) FROM (SELECT DISTINCT song_name, artists FROM ({SONGS}))"
        ))?,
        daily_average: daily_average(&dates, plays.len()),
        current_streak,
        longest_streak,
        by_hour,
        by_weekday,
        listening_ms,
        timed_plays,
    })
}

/// Plays per day over the span from the first to the last play day, inclusive.
fn daily_average(dates: &BTreeSet<NaiveDate>, plays: usize) -> f64 {
    match (dates.first(), dates.last()) {
        (Some(first), Some(last)) => plays as f64 / ((*last - *first).num_days() + 1) as f64,
        _ => 0.0,
    }
}

/// Sum of known durations and how many plays had one (imported plays have none).
fn listening(plays: &[LocalPlay]) -> (i64, i64) {
    plays
        .iter()
        .filter_map(|p| p.duration_ms)
        .fold((0, 0), |(ms, n), d| (ms + d, n + 1))
}

/// (current, longest) runs of consecutive days with at least one play. The
/// current streak may end today or yesterday (today isn't over yet).
pub fn streaks(dates: &BTreeSet<NaiveDate>, today: NaiveDate) -> (u32, u32) {
    let mut longest = 0;
    let mut run = 0;
    let mut prev: Option<NaiveDate> = None;
    for &d in dates {
        run = match prev {
            Some(p) if p.succ_opt() == Some(d) => run + 1,
            _ => 1,
        };
        longest = longest.max(run);
        prev = Some(d);
    }

    let mut day = if dates.contains(&today) {
        Some(today)
    } else {
        today.pred_opt()
    };
    let mut current = 0;
    while let Some(d) = day.filter(|d| dates.contains(d)) {
        current += 1;
        day = d.pred_opt();
    }
    (current, longest)
}

pub fn top_artists(conn: &Connection, n: usize) -> Result<Vec<(String, i64)>> {
    ranked(
        conn,
        "SELECT artist, COUNT(*) AS c FROM play_artists
         GROUP BY artist ORDER BY c DESC, artist LIMIT ?1",
        n,
    )
}

pub fn top_albums(conn: &Connection, n: usize) -> Result<Vec<(String, i64)>> {
    ranked(
        conn,
        "SELECT album, COUNT(*) AS c FROM plays GROUP BY album ORDER BY c DESC, album LIMIT ?1",
        n,
    )
}

pub fn top_songs(conn: &Connection, n: usize) -> Result<Vec<(String, i64)>> {
    ranked(
        conn,
        &format!(
            "SELECT song_name || ' - ' || IFNULL(artists, ''), COUNT(*) AS c FROM ({SONGS})
             GROUP BY song_name, artists ORDER BY c DESC, song_name LIMIT ?1"
        ),
        n,
    )
}

fn ranked(conn: &Connection, sql: &str, n: usize) -> Result<Vec<(String, i64)>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([n as i64], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Plays and listening time per month of `year` (months without plays omitted).
pub fn monthly(plays: &[LocalPlay], year: i32) -> Vec<Period> {
    let in_year = plays.iter().filter(|p| p.at.year() == year);
    group(in_year, |p| p.at.month() as i32)
        .into_iter()
        .map(|(month, (plays, ms))| Period {
            label: NaiveDate::from_ymd_opt(year, month as u32, 1)
                .map(|d| d.format("%B").to_string())
                .unwrap_or_default(),
            plays,
            listening_ms: ms,
        })
        .collect()
}

/// Plays and listening time per year.
pub fn yearly(plays: &[LocalPlay]) -> Vec<Period> {
    group(plays.iter(), |p| p.at.year())
        .into_iter()
        .map(|(year, (plays, ms))| Period {
            label: year.to_string(),
            plays,
            listening_ms: ms,
        })
        .collect()
}

fn group<'a>(
    plays: impl Iterator<Item = &'a LocalPlay>,
    key: impl Fn(&LocalPlay) -> i32,
) -> BTreeMap<i32, (i64, i64)> {
    let mut groups = BTreeMap::new();
    for p in plays {
        let entry = groups.entry(key(p)).or_insert((0, 0));
        entry.0 += 1;
        entry.1 += p.duration_ms.unwrap_or(0);
    }
    groups
}

pub fn hours(ms: i64) -> f64 {
    ms as f64 / 3_600_000.0
}

/// `6701` -> `6,701`.
pub fn thousands(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

// ---------------------------------------------------------------------------
// Terminal output
// ---------------------------------------------------------------------------

pub fn print_summary(s: &Summary, conn: &Connection) -> Result<()> {
    let rule = "=".repeat(50);
    println!("{rule}\nSPOTIFY LISTENING STATISTICS\n{rule}");
    println!("\nLifetime");
    println!("  Total plays:     {}", thousands(s.total_plays));
    println!("  Unique artists:  {}", s.unique_artists);
    println!("  Unique albums:   {}", s.unique_albums);
    println!("  Unique songs:    {}", s.unique_songs);
    println!("  Daily average:   {:.1}", s.daily_average);
    println!(
        "  Listening time:  {:.1} h ({} plays with known duration)",
        hours(s.listening_ms),
        s.timed_plays
    );
    println!("\nStreak");
    println!("  Current: {} days", s.current_streak);
    println!("  Longest: {} days", s.longest_streak);
    print_ranked("Top Artists", &top_artists(conn, 5)?);
    print_ranked("Top Albums", &top_albums(conn, 5)?);
    print_ranked("Top Songs", &top_songs(conn, 5)?);
    if let Some((hour, count)) = s.peak_hour() {
        println!("\nPeak hour: {hour:02}:00 ({count} plays)");
    }
    println!("\nPlays by weekday");
    for (name, count) in WEEKDAYS.iter().zip(s.by_weekday) {
        println!("  {name:<10} {count}");
    }
    println!("{rule}");
    Ok(())
}

pub fn print_ranked(title: &str, items: &[(String, i64)]) {
    println!("\n{title}\n{}", "-".repeat(40));
    if items.is_empty() {
        println!("  No data yet");
    }
    for (i, (name, count)) in items.iter().enumerate() {
        println!("{:2}. {name} ({count} plays)", i + 1);
    }
}

pub fn print_streak(current: u32, longest: u32) {
    println!("Current streak: {current} days");
    println!("Longest streak: {longest} days");
}

pub fn print_periods(title: &str, periods: &[Period]) {
    println!("{title}\n{}", "-".repeat(40));
    if periods.is_empty() {
        println!("No data");
        return;
    }
    println!("{:<12} {:>8} {:>10}", "", "Plays", "Hours");
    for p in periods {
        println!(
            "{:<12} {:>8} {:>10.1}",
            p.label,
            thousands(p.plays),
            hours(p.listening_ms)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn set(days: &[&str]) -> BTreeSet<NaiveDate> {
        days.iter().map(|s| d(s)).collect()
    }

    #[test]
    fn streak_empty() {
        assert_eq!(streaks(&BTreeSet::new(), d("2026-01-10")), (0, 0));
    }

    #[test]
    fn streak_single_day_today() {
        assert_eq!(streaks(&set(&["2026-01-10"]), d("2026-01-10")), (1, 1));
    }

    #[test]
    fn streak_single_day_long_ago() {
        assert_eq!(streaks(&set(&["2025-01-10"]), d("2026-01-10")), (0, 1));
    }

    #[test]
    fn streak_ending_yesterday_counts_as_current() {
        let days = set(&["2026-01-07", "2026-01-08", "2026-01-09"]);
        assert_eq!(streaks(&days, d("2026-01-10")), (3, 3));
    }

    #[test]
    fn streak_ending_two_days_ago_is_broken() {
        let days = set(&["2026-01-07", "2026-01-08"]);
        assert_eq!(streaks(&days, d("2026-01-10")), (0, 2));
    }

    #[test]
    fn streak_with_gap() {
        let days = set(&[
            "2025-12-01",
            "2025-12-02",
            "2025-12-03",
            "2025-12-04",
            "2026-01-09",
            "2026-01-10",
        ]);
        assert_eq!(streaks(&days, d("2026-01-10")), (2, 4));
    }

    #[test]
    fn streak_across_month_and_year_boundary() {
        let days = set(&["2025-12-31", "2026-01-01"]);
        assert_eq!(streaks(&days, d("2026-01-01")), (2, 2));
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(6701), "6,701");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn local_derivation_uses_timezone() {
        use chrono_tz::{America::New_York, Europe::London};
        let mut conn = db::open_in_memory().unwrap();
        let play = db::NewPlay {
            played_at: DateTime::parse_from_rfc3339("2026-01-01T02:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            track_id: None,
            song_name: "S".into(),
            album: "A".into(),
            duration_ms: Some(3_600_000),
            artists: vec!["X".into()],
        };
        db::insert_plays(&mut conn, &[play]).unwrap();
        let london = local_plays(&conn, London).unwrap();
        let ny = local_plays(&conn, New_York).unwrap();
        assert_eq!(yearly(&london)[0].label, "2026");
        assert_eq!(yearly(&ny)[0].label, "2025");
        assert_eq!(monthly(&london, 2026)[0].listening_ms, 3_600_000);
    }
}
