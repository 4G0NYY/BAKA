mod config;
mod engine;
mod search;
mod server;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::config::Settings;

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
    /// Show every setting and where it is stored.
    Settings {
        /// Print the settings file path and nothing else.
        #[arg(long)]
        path: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        None => splash(),
        Some(Command::Settings { path }) => show_settings(path),
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
