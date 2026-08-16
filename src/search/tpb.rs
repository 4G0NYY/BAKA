use anyhow::{Context, Result};
use serde::Deserialize;

use super::{Ask, Category, Found, Indexer, Torrent, encode, magnet_link};

pub struct Tpb;

// The site is a front end. apibay is the JSON behind it, and it needs no key.
const API: &str = "https://apibay.org";

/// apibay's numbering. 200 is video, and only these are films and shows.
const MOVIES: [u64; 4] = [201, 202, 207, 209];
const SHOWS: [u64; 2] = [205, 208];

/// A search with no hits comes back as one row with this hash rather than as nothing.
const NOTHING: &str = "0000000000000000000000000000000000000000";

impl Indexer for Tpb {
    fn name(&self) -> &'static str {
        "tpb"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Movies, Category::Tv]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let url = match ask {
            Ask::Words(query) => format!("{API}/q.php?q={}", encode(query)),
            // The site keeps a top 100 per category, which is its own curated shelf.
            Ask::Browse(Category::Tv) => format!("{API}/precompiled/data_top100_208.json"),
            Ask::Browse(_) => format!("{API}/precompiled/data_top100_207.json"),
        };
        vec![url]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
        let entries: Vec<Entry> =
            serde_json::from_str(body).context("the pirate bay sent something other than JSON")?;

        let mut found = Vec::new();
        for entry in entries {
            let info_hash = entry.info_hash.to_lowercase();
            let Some(category) = shelf(entry.category.get()) else {
                continue;
            };
            if info_hash == NOTHING {
                continue;
            }

            found.push(Found::Ready(Torrent {
                magnet: magnet_link(&info_hash, &entry.name),
                title: entry.name,
                size_bytes: entry.size.get(),
                seeders: entry.seeders.get() as u32,
                leechers: entry.leechers.get() as u32,
                category,
                source: "tpb",
                info_hash,
            }));
        }
        Ok(found)
    }
}

fn shelf(category: u64) -> Option<Category> {
    if MOVIES.contains(&category) {
        return Some(Category::Movies);
    }
    if SHOWS.contains(&category) {
        return Some(Category::Tv);
    }
    None
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    info_hash: String,
    category: Number,
    size: Number,
    seeders: Number,
    leechers: Number,
}

/// apibay answers a search with its numbers written as strings and its top 100 lists
/// with the same numbers written as numbers.
#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Counted(u64),
    Written(String),
}

impl Number {
    fn get(&self) -> u64 {
        match self {
            Self::Counted(value) => *value,
            Self::Written(text) => text.parse().unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const SEARCH: &str = include_str!("fixtures/tpb.json");
    const TOP: &str = include_str!("fixtures/tpb-top.json");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(Tpb.parse(body, &Ask::Words("dune".to_string())).unwrap())
    }

    #[test]
    fn a_search_lands_on_the_shelf_its_category_number_names() {
        let found = parse(SEARCH);
        assert!(found.iter().any(|t| t.category == Category::Movies));
        assert!(found.iter().any(|t| t.category == Category::Tv));

        let best = found.iter().max_by_key(|t| t.seeders).unwrap();
        assert_eq!(best.title, "Dune Part Two (2024) [1080p] [WEBRip] 88");
        assert_eq!(best.seeders, 1209);
        assert_eq!(best.leechers, 269);
        assert_eq!(best.size_bytes, 2_968_337_547);
        assert_eq!(best.info_hash, "2770fe270845674966e184be60ed1be0fe494f3a");
        assert!(best.magnet.contains(&best.info_hash));
    }

    #[test]
    fn categories_that_are_neither_films_nor_shows_are_left_alone() {
        // The fixture carries a row from 601, which is comics.
        assert!(SEARCH.contains("\"601\""));
        assert!(parse(SEARCH).iter().all(|t| t.source == "tpb"));
        assert_eq!(parse(SEARCH).len(), 3);
    }

    #[test]
    fn the_top_hundred_counts_in_numbers_where_a_search_counts_in_strings() {
        let found = ready(Tpb.parse(TOP, &Ask::Browse(Category::Movies)).unwrap());
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|t| t.seeders > 0 && t.size_bytes > 0));
        assert!(found.iter().all(|t| t.category == Category::Movies));
    }

    #[test]
    fn a_search_with_no_hits_is_not_a_result() {
        let empty = format!(
            r#"[{{"id":"0","name":"No results returned","info_hash":"{NOTHING}",
                 "leechers":"0","seeders":"0","size":"0","category":"0"}}]"#
        );
        assert!(parse(&empty).is_empty());
    }

    #[test]
    fn a_broken_response_says_so() {
        assert!(
            Tpb.parse("<html>nope</html>", &Ask::Words("x".into()))
                .is_err()
        );
    }

    #[test]
    fn each_shelf_browses_its_own_top_hundred() {
        assert!(Tpb.urls(&Ask::Browse(Category::Tv))[0].contains("208"));
        assert!(Tpb.urls(&Ask::Browse(Category::Movies))[0].contains("207"));
    }
}
