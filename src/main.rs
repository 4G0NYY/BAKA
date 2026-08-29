mod config;
mod engine;
mod search;
mod server;
mod session;
mod tui;

use std::io::{self, Write};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config::Settings;
use crate::engine::{DownloadId, Engine, Input, Progress, State, human_eta};
use crate::search::{Category, human_size};

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
        /// What to look for. Leave it out to browse what the sources are listing.
        query: Option<String>,
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
    /// Download anything dropped into a directory.
    Watch {
        /// Where to look. Defaults to the watch folder on the Settings page.
        directory: Option<PathBuf>,
        /// Keep running after this terminal closes.
        #[arg(long)]
        daemon: bool,
    },
    /// Accept magnets over HTTP.
    Serve {
        /// Keep running after this terminal closes.
        #[arg(long)]
        daemon: bool,
    },
    /// Serve finished downloads over HTTP.
    Files {
        /// Keep running after this terminal closes.
        #[arg(long)]
        daemon: bool,
    },
    /// Drive a BAKA that is already running, here or on another machine.
    Attach {
        /// Where it is listening. Defaults to the bind address and the magnet intake
        /// port on the Settings page.
        address: Option<String>,
    },
    /// Open the settings page without the rest of the interface.
    Settings {
        /// Print the settings file path and nothing else.
        #[arg(long)]
        path: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let loaded = Settings::load()?;
    let complaint = loaded.complaint();
    let settings = loaded.settings;
    let command = Cli::parse().command;

    // The interface has a status line to say this on. Everything else has stderr.
    let has_a_status_line = matches!(
        command,
        None | Some(Command::Attach { .. }) | Some(Command::Settings { path: false })
    );
    if let Some(complaint) = &complaint
        && !has_a_status_line
    {
        eprintln!("{complaint}");
    }

    match command {
        None => tui::run(settings, complaint).await,
        Some(Command::Search { query, category }) => {
            search(&settings, query.as_deref().unwrap_or_default(), category).await
        }
        Some(Command::Get { target }) => get(&settings, &target).await,
        Some(Command::Watch { directory, daemon }) => match daemon {
            true => server::detach(),
            false => {
                let folder = directory.unwrap_or_else(|| settings.server.watch_folder.clone());
                server::watch(&settings, &folder).await
            }
        },
        Some(Command::Serve { daemon }) => match daemon {
            true => server::detach(),
            false => server::serve(&settings).await,
        },
        Some(Command::Files { daemon }) => match daemon {
            true => server::detach(),
            false => server::files(&settings).await,
        },
        Some(Command::Attach { address }) => {
            let remote = server::Remote::reach(&settings.server, address.as_deref()).await?;
            tui::attach(settings, remote, complaint).await
        }
        Some(Command::Settings { path }) => match path {
            true => print_path(),
            false => tui::settings_page(settings, complaint).await,
        },
    }
}

async fn search(settings: &Settings, query: &str, category: Option<Category>) -> Result<()> {
    let outcome = search::run(&settings.search, query, category).await?;

    for failure in &outcome.failures {
        eprintln!("{} skipped: {}", failure.source, failure.reason);
    }

    if outcome.torrents.is_empty() {
        println!("Nothing found.");
        return Ok(());
    }

    println!(
        "{:<12} {:<10} {:>6}  {:>9}  TITLE",
        "SOURCE", "SHELF", "SEED", "SIZE"
    );
    for torrent in &outcome.torrents {
        println!(
            "{:<12} {:<10} {:>6}  {:>9}  {}",
            torrent.source,
            torrent.category.to_string(),
            torrent.seeders,
            human_size(torrent.size_bytes),
            torrent.title
        );
    }
    Ok(())
}

async fn get(settings: &Settings, target: &str) -> Result<()> {
    let input = Input::parse(target)?;
    let engine = Engine::start(settings).await?;

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
        let id = engine.add(&input, &settings.downloads.folder).await?;
        let progress = follow(&engine, settings, id).await?;
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
                tokio::select! {
                    result = seed(&engine, settings, id) => result?,
                    _ = tokio::signal::ctrl_c() => println!(),
                }
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
async fn follow(engine: &Engine, settings: &Settings, id: DownloadId) -> Result<Progress> {
    let mut named = false;
    loop {
        // The same queue the interface runs, so the concurrency limits mean one thing.
        engine.enforce(settings).await?;
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

/// Seeding ends when the queue says it does, which is what stop at ratio and the seed
/// limit are for. Until then this is the same status line with different numbers.
async fn seed(engine: &Engine, settings: &Settings, id: DownloadId) -> Result<()> {
    loop {
        engine.enforce(settings).await?;
        let Some(progress) = engine.progress(id) else {
            return Ok(());
        };
        if progress.state == State::Paused {
            println!("\nStopped seeding.");
            return Ok(());
        }

        print!("\r{:<70}", status(&progress));
        io::stdout().flush()?;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn status(progress: &Progress) -> String {
    match progress.state {
        State::Checking => "checking files".to_string(),
        State::Queued => "queued behind the other downloads".to_string(),
        State::Paused => "paused".to_string(),
        State::Failed => "failed".to_string(),
        State::Active if progress.total_bytes == 0 => "waiting for the file list".to_string(),
        State::Active if progress.finished => format!(
            "seeding   {}/s up   {} shared   ratio {:.2}   {} peers",
            human_size(progress.upload_bps),
            human_size(progress.uploaded_bytes),
            progress.ratio,
            progress.peers
        ),
        State::Active => format!(
            "{:>5.1}%   {} of {}   {}/s   eta {}   {} peers",
            progress.done_bytes as f64 / progress.total_bytes as f64 * 100.0,
            human_size(progress.done_bytes),
            human_size(progress.total_bytes),
            human_size(progress.download_bps),
            human_eta(progress.eta),
            progress.peers
        ),
    }
}

fn print_path() -> Result<()> {
    println!("{}", Settings::path()?.display());
    Ok(())
}
