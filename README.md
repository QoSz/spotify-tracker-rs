# Spotify Tracker

Tracking my Spotify listening history. A long-running Rust daemon that polls
Spotify's recently-played endpoint, stores every play in SQLite, and publishes
a rolling JSON history plus the stats below to git.

## Usage

```bash
cargo build --release
cp .env.example .env            # fill in SPOTIPY_CLIENT_ID / SECRET / REDIRECT_URI
./target/release/spotify-tracker login     # paste the redirect URL when asked
./target/release/spotify-tracker import --from ../../Python/Spotify-Tracker/play_counter.db
./target/release/spotify-tracker sync      # one-shot
./target/release/spotify-tracker daemon    # forever (also the default)
```

Stats: `stats`, `top-artists [N]`, `top-albums [N]`, `top-songs [N]`, `streak`,
`monthly [--year YYYY]`, `yearly`. Run with `--help` for details. Log level via
`RUST_LOG` (default `info,rspotify=warn`).

### systemd (user service)

```bash
./scripts/install-service.sh    # copies the unit, daemon-reload, enable --now
journalctl --user -u spotify-tracker -f
```

## Design notes

- Plays are stored by UTC instant (`plays.played_at_utc`, RFC3339, second
  precision) with artists in `play_artists`. Local date/hour/weekday/month/year
  are computed in Rust at query time from `TIMEZONE`, so changing the timezone
  never requires a migration or reindex, and a few thousand rows take
  milliseconds to process.
- Plays imported from the Python tracker have no `duration_ms`/`track_id`, so
  listening time only covers plays fetched by this tool.
- The section between the markers below is regenerated on every sync that
  finds new plays; everything outside them is left alone.

<!-- STATS:START -->
## Lifetime Stats

**7,174** songs listened to

- **1559** unique artists
- **1673** unique albums
- **2066** unique songs
- **11.9** songs per day (average)

## Top Artists (All Time)

1. Drake (342 plays)
2. MIND (276 plays)
3. Nu Aspect (214 plays)
4. Small Town Kid (206 plays)
5. Astrality (198 plays)

## Listening Streak

- Current: **7** days
- Longest: **34** days

## Listening Patterns

- Peak hour: **11:00**

## Listening Time

- **22.9** hours listened (473 plays with known duration)
- 6,701 older plays (imported from the legacy tracker) have no duration and are excluded

## This Year by Month

| Month | Plays | Hours |
|---|---:|---:|
| January | 748 | 0.0 |
| February | 1,055 | 0.0 |
| March | 1,080 | 0.0 |
| April | 719 | 0.0 |
| May | 588 | 0.0 |
| June | 589 | 0.0 |
| July | 508 | 0.0 |
| October | 473 | 22.9 |

---
*Automatically updated by spotify-tracker (Rust)*
<!-- STATS:END -->
