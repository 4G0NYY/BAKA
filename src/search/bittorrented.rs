use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Ask, Category, Found, Indexer, Torrent, encode, magnet_link};

pub struct BitTorrented;

const API: &str = "https://bittorrented.com/api/search/torrents";

/// The API turns away anything shorter, and it keeps no listing of its own, so this
/// source sits out a browse rather than pretending to have one.
const SHORTEST: usize = 3;

impl Indexer for BitTorrented {
    fn name(&self) -> &'static str {
        "bittorrented"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Movies, Category::Tv]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let Ask::Words(query) = ask else {
            return Vec::new();
        };
        if query.chars().count() < SHORTEST {
            return Vec::new();
        }
        // Video only. The index carries every other kind of file as well, and the
        // other shelves have sources that know what they are looking at.
        vec![format!(
            "{API}?q={}&type=video&limit=50&sortBy=seeders&sortOrder=desc",
            encode(query)
        )]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
        let response: Response = serde_json::from_str(body)
            .context("bittorrented sent something other than its usual JSON")?;

        let mut found = Vec::new();
        for entry in response.results {
            let info_hash = entry.torrent_infohash.to_lowercase();
            if info_hash.len() != 40 || !info_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let title = match entry.torrent_name.is_empty() {
                true => info_hash.clone(),
                false => entry.torrent_name,
            };

            found.push(Found::Ready(Torrent {
                magnet: magnet_link(&info_hash, &title),
                category: shelf(&title),
                title,
                size_bytes: entry.torrent_total_size,
                seeders: entry.torrent_seeders.unwrap_or(0),
                leechers: entry.torrent_leechers.unwrap_or(0),
                source: "bittorrented",
                info_hash,
            }));
        }
        Ok(found)
    }
}

/// The index knows only that a result is video, so a season and episode number in the
/// name is the one honest signal for which shelf it belongs on.
fn shelf(title: &str) -> Category {
    match episode_number(title) {
        true => Category::Tv,
        false => Category::Movies,
    }
}

fn episode_number(title: &str) -> bool {
    let name = title.to_ascii_lowercase();
    if name.contains("season ") || name.contains("complete series") {
        return true;
    }

    let letters = name.as_bytes();
    for (at, letter) in letters.iter().enumerate() {
        if *letter != b's' {
            continue;
        }
        let season = digits(letters, at + 1);
        if season == at + 1 || letters.get(season) != Some(&b'e') {
            continue;
        }
        if digits(letters, season + 1) > season + 1 {
            return true;
        }
    }
    false
}

fn digits(letters: &[u8], from: usize) -> usize {
    let mut at = from;
    while letters.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
    }
    at
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    results: Vec<Entry>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Entry {
    torrent_infohash: String,
    torrent_name: String,
    torrent_total_size: u64,
    torrent_seeders: Option<u32>,
    torrent_leechers: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/bittorrented.json");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(
            BitTorrented
                .parse(body, &Ask::Words("dune".to_string()))
                .unwrap(),
        )
    }

    #[test]
    fn a_saved_response_becomes_torrents() {
        let found = parse(FIXTURE);
        assert_eq!(found.len(), 3);

        let first = &found[0];
        assert_eq!(
            first.title,
            "Dune.2021.1080p.BluRay.REMUX.AVC.DTS-HD.MA.TrueHD.7.1.Atmos-FGT"
        );
        assert_eq!(first.info_hash, "d0edbfa5f48275bcc841e6979b7601e6f7eea6bc");
        assert_eq!(first.seeders, 171);
        assert_eq!(first.size_bytes, 39_962_824_835);
        assert!(first.magnet.contains(&first.info_hash));
    }

    #[test]
    fn a_row_without_a_usable_hash_is_dropped_not_fatal() {
        let body = r#"{"results":[{"torrent_infohash":"nope","torrent_name":"Something"}]}"#;
        assert!(parse(body).is_empty());
    }

    #[test]
    fn an_episode_number_is_what_puts_a_result_on_the_tv_shelf() {
        assert_eq!(shelf("Dune.Prophecy.S01E01.WEB.x264"), Category::Tv);
        assert_eq!(shelf("Some Show Season 3 1080p"), Category::Tv);
        assert_eq!(shelf("Dune.Part.Two.2024.2160p"), Category::Movies);
        assert_eq!(shelf("Se7en.1995.1080p.BluRay"), Category::Movies);
    }

    #[test]
    fn a_query_the_api_would_turn_away_is_never_sent() {
        assert!(BitTorrented.urls(&Ask::Words("ab".into())).is_empty());
        assert!(!BitTorrented.urls(&Ask::Words("dune".into())).is_empty());
    }

    #[test]
    fn a_source_with_no_listing_sits_out_a_browse() {
        for category in Category::ALL {
            assert!(BitTorrented.urls(&Ask::Browse(category)).is_empty());
        }
    }

    #[test]
    fn a_broken_response_says_so() {
        assert!(
            BitTorrented
                .parse("<html>nope</html>", &Ask::Words("x".into()))
                .is_err()
        );
    }
}
