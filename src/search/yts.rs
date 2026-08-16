use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Ask, Category, Found, Indexer, Torrent, encode, magnet_link};

pub struct Yts;

// yts.mx does not resolve on some networks, and the API itself now points callers here.
const BASE: &str = "https://movies-api.accel.li/api/v2/list_movies.json";

impl Indexer for Yts {
    fn name(&self) -> &'static str {
        "yts"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Movies]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let url = match ask {
            Ask::Words(query) => {
                format!("{BASE}?limit=50&sort_by=seeds&query_term={}", encode(query))
            }
            // The most downloaded films are the library YTS keeps of its own accord.
            Ask::Browse(_) => format!("{BASE}?limit=50&sort_by=download_count"),
        };
        vec![url]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
        let response: Response =
            serde_json::from_str(body).context("yts sent something other than its usual JSON")?;

        let mut found = Vec::new();
        for movie in &response.data.movies {
            for entry in &movie.torrents {
                let title = format!("{} [{} {}]", movie.title_long, entry.quality, entry.kind);
                let info_hash = entry.hash.to_lowercase();
                found.push(Found::Ready(Torrent {
                    magnet: magnet_link(&info_hash, &title),
                    title,
                    size_bytes: entry.size_bytes,
                    seeders: entry.seeds,
                    leechers: entry.peers,
                    category: Category::Movies,
                    source: "yts",
                    info_hash,
                }));
            }
        }
        Ok(found)
    }
}

#[derive(Deserialize)]
struct Response {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    // A search with no hits drops the key rather than sending an empty list.
    #[serde(default)]
    movies: Vec<Movie>,
}

#[derive(Deserialize)]
struct Movie {
    title_long: String,
    #[serde(default)]
    torrents: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    hash: String,
    quality: String,
    #[serde(rename = "type")]
    kind: String,
    seeds: u32,
    peers: u32,
    size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/yts.json");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(Yts.parse(body, &Ask::Words("dune".to_string())).unwrap())
    }

    #[test]
    fn every_quality_becomes_its_own_result() {
        let found = parse(FIXTURE);
        assert_eq!(found.len(), 6);

        let best = found.iter().max_by_key(|t| t.seeders).unwrap();
        assert!(best.title.starts_with("Dune: Part Two (2024) ["));
        assert_eq!(best.seeders, 100);
        assert_eq!(best.info_hash.len(), 40);
        assert!(best.magnet.contains(&best.info_hash));
        assert!(found.iter().all(|t| t.size_bytes > 0));
    }

    #[test]
    fn hashes_come_out_lowercase_so_duplicates_collapse() {
        let found = parse(FIXTURE);
        assert!(
            found
                .iter()
                .all(|t| t.info_hash == t.info_hash.to_lowercase())
        );
    }

    #[test]
    fn a_search_with_no_hits_is_not_an_error() {
        assert!(parse(r#"{"status":"ok","data":{"movie_count":0}}"#).is_empty());
    }

    #[test]
    fn a_broken_response_says_so() {
        assert!(
            Yts.parse(
                "<html>down for maintenance</html>",
                &Ask::Browse(Category::Movies)
            )
            .is_err()
        );
    }

    #[test]
    fn browsing_asks_for_the_library_rather_than_for_words() {
        let browse = Yts.urls(&Ask::Browse(Category::Movies));
        assert!(browse[0].contains("sort_by=download_count"));
        assert!(!browse[0].contains("query_term"));
    }
}
