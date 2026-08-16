mod config;
mod engine;
mod search;
mod server;
mod tui;

use std::io::{self, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Settings;
use crate::engine::{DownloadId, Engine, Input, Progress, State};
use crate::search::{Category, human_size};

const ART: &str = include_str!("../stuff/ascii-art.txt");
const TAGLINE: &str = "BitTorrent Acquisition & Keyword Aggregator";

#[derive(Parser)]
#[command(name = "baka", version, about = TAGLINE)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Search every source and print what came back.
    Search {
        /// What to look for.
        query: String,
        /// Only ask sources that serve this category.
        #[arg(long, value_enum)]
        category: Option<Category>,
    },
    /// Download a magnet link, an infohash or a .torrent file.
    Get {
        /// A magnet link, a 40 character hex or 32 character base32 infohash,
        /// or the path to a .torrent file.
        target: String,
    },
    /// Show every setting and where it is stored.
    Settings {
        /// Print the settings file path and nothing else.
        #[arg(long)]
        path: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        None => splash(),
        Some(Command::Search { query, category }) => search(&query, category).await,
        Some(Command::Get { target }) => get(&target).await,
        Some(Command::Settings { path }) => show_settings(path),
    }
}

async fn search(query: &str, category: Option<Category>) -> Result<()> {
    let settings = Settings::load()?;
    let outcome = search::run(&settings.search, query, category).await?;

    for failure in &outcome.failures {
        eprintln!("{} skipped: {}", failure.source, failure.reason);
    }

    if outcome.torrents.is_empty() {
        println!("Nothing found.");
        return Ok(());
    }

    println!("{:<6} {:>6}  {:>9}  TITLE", "SOURCE", "SEED", "SIZE");
    for torrent in &outcome.torrents {
        println!(
            "{:<6} {:>6}  {:>9}  {}",
            torrent.source,
            torrent.seeders,
            human_size(torrent.size_bytes),
            torrent.title
        );
    }
    Ok(())
}

async fn get(target: &str) -> Result<()> {
    let settings = Settings::load()?;
    let input = Input::parse(target)?;

    let engine = Engine::start(&settings).await?;

    // Whatever was running last time comes back with the session, and it uses the
    // same bandwidth, so saying nothing about it would be a surprise.
    match engine.snapshot().len() {
        0 => {}
        1 => println!("One torrent from the last run is running as well."),
        n => println!("{n} torrents from the last run are running as well."),
    }

    // A magnet is only a hash until peers hand over the file list, and that step can
    // take longer than the download, so it gets its own line rather than a dead prompt.
    if matches!(input, Input::Magnet(_)) {
        println!("Asking peers for the file list.");
    }

    let download = async {
        let id = engine.add(&input).await?;
        let progress = follow(&engine, id).await?;
        Ok::<_, anyhow::Error>((id, progress))
    };

    let done = tokio::select! {
        result = download => Some(result?),
        _ = tokio::signal::ctrl_c() => None,
    };

    match done {
        None => println!("\nStopped. Run the same command again to carry on where it left off."),
        Some((id, progress)) => {
            println!("Done. Files are in {}", progress.folder.display());
            if settings.seeding.after_completion {
                println!("Seeding. Press Ctrl+C to stop.");
                let _ = tokio::signal::ctrl_c().await;
            } else {
                engine.pause(id).await?;
            }
        }
    }

    engine.shutdown().await;
    Ok(())
}

/// Polling rather than waiting on completion keeps the status line moving and lets a
/// torrent that fails say so instead of hanging.
async fn follow(engine: &Engine, id: DownloadId) -> Result<Progress> {
    let mut named = false;
    loop {
        let progress = engine
            .progress(id)
            .context("the download vanished from the session")?;

        if let Some(error) = &progress.error {
            println!();
            bail!("{error}");
        }

        if !named && let Some(name) = &progress.name {
            println!("\r{name:<70}");
            named = true;
        }

        print!("\r{:<70}", status(&progress));
        io::stdout().flush()?;

        if progress.finished {
            println!();
            return Ok(progress);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn status(progress: &Progress) -> String {
    match progress.state {
        State::Checking => "checking files".to_string(),
        State::Paused => "paused".to_string(),
        State::Failed => "failed".to_string(),
        State::Active if progress.total_bytes == 0 => "waiting for the file list".to_string(),
        State::Active if progress.finished => format!(
            "seeding   {}/s up   {} shared   {} peers",
            human_size(progress.upload_bps),
            human_size(progress.uploaded_bytes),
            progress.peers
        ),
        State::Active => format!(
            "{:>5.1}%   {} of {}   {}/s   {}   {} peers",
            progress.done_bytes as f64 / progress.total_bytes as f64 * 100.0,
            human_size(progress.done_bytes),
            human_size(progress.total_bytes),
            human_size(progress.download_bps),
            human_eta(progress.eta),
            progress.peers
        ),
    }
}

fn human_eta(eta: Option<Duration>) -> String {
    let Some(eta) = eta else {
        return "eta unknown".to_string();
    };
    let seconds = eta.as_secs();
    match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
        (0, 0, s) => format!("eta {s}s"),
        (0, m, s) => format!("eta {m}m {s}s"),
        (h, m, _) => format!("eta {h}h {m}m"),
    }
}

fn splash() -> Result<()> {
    // The art file is CRLF, and a stray carriage return smears braille on some terminals.
    for line in ART.lines() {
        println!("{line}");
    }

    println!();
    println!("BAKA {}", env!("CARGO_PKG_VERSION"));
    println!("{TAGLINE}");
    println!();
    println!("The full terminal interface arrives in phase 3. See ROADMAP.md.");
    println!("Settings: {}", Settings::path()?.display());
    println!("Run baka --help for what works today.");
    Ok(())
}

fn show_settings(path_only: bool) -> Result<()> {
    let path = Settings::path()?;
    if path_only {
        println!("{}", path.display());
        return Ok(());
    }

    let mut settings = Settings::load()?;
    if !path.exists() {
        // Writing it on first look gives the user something to edit before the page exists.
        settings.save()?;
        println!("Created {}", path.display());
    } else {
        println!("{}", path.display());
    }

    let mut group = "";
    for field in settings.fields() {
        if field.group != group {
            group = field.group;
            println!("\n{group}");
        }
        let restart = if field.needs_restart {
            "   (applies on restart)"
        } else {
            ""
        };
        println!("  {:<26}{}{restart}", field.label, field.display());
        println!("  {:<26}{}", "", field.description);
    }

    println!("\nEditing arrives with the settings page in phase 3. Until then, edit the file.");
    Ok(())
}
