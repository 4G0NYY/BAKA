use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Ask, Category, Found, Indexer, Torrent, encode, hex_hash};

pub struct SubsPlease;

const API: &str = "https://subsplease.org/api/";

/// Every release comes in three sizes. One result per show reads better than three
/// copies of the same episode, so the best picture wins.
const QUALITY: [&str; 3] = ["1080", "720", "480"];

impl Indexer for SubsPlease {
    fn name(&self) -> &'static str {
        "subsplease"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Anime]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let url = match ask {
            Ask::Words(query) => format!("{API}?tz=UTC&f=search&s={}", encode(query)),
            Ask::Browse(_) => format!("{API}?tz=UTC&f=latest"),
        };
        vec![url]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
        let response: serde_json::Value = serde_json::from_str(body)
            .context("subsplease sent something other than its usual JSON")?;

        // A search with no hits answers with an empty list where a hit answers with a
        // map of shows, so an unexpected shape here is an empty result, not a fault.
        let Some(entries) = response.as_object() else {
            return Ok(Vec::new());
        };

        let mut found = Vec::new();
        for value in entries.values() {
            let Ok(entry) = serde_json::from_value::<Entry>(value.clone()) else {
                continue;
            };
            let Some(release) = best(&entry.downloads) else {
                continue;
            };
            let Some(info_hash) = hex_hash(&release.magnet) else {
                continue;
            };

            let episode = match entry.episode.is_empty() {
                true => String::new(),
                false => format!(" - {}", entry.episode),
            };
            found.push(Found::Ready(Torrent {
                title: format!("{}{episode} [{}p]", entry.show, release.res),
                // The API states the byte count in the magnet link it hands out.
                size_bytes: exact_length(&release.magnet),
                // SubsPlease publishes releases, not swarm counts. Nothing here is a
                // dead torrent; the number is simply not on offer.
                seeders: 0,
                leechers: 0,
                category: Category::Anime,
                source: "subsplease",
                info_hash,
                magnet: release.magnet.clone(),
            }));
        }
        Ok(found)
    }
}

fn best(downloads: &[Release]) -> Option<&Release> {
    QUALITY
        .iter()
        .find_map(|wanted| downloads.iter().find(|release| release.res == *wanted))
        .or_else(|| downloads.first())
}

fn exact_length(magnet: &str) -> u64 {
    let Some(at) = magnet.find("&xl=") else {
        return 0;
    };
    magnet[at + 4..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct Entry {
    #[serde(default)]
    show: String,
    #[serde(default)]
    episode: String,
    #[serde(default)]
    downloads: Vec<Release>,
}

#[derive(Deserialize)]
struct Release {
    #[serde(default)]
    res: String,
    #[serde(default)]
    magnet: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/subsplease.json");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(
            SubsPlease
                .parse(body, &Ask::Words("one piece".to_string()))
                .unwrap(),
        )
    }

    #[test]
    fn one_episode_becomes_one_result_at_the_best_picture() {
        let found = parse(FIXTURE);
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|t| t.title.ends_with("[1080p]")));
        assert!(found.iter().all(|t| t.size_bytes > 0));

        let titles: Vec<&str> = found.iter().map(|t| t.title.as_str()).collect();
        assert!(titles.contains(&"One Piece - 1173 [1080p]"), "{titles:?}");
        assert!(
            found
                .iter()
                .all(|t| t.magnet.starts_with("magnet:?xt=urn:btih:"))
        );
    }

    #[test]
    fn a_base32_magnet_still_gives_a_hex_hash_to_match_nyaa_on() {
        let found = parse(FIXTURE);
        for torrent in &found {
            assert_eq!(torrent.info_hash.len(), 40);
            assert!(torrent.info_hash.bytes().all(|b| b.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn a_search_with_no_hits_answers_with_a_list_and_that_is_fine() {
        assert!(parse("[]").is_empty());
    }

    #[test]
    fn a_broken_response_says_so() {
        assert!(
            SubsPlease
                .parse("not json", &Ask::Browse(Category::Anime))
                .is_err()
        );
    }

    #[test]
    fn the_byte_count_comes_out_of_the_magnet_link() {
        assert_eq!(
            exact_length("magnet:?xt=urn:btih:abc&xl=376124912&dn=x"),
            376_124_912
        );
        assert_eq!(exact_length("magnet:?xt=urn:btih:abc"), 0);
    }
}
