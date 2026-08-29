use anyhow::{Context, Result};
use roxmltree::Document;

use super::{Ask, Category, Found, Indexer, Torrent, encode, magnet_link, parse_size};

pub struct Nyaa;

const BASE: &str = "https://nyaa.si/";

// Nyaa's category for anime somebody has subtitled, which is what the Anime shelf is.
const SUBTITLED: &str = "1_2";

// Its category for written work that has been translated. Manga and light novels.
const TRANSLATED: &str = "3_1";

impl Indexer for Nyaa {
    fn name(&self) -> &'static str {
        "nyaa"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Anime, Category::Books]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let url = match ask {
            Ask::Words(query) => format!("{BASE}?page=rss&q={}", encode(query)),
            Ask::Browse(Category::Books) => format!("{BASE}?page=rss&c={TRANSLATED}"),
            Ask::Browse(_) => format!("{BASE}?page=rss&c={SUBTITLED}"),
        };
        vec![url]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
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
            let Some(category) = shelf(field("categoryId")) else {
                continue;
            };

            found.push(Found::Ready(Torrent {
                magnet: magnet_link(&info_hash, title),
                title: title.to_string(),
                size_bytes: parse_size(field("size")),
                seeders: field("seeders").parse().unwrap_or(0),
                leechers: field("leechers").parse().unwrap_or(0),
                category,
                source: "nyaa",
                info_hash,
            }));
        }
        Ok(found)
    }
}

/// A search is answered from the whole site, so an item names the shelf it is on and
/// the two this source serves are the two worth keeping.
fn shelf(id: &str) -> Option<Category> {
    match id.split_once('_')?.0 {
        "1" => Some(Category::Anime),
        "3" => Some(Category::Books),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/nyaa.xml");
    const BOOKS: &str = include_str!("fixtures/nyaa-books.xml");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(
            Nyaa.parse(body, &Ask::Words("one piece".to_string()))
                .unwrap(),
        )
    }

    #[test]
    fn a_saved_feed_becomes_torrents() {
        let found = parse(FIXTURE);
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
    fn the_feed_says_which_shelf_an_item_is_on() {
        assert!(parse(FIXTURE).iter().all(|t| t.category == Category::Anime));

        let books = parse(BOOKS);
        assert_eq!(books.len(), 3);
        assert!(books.iter().all(|t| t.category == Category::Books));
        assert_eq!(
            books[0].title,
            "A Good Day Starts with Cats and Books [Audiobook] [Seven Seas Siren] [Stick & Oak]"
        );
        assert_eq!(books[0].seeders, 31);
        assert_eq!(books[0].size_bytes, 357_040_128);
    }

    #[test]
    fn a_shelf_this_source_does_not_serve_is_left_alone() {
        let music = "<rss><channel><item><title>An album</title>
            <infoHash>abc</infoHash><categoryId>2_2</categoryId></item></channel></rss>";
        assert!(parse(music).is_empty());
    }

    #[test]
    fn an_item_with_no_hash_is_skipped_not_fatal() {
        let feed = "<rss><channel><item><title>No hash here</title></item></channel></rss>";
        assert!(parse(feed).is_empty());
    }

    #[test]
    fn a_broken_feed_says_so() {
        assert!(
            Nyaa.parse("not xml at all", &Ask::Browse(Category::Anime))
                .is_err()
        );
    }

    #[test]
    fn browsing_asks_for_the_shelf_rather_than_for_words() {
        let browse = Nyaa.urls(&Ask::Browse(Category::Anime));
        assert!(browse[0].contains(SUBTITLED));
        assert!(!browse[0].contains("&q="));
        assert!(Nyaa.urls(&Ask::Browse(Category::Books))[0].contains(TRANSLATED));
    }
}
