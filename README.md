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
*Stats appear here after the first sync with new plays.*
<!-- STATS:END -->
