use anyhow::{Context, Result};
use roxmltree::Document;

use super::{Ask, Category, Found, Indexer, Torrent, encode, first_magnet, hex_hash, parse_size};

pub struct FitGirl;

const HOME: &str = "https://fitgirl-repacks.site";

/// Where the post body states how big the repack is once it is installed.
const SIZE: &str = "Repack Size:";

impl Indexer for FitGirl {
    fn name(&self) -> &'static str {
        "fitgirl"
    }

    fn categories(&self) -> &'static [Category] {
        &[Category::Games]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let url = match ask {
            Ask::Words(query) => format!("{HOME}/?s={}&feed=rss2", encode(query)),
            Ask::Browse(_) => format!("{HOME}/feed/"),
        };
        vec![url]
    }

    fn parse(&self, body: &str, _ask: &Ask) -> Result<Vec<Found>> {
        let feed = Document::parse(body).context("fitgirl sent something other than RSS")?;

        let mut found = Vec::new();
        for item in feed.descendants().filter(|node| node.has_tag_name("item")) {
            let field = |name: &str| {
                item.children()
                    .find(|child| child.tag_name().name() == name)
                    .and_then(|child| child.text())
                    .unwrap_or_default()
            };

            let title = field("title").trim().to_string();
            let post = field("encoded");
            let Some(magnet) = first_magnet(post) else {
                continue;
            };
            let Some(info_hash) = hex_hash(&magnet) else {
                continue;
            };

            found.push(Found::Ready(Torrent {
                title,
                size_bytes: repack_size(post),
                // A blog post is not a tracker. Nothing here is a dead torrent; the
                // swarm is simply not something the feed knows.
                seeders: 0,
                leechers: 0,
                category: Category::Games,
                source: "fitgirl",
                info_hash,
                magnet,
            }));
        }
        Ok(found)
    }
}

/// A range like "328/804 MB" is the smallest and the largest selective install, and
/// the largest is the whole game.
fn repack_size(post: &str) -> u64 {
    let Some(at) = post.find(SIZE) else {
        return 0;
    };
    let stated: String = post[at + SIZE.len()..].chars().take(60).collect();
    let plain = without_tags(&stated);
    parse_size(plain.trim().rsplit('/').next().unwrap_or_default())
}

fn without_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;
    for letter in html.chars() {
        match letter {
            '<' => inside = true,
            '>' => inside = false,
            _ if !inside => out.push(letter),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ready;

    const FIXTURE: &str = include_str!("fixtures/fitgirl.xml");

    fn parse(body: &str) -> Vec<Torrent> {
        ready(
            FitGirl
                .parse(body, &Ask::Words("cyberpunk".to_string()))
                .unwrap(),
        )
    }

    #[test]
    fn a_saved_feed_becomes_repacks() {
        let found = parse(FIXTURE);
        assert_eq!(found.len(), 3);

        let first = &found[0];
        assert_eq!(first.title, "Gunstoppable");
        assert_eq!(first.info_hash, "1dbd47e78b0f7c975c0c5da1499ccf8e99c4f69b");
        assert_eq!(first.size_bytes, parse_size("494 MB"));
        assert_eq!(first.category, Category::Games);
        assert!(first.magnet.contains("&dn="));
        assert!(!first.magnet.contains("&#038;"));
    }

    #[test]
    fn a_selective_install_is_measured_at_its_largest() {
        assert_eq!(
            repack_size("Repack Size: <strong>328/804 MB</strong>"),
            parse_size("804 MB")
        );
        assert_eq!(
            repack_size("Repack Size: <strong>1.7 GB</strong>"),
            parse_size("1.7 GB")
        );
        assert_eq!(repack_size("<p>no size stated</p>"), 0);
    }

    #[test]
    fn a_post_with_no_magnet_link_is_skipped_not_fatal() {
        let feed = "<rss><channel><item><title>Some game</title></item></channel></rss>";
        assert!(parse(feed).is_empty());
    }

    #[test]
    fn a_broken_feed_says_so() {
        assert!(
            FitGirl
                .parse("not xml", &Ask::Browse(Category::Games))
                .is_err()
        );
    }

    #[test]
    fn browsing_reads_the_front_page_feed() {
        assert_eq!(
            FitGirl.urls(&Ask::Browse(Category::Games))[0],
            format!("{HOME}/feed/")
        );
    }
}
