mod config;
mod engine;
mod search;
mod server;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Settings;
use crate::search::Category;

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
            search::human_size(torrent.size_bytes),
            torrent.title
        );
    }
    Ok(())
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
    println!("Search and downloads arrive in phase 3. See ROADMAP.md.");
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
