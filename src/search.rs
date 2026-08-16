//! A source only builds a URL and parses a body. The requests, the timeouts and the
//! ranking live here once, which is why every scraper test runs offline.

mod nyaa;
mod yts;

use std::cmp::Reverse;
use std::collections::HashSet;
use std::time::Duration;

use anyhow::Result;
use reqwest::Client;
use tokio::task::JoinSet;

use crate::config;

const USER_AGENT: &str = concat!("baka/", env!("CARGO_PKG_VERSION"));

/// Public trackers, so a magnet built from a bare infohash can still find peers.
const TRACKERS: &[&str] = &[
    "udp://tracker.opentrackr.org:1337/announce",
    "udp://open.demonii.com:1337/announce",
    "udp://open.stealth.si:80/announce",
    "udp://exodus.desync.com:6969/announce",
    "udp://tracker.torrent.eu.org:451/announce",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Category {
    Movies,
    Tv,
    Anime,
    Games,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Torrent {
    pub title: String,
    pub size_bytes: u64,
    pub seeders: u32,
    pub leechers: u32,
    pub category: Category,
    pub source: &'static str,
    pub info_hash: String,
    pub magnet: String,
}

pub struct Outcome {
    pub torrents: Vec<Torrent>,
    pub failures: Vec<Failure>,
}

pub struct Failure {
    pub source: &'static str,
    pub reason: String,
}

pub trait Indexer: Send + Sync {
    fn name(&self) -> &'static str;
    fn categories(&self) -> &'static [Category];
    fn url(&self, query: &str) -> String;
    fn parse(&self, body: &str) -> Result<Vec<Torrent>>;
}

fn all() -> Vec<Box<dyn Indexer>> {
    vec![Box::new(yts::Yts), Box::new(nyaa::Nyaa)]
}

pub async fn run(
    settings: &config::Search,
    query: &str,
    only: Option<Category>,
) -> Result<Outcome> {
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    let limit = Duration::from_secs(settings.timeout_secs.into());

    let mut tasks = JoinSet::new();
    for indexer in all() {
        if !only.is_none_or(|wanted| indexer.categories().contains(&wanted)) {
            continue;
        }
        let client = client.clone();
        let query = query.to_string();
        tasks.spawn(async move {
            let name = indexer.name();
            let work = fetch(&client, indexer.as_ref(), &query);
            match tokio::time::timeout(limit, work).await {
                Ok(result) => (name, result),
                Err(_) => (name, Err(anyhow::anyhow!("timed out"))),
            }
        });
    }

    let mut torrents = Vec::new();
    let mut failures = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        // A panicking scraper is a bug, but it still must not take the search down.
        let (source, result) = joined.unwrap_or_else(|e| ("unknown", Err(e.into())));
        match result {
            Ok(found) => torrents.extend(found),
            Err(e) => failures.push(Failure {
                source,
                reason: format!("{e:#}"),
            }),
        }
    }

    Ok(Outcome {
        torrents: rank(query, torrents, settings),
        failures,
    })
}

async fn fetch(client: &Client, indexer: &dyn Indexer, query: &str) -> Result<Vec<Torrent>> {
    let body = client
        .get(indexer.url(query))
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    indexer.parse(&body)
}

fn rank(query: &str, mut torrents: Vec<Torrent>, settings: &config::Search) -> Vec<Torrent> {
    torrents.sort_by_key(|t| (Reverse(title_matches(query, &t.title)), Reverse(t.seeders)));

    // Sorted first, so the copy kept for a given infohash is the best seeded one.
    let mut seen = HashSet::new();
    torrents.retain(|t| seen.insert(t.info_hash.clone()));

    torrents.retain(|t| t.seeders >= settings.min_seeders);
    torrents.truncate(settings.result_limit as usize);
    torrents
}

fn title_matches(query: &str, title: &str) -> bool {
    let title = title.to_lowercase();
    query
        .split_whitespace()
        .all(|word| title.contains(&word.to_lowercase()))
}

/// An empty title drops the display name rather than sending an empty one, which is
/// what happens when the engine is handed a bare infohash and nothing else.
pub fn magnet_link(info_hash: &str, title: &str) -> String {
    let mut link = format!("magnet:?xt=urn:btih:{info_hash}");
    if !title.is_empty() {
        link.push_str(&format!("&dn={}", encode(title)));
    }
    for tracker in TRACKERS {
        link.push_str(&format!("&tr={}", encode(tracker)));
    }
    link
}

fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn torrent(title: &str, seeders: u32, info_hash: &str) -> Torrent {
        Torrent {
            title: title.to_string(),
            size_bytes: 1024,
            seeders,
            leechers: 0,
            category: Category::Movies,
            source: "test",
            info_hash: info_hash.to_string(),
            magnet: String::new(),
        }
    }

    fn settings() -> config::Search {
        config::Search {
            min_seeders: 0,
            result_limit: 50,
            ..config::Search::default()
        }
    }

    #[test]
    fn a_title_match_outranks_a_bigger_swarm() {
        let found = vec![
            torrent("Something Else", 900, "a"),
            torrent("Dune Part Two", 5, "b"),
        ];
        let ranked = rank("dune part two", found, &settings());
        assert_eq!(ranked[0].title, "Dune Part Two");
    }

    #[test]
    fn the_best_seeded_copy_of_a_duplicate_wins() {
        let found = vec![
            torrent("Dune from one source", 10, "same"),
            torrent("Dune from another", 400, "same"),
        ];
        let ranked = rank("dune", found, &settings());
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].seeders, 400);
    }

    #[test]
    fn dead_swarms_can_be_filtered_out() {
        let found = vec![torrent("Dune", 0, "a"), torrent("Dune", 3, "b")];
        let picky = config::Search {
            min_seeders: 1,
            ..settings()
        };
        let ranked = rank("dune", found, &picky);
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].seeders, 3);
    }

    #[test]
    fn a_magnet_carries_the_hash_the_name_and_trackers() {
        let magnet = magnet_link("abc123", "Some Title");
        assert!(magnet.starts_with("magnet:?xt=urn:btih:abc123"));
        assert!(magnet.contains("dn=Some%20Title"));
        assert_eq!(magnet.matches("&tr=").count(), TRACKERS.len());
    }

    #[test]
    fn a_magnet_with_no_title_has_no_empty_name_field() {
        let magnet = magnet_link("abc123", "");
        assert!(!magnet.contains("dn="));
        assert_eq!(magnet.matches("&tr=").count(), TRACKERS.len());
    }

    #[test]
    fn sizes_read_the_way_a_person_would_say_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3_285_649_981), "3.1 GiB");
    }
}
