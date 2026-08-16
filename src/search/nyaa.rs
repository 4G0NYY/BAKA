use anyhow::{Context, Result};
use roxmltree::Document;

use super::{Category, Indexer, Torrent, encode, magnet_link};

pub struct Nyaa;

const BASE: &str = "https://nyaa.si/";

impl Indexer for Nyaa {
    fn name(&self) -> &'static str {
        "nyaa"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Anime]
    }

    fn url(&self, query: &str) -> String {
        format!("{BASE}?page=rss&q={}", encode(query))
    }

    fn parse(&self, body: &str) -> Result<Vec<Torrent>> {
        let feed = Document::parse(body).context("nyaa sent something other than RSS")?;

        let mut found = Vec::new();
        for item in feed.descendants().filter(|node| node.has_tag_name("item")) {
            let field = |name: &str| {
                item.children()
                    .find(|child| child.tag_name().name() == name)
                    .and_then(|child| child.text())
                    .unwrap_or_default()
                    .trim()
            };

            let title = field("title");
            let info_hash = field("infoHash").to_lowercase();
            // One malformed item must not cost us the rest of the feed.
            if title.is_empty() || info_hash.is_empty() {
                continue;
            }

            found.push(Torrent {
                magnet: magnet_link(&info_hash, title),
                title: title.to_string(),
                size_bytes: parse_size(field("size")),
                seeders: field("seeders").parse().unwrap_or(0),
                leechers: field("leechers").parse().unwrap_or(0),
                category: Category::Anime,
                source: "nyaa",
                info_hash,
            });
        }
        Ok(found)
    }
}

fn parse_size(text: &str) -> u64 {
    let mut parts = text.split_whitespace();
    let Some(Ok(amount)) = parts.next().map(str::parse::<f64>) else {
        return 0;
    };
    let unit = match parts.next().unwrap_or_default() {
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (amount * unit) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("fixtures/nyaa.xml");

    #[test]
    fn a_saved_feed_becomes_torrents() {
        let found = Nyaa.parse(FIXTURE).unwrap();
        assert_eq!(found.len(), 3);

        let first = &found[0];
        assert_eq!(
            first.title,
            "[KiyoshiiSubs] One Piece - 1173v2 [1080p][H.265 - 10Bit].mkv"
        );
        assert_eq!(first.seeders, 140);
        assert_eq!(first.leechers, 4);
        assert_eq!(first.info_hash, "7f323023040a468749fb2336350588c17907522f");
        assert_eq!(first.size_bytes, 852_282_572);
        assert!(first.magnet.contains(&first.info_hash));
    }

    #[test]
    fn an_item_with_no_hash_is_skipped_not_fatal() {
        let feed = "<rss><channel><item><title>No hash here</title></item></channel></rss>";
        assert!(Nyaa.parse(feed).unwrap().is_empty());
    }

    #[test]
    fn a_broken_feed_says_so() {
        assert!(Nyaa.parse("not xml at all").is_err());
    }

    #[test]
    fn sizes_come_back_in_bytes() {
        assert_eq!(parse_size("812.8 MiB"), 852_282_572);
        assert_eq!(parse_size("1.5 GiB"), 1_610_612_736);
        assert_eq!(parse_size("nonsense"), 0);
    }
}
