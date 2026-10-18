mod config;
mod daemon;
mod db;
mod export;
mod git;
mod import;
mod readme;
mod spotify;
mod stats;

use anyhow::Result;
use chrono::Datelike;
use chrono_tz::Tz;
use clap::{Parser, Subcommand};
use config::{Config, SpotifyCreds};
use std::path::PathBuf;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

/// Track your Spotify listening history in SQLite and publish stats to git.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Sync forever every SYNC_INTERVAL (default when no subcommand is given)
    Daemon,
    /// Sync once and exit
    Sync,
    /// Authorize with Spotify and cache the token (works over SSH)
    Login,
    /// Import history from the legacy Python tracker database
    Import {
        /// Path to the old play_counter.db
        #[arg(long)]
        from: PathBuf,
        /// Timezone the legacy timestamps were recorded in
        #[arg(long, default_value = "Europe/London")]
        source_tz: Tz,
    },
    /// Show a summary of listening statistics
    Stats,
    /// Show the top N artists
    TopArtists {
        #[arg(default_value_t = 10)]
        n: usize,
    },
    /// Show the top N albums
    TopAlbums {
        #[arg(default_value_t = 10)]
        n: usize,
    },
    /// Show the top N songs
    TopSongs {
        #[arg(default_value_t = 10)]
        n: usize,
    },
    /// Show current and longest listening streaks
    Streak,
    /// Plays and listening time per month
    Monthly {
        /// Year to show (default: current year)
        #[arg(long)]
        year: Option<i32>,
    },
    /// Plays and listening time per year
    Yearly,
}

#[tokio::main]
async fn main() -> ExitCode {
    // Rust ignores SIGPIPE by default, so `spotify-tracker stats | head` panics
    // on "failed printing to stdout: Broken pipe". Restore the default action
    // (terminate quietly), which is what a CLI piped into `head` should do.
    // SAFETY: called once, before any threads or stdout writes exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,rspotify=warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match run(cli.command.unwrap_or(Command::Daemon)).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> Result<()> {
    config::load_dotenv()?;
    let cfg = Config::from_env()?;
    match command {
        Command::Daemon => daemon::run(&cfg, &SpotifyCreds::from_env()?).await,
        Command::Sync => {
            let new = daemon::sync_once(&cfg, &SpotifyCreds::from_env()?).await?;
            println!("{new} new plays");
            Ok(())
        }
        Command::Login => spotify::login(&cfg, &SpotifyCreds::from_env()?).await,
        Command::Import { from, source_tz } => {
            let mut conn = db::open(&cfg.database_file)?;
            let r = import::run(&mut conn, &from, source_tz)?;
            println!(
                "imported {}, skipped {} (already present), invalid {}; total plays {}",
                r.imported,
                r.skipped,
                r.invalid,
                db::total_plays(&conn)?
            );
            Ok(())
        }
        Command::Stats => {
            let conn = db::open(&cfg.database_file)?;
            let plays = stats::local_plays(&conn, cfg.timezone)?;
            let summary = stats::summary(&conn, &plays, stats::today(cfg.timezone))?;
            stats::print_summary(&summary, &conn)
        }
        Command::TopArtists { n } => {
            let conn = db::open(&cfg.database_file)?;
            stats::print_ranked(&format!("Top {n} Artists"), &stats::top_artists(&conn, n)?);
            Ok(())
        }
        Command::TopAlbums { n } => {
            let conn = db::open(&cfg.database_file)?;
            stats::print_ranked(&format!("Top {n} Albums"), &stats::top_albums(&conn, n)?);
            Ok(())
        }
        Command::TopSongs { n } => {
            let conn = db::open(&cfg.database_file)?;
            stats::print_ranked(&format!("Top {n} Songs"), &stats::top_songs(&conn, n)?);
            Ok(())
        }
        Command::Streak => {
            let conn = db::open(&cfg.database_file)?;
            let plays = stats::local_plays(&conn, cfg.timezone)?;
            let dates = plays.iter().map(|p| p.at.date_naive()).collect();
            let (current, longest) = stats::streaks(&dates, stats::today(cfg.timezone));
            stats::print_streak(current, longest);
            Ok(())
        }
        Command::Monthly { year } => {
            let conn = db::open(&cfg.database_file)?;
            let plays = stats::local_plays(&conn, cfg.timezone)?;
            let year = year.unwrap_or_else(|| stats::today(cfg.timezone).year());
            stats::print_periods(&format!("{year} by month"), &stats::monthly(&plays, year));
            Ok(())
        }
        Command::Yearly => {
            let conn = db::open(&cfg.database_file)?;
            let plays = stats::local_plays(&conn, cfg.timezone)?;
            stats::print_periods("By year", &stats::yearly(&plays));
            Ok(())
        }
    }
}
