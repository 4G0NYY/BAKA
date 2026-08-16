use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Ask, Category, Found, Indexer, Torrent, magnet_link, title_matches};

pub struct Eztv;

// The API takes a show id, never words, so both a browse and a search read the same
// feed of new releases and a search keeps the titles that match what was typed.
const FEED: &str = "https://eztvx.to/api/get-torrents?limit=100&page=1";

impl Indexer for Eztv {
    fn name(&self) -> &'static str {
        "eztv"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Tv]
    }

    fn urls(&self, _ask: &Ask) -> Vec<String> {
        vec![FEED.to_string()]
    }

    fn parse(&self, body: &str, ask: &Ask) -> Result<Vec<Found>> {
        let response: Response =
            serde_json::from_str(body).context("eztv sent something other than its usual JSON")?;

        let mut found = Vec::new();
        for entry in response.torrents {
            let info_hash = entry.hash.to_lowercase();
            let title = match entry.title.is_empty() {
                true => entry.filename,
                false => entry.title,
            };
            if info_hash.is_empty() || title.is_empty() {
                continue;
            }
            if let Ask::Words(query) = ask
                && !title_matches(query, &title)
            {
                continue;
            }

            found.push(Found::Ready(Torrent {
                magnet: match entry.magnet_url.is_empty() {
                    true => magnet_link(&info_hash, &title),
                    false => entry.magnet_url,
                },
                title,
                size_bytes: entry.size_bytes.parse().unwrap_or(0),
                seeders: entry.seeds,
                leechers: entry.peers,
                category: Category::Tv,
                source: "eztv",
                info_hash,
            }));
        }
        Ok(found)
    }
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    torrents: Vec<Entry>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    title: String,
    filename: String,
    hash: String,
    magnet_url: String,
    seeds: u32,
    peers: u32,
    // EZTV writes the byte count as a string.
    size_bytes: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/eztv.json");

    #[test]
    fn the_feed_of_new_releases_becomes_torrents() {
        let found = ready(Eztv.parse(FIXTURE, &Ask::Browse(Category::Tv)).unwrap());
        assert_eq!(found.len(), 3);

        let first = &found[0];
        assert!(first.title.starts_with("The Only Way Is Essex S37E05"));
        assert_eq!(first.info_hash, "e21d289964f79d78c1af82c605c483b9fbde8a1e");
        assert!(first.magnet.starts_with("magnet:?xt=urn:btih:"));
        assert!(first.size_bytes > 0);
        assert!(found.iter().all(|t| t.category == Category::Tv));
    }

    #[test]
    fn words_keep_only_what_the_feed_already_had() {
        let found = ready(Eztv.parse(FIXTURE, &Ask::Words("essex".into())).unwrap());
        assert_eq!(found.len(), 1);
        assert!(found[0].title.to_lowercase().contains("essex"));

        let missing = ready(Eztv.parse(FIXTURE, &Ask::Words("dune".into())).unwrap());
        assert!(missing.is_empty());
    }

    #[test]
    fn an_empty_feed_is_not_an_error() {
        let found = Eztv.parse(r#"{"torrents_count":0}"#, &Ask::Browse(Category::Tv));
        assert!(found.unwrap().is_empty());
    }

    #[test]
    fn a_broken_response_says_so() {
        assert!(
            Eztv.parse("<html>down</html>", &Ask::Browse(Category::Tv))
                .is_err()
        );
    }
}
