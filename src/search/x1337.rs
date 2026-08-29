use anyhow::{Result, bail};

use super::{Ask, Category, Found, Indexer, Torrent, encode, parse_size, title_matches, unescape};

pub struct X1337;

/// Which host answers depends on where you are: some networks reach one of these and
/// are challenged by the rest, so every one of them is worth trying in turn.
const HOSTS: [&str; 4] = [
    "https://1337x.to",
    "https://www.1377x.to",
    "https://1337x.st",
    "https://x1337x.ws",
];

impl Indexer for X1337 {
    fn name(&self) -> &'static str {
        "1337x"
    }

    fn categories(&self) -> &'static [Category] {
        &[
            Category::Movies,
            Category::Tv,
            Category::Books,
            Category::Audiobooks,
        ]
    }

    fn urls(&self, ask: &Ask) -> Vec<String> {
        let path = match ask {
            // The site writes spaces in a search path as plus signs.
            Ask::Words(query) => format!("/search/{}/1/", encode(query).replace("%20", "+")),
            Ask::Browse(Category::Tv) => "/popular-tv".to_string(),
            // The site keeps no popular page for what it files under Other, and its
            // own E-Books listing comes back empty, so the shelf below it is the one
            // that answers. Rows from the rest of Other are dropped by `shelf`.
            Ask::Browse(Category::Books) => "/cat/Other/1/".to_string(),
            Ask::Browse(Category::Audiobooks) => "/sub/other/Audiobook/1/".to_string(),
            Ask::Browse(_) => "/popular-movies".to_string(),
        };
        HOSTS.iter().map(|host| format!("{host}{path}")).collect()
    }

    fn parse(&self, body: &str, ask: &Ask) -> Result<Vec<Found>> {
        let Some(table) = body.find("table-list") else {
            // Every host serves the same table. A body without one is a block page.
            bail!("1337x answered with something other than a result table");
        };

        let mut found = Vec::new();
        for row in body[table..].split("<tr").skip(1) {
            if let Some(result) = torrent(row, ask) {
                found.push(result);
            }
        }
        Ok(found)
    }
}

/// A row carries everything but the magnet link, which lives on the result's own page.
fn torrent(row: &str, ask: &Ask) -> Option<Found> {
    let category = shelf(field(row, "href=\"/sub/", "\"")?)?;
    let (page, title) = field(row, "href=\"/torrent/", "</a>")?.split_once("\">")?;
    let title = unescape(title.trim());

    // The site answers a phrase with anything carrying one of its words, and a row
    // missing the rest is not worth the second request its magnet would cost.
    if let Ask::Words(query) = ask
        && !title_matches(query, &title)
    {
        return None;
    }

    Some(Found::OnPage {
        page: format!("/torrent/{page}"),
        torrent: Torrent {
            title,
            size_bytes: cell(row, "class=\"coll-4 size").map_or(0, parse_size),
            seeders: count(row, "class=\"coll-2 seeds\">"),
            leechers: count(row, "class=\"coll-3 leeches\">"),
            category,
            source: "1337x",
            // Both arrive with the page, which the search loop fetches.
            info_hash: String::new(),
            magnet: String::new(),
        },
    })
}

/// The icon link reads `/sub/<section>/<shelf>/<page>/`, and everything written is
/// filed under one section, so books are told apart by the second name rather than
/// the first.
fn shelf(link: &str) -> Option<Category> {
    let mut names = link.split('/');
    match (names.next()?, names.next()?) {
        ("movies", _) => Some(Category::Movies),
        ("tv", _) => Some(Category::Tv),
        ("other", "E-Books" | "Comics") => Some(Category::Books),
        ("other", "Audiobook") => Some(Category::Audiobooks),
        _ => None,
    }
}

fn field<'a>(row: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let at = row.find(start)? + start.len();
    let rest = &row[at..];
    let to = rest.find(end)?;
    Some(&rest[..to])
}

/// The size cell keeps a second class name after the one worth matching on, so the
/// text is whatever follows the end of the opening tag.
fn cell<'a>(row: &'a str, start: &str) -> Option<&'a str> {
    let (_, text) = field(row, start, "</td>")?.split_once('>')?;
    Some(text)
}

fn count(row: &str, start: &str) -> u32 {
    field(row, start, "<")
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = include_str!("fixtures/1337x.html");
    const PAGE: &str = include_str!("fixtures/1337x-detail.html");
    const BOOKS: &str = include_str!("fixtures/1337x-books.html");

    fn pending(body: &str) -> Vec<(String, Torrent)> {
        X1337
            .parse(body, &Ask::Words("dune".to_string()))
            .unwrap()
            .into_iter()
            .filter_map(|item| match item {
                Found::OnPage { page, torrent } => Some((page, torrent)),
                Found::Ready(_) => None,
            })
            .collect()
    }

    #[test]
    fn a_listing_becomes_results_that_still_need_their_page() {
        let found = pending(LISTING);
        assert_eq!(found.len(), 4);

        let (page, torrent) = &found[0];
        assert_eq!(
            page,
            "/torrent/5020920/Dune-2021-1080p-WEBRip-DD5-1-x264-SHITBOX/"
        );
        assert_eq!(torrent.title, "Dune.2021.1080p.WEBRip.DD5.1.x264-SHITBOX");
        assert_eq!(torrent.seeders, 7312);
        assert_eq!(torrent.leechers, 899);
        assert_eq!(torrent.size_bytes, 11_381_663_334);
        assert_eq!(torrent.category, Category::Movies);
        assert!(torrent.magnet.is_empty());
    }

    #[test]
    fn written_and_spoken_are_told_apart_by_the_shelf_under_other() {
        let found: Vec<Torrent> = X1337
            .parse(BOOKS, &Ask::Words("audiobook".to_string()))
            .unwrap()
            .into_iter()
            .filter_map(|item| match item {
                Found::OnPage { torrent, .. } => Some(torrent),
                Found::Ready(_) => None,
            })
            .collect();
        assert_eq!(found.len(), 3);

        assert_eq!(found[0].title, "Terry Pratchett Audiobook Collection");
        assert_eq!(found[0].category, Category::Audiobooks);
        assert_eq!(found[0].seeders, 196);
        assert_eq!(found[0].size_bytes, 25_662_429_593);
        assert_eq!(found[1].category, Category::Books);
        assert_eq!(found[2].category, Category::Audiobooks);
    }

    #[test]
    fn the_page_a_result_points_at_carries_the_magnet_link() {
        let magnet = crate::search::first_magnet(PAGE).unwrap();
        assert!(magnet.starts_with("magnet:?xt=urn:btih:4D165EAE"));
        assert_eq!(
            crate::search::hex_hash(&magnet).as_deref(),
            Some("4d165eae3c3f1c8fcd467e7a9b21add164d6e969")
        );
    }

    #[test]
    fn a_challenge_page_is_a_failure_rather_than_an_empty_search() {
        assert!(
            X1337
                .parse(
                    "<html><title>Just a moment</title></html>",
                    &Ask::Words("x".into())
                )
                .is_err()
        );
    }

    #[test]
    fn a_shelf_this_source_does_not_serve_is_left_alone() {
        let row = r#"<td class="coll-1 name"><a href="/sub/games/PC/1/" class="icon"></a>
            <a href="/torrent/1/Some-Game/">Some Game</a></td>"#;
        assert!(torrent(row, &Ask::Browse(Category::Movies)).is_none());
    }

    #[test]
    fn every_mirror_is_offered_for_both_a_search_and_a_browse() {
        assert_eq!(
            X1337.urls(&Ask::Words("dune part two".into())).len(),
            HOSTS.len()
        );
        assert!(
            X1337.urls(&Ask::Words("dune part two".into()))[0]
                .ends_with("/search/dune+part+two/1/")
        );
        assert!(X1337.urls(&Ask::Browse(Category::Tv))[0].ends_with("/popular-tv"));
        assert!(X1337.urls(&Ask::Browse(Category::Books))[0].ends_with("/cat/Other/1/"));
        assert!(
            X1337.urls(&Ask::Browse(Category::Audiobooks))[0].ends_with("/sub/other/Audiobook/1/")
        );
    }
}
