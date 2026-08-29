//! A source builds URLs and parses bodies. The requests, the timeouts and the ranking
//! live here once, which is why every scraper test runs offline.

mod bittorrented;
mod eztv;
mod fitgirl;
mod nyaa;
mod subsplease;
mod tpb;
mod x1337;
mod yts;

use std::cmp::Reverse;
use std::collections::{HashSet, VecDeque};
use std::fmt;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
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

/// A listing that carries no magnet link costs one more request per result, so only
/// the best seeded handful are followed.
const PAGES: usize = 10;

/// How many searches are worth keeping. A remembered search is one merged answer, so
/// this is a few screens of results rather than anything that needs managing.
const MOST_REMEMBERED: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Category {
    Movies,
    Tv,
    Anime,
    Games,
    Books,
    Audiobooks,
}

impl Category {
    pub const ALL: [Self; 6] = [
        Self::Movies,
        Self::Tv,
        Self::Anime,
        Self::Games,
        Self::Books,
        Self::Audiobooks,
    ];

    fn shelf(self) -> usize {
        Self::ALL.iter().position(|cat| *cat == self).unwrap_or(0)
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Movies => "movies",
            Self::Tv => "tv",
            Self::Anime => "anime",
            Self::Games => "games",
            Self::Books => "books",
            Self::Audiobooks => "audiobooks",
        })
    }
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

/// What a source is being asked for. Words to look up, or the shelf to list when the
/// search box is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Words(String),
    Browse(Category),
}

/// What a source parsed out of a body. A site whose listing carries no magnet link
/// hands back the page the link is on, and the search loop fetches that page for it.
pub enum Found {
    Ready(Torrent),
    OnPage { page: String, torrent: Torrent },
}

pub struct Outcome {
    pub torrents: Vec<Torrent>,
    pub failures: Vec<Failure>,
}

#[derive(Clone)]
pub struct Failure {
    pub source: &'static str,
    pub reason: String,
}

/// What the sources answered. The settings that decide how much of it is shown are
/// applied after, so a remembered answer follows the settings as they are now.
#[derive(Clone)]
struct Answer {
    torrents: Vec<Torrent>,
    failures: Vec<Failure>,
}

/// The same question asked twice is the same question only if the same sources are
/// being asked it.
#[derive(PartialEq)]
struct Question {
    words: String,
    only: Option<Category>,
    sources: Vec<&'static str>,
}

type Searches = Mutex<Vec<(Question, Instant, Answer)>>;

/// Asking eight sites the same thing again because the user pressed Enter twice is
/// eight requests nobody wanted.
static REMEMBERED: LazyLock<Searches> = LazyLock::new(|| Mutex::new(Vec::new()));

pub trait Indexer: Send + Sync {
    fn name(&self) -> &'static str;
    fn categories(&self) -> &'static [Category];
    /// Every address worth trying, in order, until one answers. Empty when a source
    /// has nothing to say: a listing it does not keep, or a query its API refuses.
    fn urls(&self, ask: &Ask) -> Vec<String>;
    fn parse(&self, body: &str, ask: &Ask) -> Result<Vec<Found>>;
}

const INDEXERS: &[&dyn Indexer] = &[
    &yts::Yts,
    &tpb::Tpb,
    &x1337::X1337,
    &bittorrented::BitTorrented,
    &eztv::Eztv,
    &nyaa::Nyaa,
    &subsplease::SubsPlease,
    &fitgirl::FitGirl,
];

/// The Settings page lists one row per source from this, so a source added to
/// `INDEXERS` can be switched off without touching the settings model.
pub fn source_names() -> impl Iterator<Item = &'static str> {
    INDEXERS.iter().map(|indexer| indexer.name())
}

/// An empty query browses instead of searching, which is what an empty search box is
/// asking for. A category narrows either one to the sources that serve it.
pub async fn run(
    settings: &config::Search,
    query: &str,
    only: Option<Category>,
) -> Result<Outcome> {
    let query = query.trim();
    let asked: Vec<(&'static dyn Indexer, Ask)> = asks(query, only)
        .into_iter()
        .filter(|(indexer, _)| settings.sources.enabled(indexer.name()))
        .collect();
    let question = Question {
        words: query.to_lowercase(),
        only,
        sources: asked.iter().map(|(indexer, _)| indexer.name()).collect(),
    };
    let life = Duration::from_secs(u64::from(settings.remember_minutes) * 60);

    let answer = match remembered(&REMEMBERED, &question, life) {
        Some(answer) => answer,
        None => {
            let answer = everyone(settings, &asked).await?;
            remember(&REMEMBERED, question, &answer, life);
            answer
        }
    };

    let torrents = keep(answer.torrents, settings, only);
    Ok(Outcome {
        torrents: match query.is_empty() {
            true => library(torrents, settings),
            false => rank(query, torrents, settings),
        },
        failures: answer.failures,
    })
}

/// Every source at once, each under its own timeout, because the slowest of them is
/// not worth waiting for.
async fn everyone(
    settings: &config::Search,
    asked: &[(&'static dyn Indexer, Ask)],
) -> Result<Answer> {
    let client = Client::builder().user_agent(USER_AGENT).build()?;
    let limit = Duration::from_secs(settings.timeout_secs.into());

    let mut tasks = JoinSet::new();
    for (indexer, ask) in asked {
        let client = client.clone();
        let (indexer, ask) = (*indexer, ask.clone());
        tasks.spawn(async move {
            let name = indexer.name();
            let work = fetch(&client, indexer, &ask);
            match tokio::time::timeout(limit, work).await {
                Ok(result) => (name, result),
                Err(_) => (name, Err(anyhow!("timed out"))),
            }
        });
    }

    let mut torrents = Vec::new();
    let mut failures: Vec<Failure> = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        // A panicking scraper is a bug, but it still must not take the search down.
        let (source, result) = joined.unwrap_or_else(|e| ("unknown", Err(e.into())));
        match result {
            Ok(found) => torrents.extend(found),
            // A source asked for two shelves can fail twice, and the second line
            // tells the reader nothing the first did not.
            Err(e) => {
                if !failures.iter().any(|failed| failed.source == source) {
                    failures.push(Failure {
                        source,
                        reason: format!("{e:#}"),
                    });
                }
            }
        }
    }
    Ok(Answer { torrents, failures })
}

/// A lifetime of nothing is the setting for asking every time.
fn remembered(searches: &Searches, question: &Question, life: Duration) -> Option<Answer> {
    if life.is_zero() {
        return None;
    }
    let mut store = searches.lock().ok()?;
    store.retain(|(_, at, _)| at.elapsed() < life);
    store
        .iter()
        .find(|(asked, _, _)| asked == question)
        .map(|(_, _, answer)| answer.clone())
}

fn remember(searches: &Searches, question: Question, answer: &Answer, life: Duration) {
    if life.is_zero() {
        return;
    }
    let Ok(mut store) = searches.lock() else {
        return;
    };
    store.retain(|(asked, _, _)| *asked != question);
    store.push((question, Instant::now(), answer.clone()));
    if store.len() > MOST_REMEMBERED {
        store.remove(0);
    }
}

/// Which source is asked what. A browse is per category, so a source that serves two
/// of them is asked for both listings.
fn asks(query: &str, only: Option<Category>) -> Vec<(&'static dyn Indexer, Ask)> {
    let mut asks = Vec::new();
    for indexer in INDEXERS {
        let shelves = indexer
            .categories()
            .iter()
            .filter(|cat| only.is_none_or(|wanted| wanted == **cat));

        match query.is_empty() {
            true => asks.extend(shelves.map(|cat| (*indexer, Ask::Browse(*cat)))),
            false => {
                if shelves.count() > 0 {
                    asks.push((*indexer, Ask::Words(query.to_string())));
                }
            }
        }
    }
    asks
}

/// Mirrors are tried in turn because a site that answers here may be blocked there.
async fn fetch(client: &Client, indexer: &dyn Indexer, ask: &Ask) -> Result<Vec<Torrent>> {
    let urls = indexer.urls(ask);
    let mut failure = anyhow!("nothing to ask for this");

    for url in &urls {
        match read(client, url).await {
            Err(e) => failure = e,
            Ok(body) => match indexer.parse(&body, ask) {
                Err(e) => failure = e,
                Ok(found) => return second_pages(client, url, found).await,
            },
        }
    }

    match urls.is_empty() {
        true => Ok(Vec::new()),
        false => Err(failure),
    }
}

async fn read(client: &Client, url: &str) -> Result<String> {
    let body = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(body)
}

/// One result whose page will not load is one result lost, not a failed search.
async fn second_pages(client: &Client, from: &str, found: Vec<Found>) -> Result<Vec<Torrent>> {
    let mut torrents = Vec::new();
    let mut waiting = Vec::new();
    for item in found {
        match item {
            Found::Ready(torrent) => torrents.push(torrent),
            Found::OnPage { page, torrent } => waiting.push((page, torrent)),
        }
    }

    waiting.sort_by_key(|(_, torrent)| Reverse(torrent.seeders));
    waiting.truncate(PAGES);

    let mut jobs = JoinSet::new();
    for (page, torrent) in waiting {
        let client = client.clone();
        let url = beside(from, &page);
        jobs.spawn(async move {
            let magnet = read(&client, &url)
                .await
                .ok()
                .and_then(|body| first_magnet(&body));
            (magnet, torrent)
        });
    }

    while let Some(joined) = jobs.join_next().await {
        let Ok((Some(magnet), mut torrent)) = joined else {
            continue;
        };
        let Some(info_hash) = hex_hash(&magnet) else {
            continue;
        };
        torrent.info_hash = info_hash;
        torrent.magnet = magnet;
        torrents.push(torrent);
    }
    Ok(torrents)
}

/// A page a listing points at is a path on the host that served the listing, which is
/// the one host known to answer for a source with mirrors.
fn beside(from: &str, page: &str) -> String {
    if page.starts_with("http") {
        return page.to_string();
    }
    let Some((scheme, rest)) = from.split_once("://") else {
        return page.to_string();
    };
    let host = rest.split('/').next().unwrap_or(rest);
    format!("{scheme}://{host}{page}")
}

/// The first magnet link in a page. Every site that keeps the link on a page of its
/// own puts it in the markup as it stands.
fn first_magnet(body: &str) -> Option<String> {
    let at = body.find("magnet:?")?;
    let rest = &body[at..];
    let end = rest
        .find(['"', '\'', '<', '>', ' ', '\n', '\r', '\t'])
        .unwrap_or(rest.len());
    Some(unescape(&rest[..end]))
}

/// What a source sent that is allowed on screen at all. A source asked for words
/// answers from every shelf it serves, so the category has to be honoured here too.
fn keep(
    mut torrents: Vec<Torrent>,
    settings: &config::Search,
    only: Option<Category>,
) -> Vec<Torrent> {
    torrents.retain(|torrent| only.is_none_or(|wanted| torrent.category == wanted));
    torrents.retain(|torrent| torrent.seeders >= settings.min_seeders);
    torrents
}

fn rank(query: &str, mut torrents: Vec<Torrent>, settings: &config::Search) -> Vec<Torrent> {
    torrents.sort_by_key(|t| (Reverse(title_matches(query, &t.title)), Reverse(t.seeders)));
    let mut torrents = dedupe(torrents);
    torrents.truncate(settings.result_limit as usize);
    torrents
}

/// A browse has no words to match, so it shows a share of every category rather than
/// letting whichever one has the biggest swarms fill the screen.
fn library(mut torrents: Vec<Torrent>, settings: &config::Search) -> Vec<Torrent> {
    torrents.sort_by_key(|t| Reverse(t.seeders));

    let mut shelves: Vec<VecDeque<Torrent>> =
        Category::ALL.iter().map(|_| VecDeque::new()).collect();
    for torrent in dedupe(torrents) {
        shelves[torrent.category.shelf()].push_back(torrent);
    }

    let limit = settings.result_limit as usize;
    let mut picked = Vec::new();
    while picked.len() < limit && shelves.iter().any(|shelf| !shelf.is_empty()) {
        for shelf in &mut shelves {
            if picked.len() == limit {
                break;
            }
            if let Some(torrent) = shelf.pop_front() {
                picked.push(torrent);
            }
        }
    }
    picked
}

/// Sorted first, so the copy kept for a given infohash is the best seeded one.
fn dedupe(mut torrents: Vec<Torrent>) -> Vec<Torrent> {
    let mut seen = HashSet::new();
    torrents.retain(|torrent| seen.insert(torrent.info_hash.clone()));
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

/// The infohash out of a magnet link, always as hex, so the same torrent offered by
/// two sources collapses into one result whichever form each of them wrote it in.
fn hex_hash(magnet: &str) -> Option<String> {
    let at = magnet.find("urn:btih:")? + "urn:btih:".len();
    let raw: String = magnet[at..]
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();

    if raw.len() == 40 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(raw.to_ascii_lowercase());
    }
    base32(&raw)
}

/// The other form a magnet link writes an infohash in: RFC 4648 base32, which is
/// exactly 32 characters for the 20 bytes of a hash.
fn base32(raw: &str) -> Option<String> {
    if raw.len() != 32 {
        return None;
    }
    let mut bits = 0u32;
    let mut held = 0;
    let mut hex = String::with_capacity(40);
    for letter in raw.to_ascii_uppercase().bytes() {
        let value = match letter {
            b'A'..=b'Z' => letter - b'A',
            b'2'..=b'7' => letter - b'2' + 26,
            _ => return None,
        };
        bits = (bits << 5) | u32::from(value);
        held += 5;
        if held >= 8 {
            held -= 8;
            hex.push_str(&format!("{:02x}", (bits >> held) as u8));
        }
    }
    Some(hex)
}

pub fn encode(input: &str) -> String {
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

/// HTML entities, which the sites BAKA reads leave in titles and in the magnet links
/// they print. The XML parsers handle their own; this is for markup read as text.
fn unescape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|end| *end <= 8) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        rest = &rest[end + 1..];
        match named(entity).or_else(|| numbered(entity)) {
            Some(letter) => out.push(letter),
            None => out.push_str(&format!("&{entity};")),
        }
    }
    out.push_str(rest);
    out
}

fn named(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => None,
    }
}

fn numbered(entity: &str) -> Option<char> {
    let digits = entity.strip_prefix('#')?;
    let code = match digits.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => digits.parse().ok()?,
    };
    char::from_u32(code)
}

/// Every site BAKA reads counts in powers of 1024 whichever suffix it prints, so
/// "812.8 MiB" and "10.6 GB" mean the same thing.
fn parse_size(text: &str) -> u64 {
    let text = text.trim();
    let digits: String = text
        .chars()
        .take_while(|letter| letter.is_ascii_digit() || *letter == '.')
        .collect();
    let Ok(amount) = digits.parse::<f64>() else {
        return 0;
    };
    let unit = text[digits.len()..].trim_start().to_ascii_uppercase();
    let scale: f64 = match unit.as_bytes().first() {
        Some(b'K') => 1024.0,
        Some(b'M') => 1024.0 * 1024.0,
        Some(b'G') => 1024.0 * 1024.0 * 1024.0,
        Some(b'T') => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        _ => 1.0,
    };
    (amount * scale) as u64
}

/// A scraper test reads the torrents its source parsed. Fetching the second page a
/// listing points at is the search loop's job, not the scraper's.
#[cfg(test)]
fn ready(found: Vec<Found>) -> Vec<Torrent> {
    found
        .into_iter()
        .filter_map(|item| match item {
            Found::Ready(torrent) => Some(torrent),
            Found::OnPage { .. } => None,
        })
        .collect()
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

    fn shelved(category: Category, seeders: u32, info_hash: &str) -> Torrent {
        Torrent {
            category,
            ..torrent("Something", seeders, info_hash)
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
        let kept = keep(found, &picky, None);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].seeders, 3);
    }

    #[test]
    fn a_source_that_serves_two_shelves_only_shows_the_one_asked_for() {
        let found = vec![
            shelved(Category::Movies, 9, "a"),
            shelved(Category::Tv, 9, "b"),
        ];
        let kept = keep(found, &settings(), Some(Category::Tv));
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].category, Category::Tv);
    }

    #[test]
    fn a_browse_shows_every_shelf_rather_than_the_biggest_swarms() {
        let found = vec![
            shelved(Category::Movies, 900, "a"),
            shelved(Category::Movies, 800, "b"),
            shelved(Category::Movies, 700, "c"),
            shelved(Category::Games, 0, "d"),
            shelved(Category::Anime, 1, "e"),
        ];
        let small = config::Search {
            result_limit: 4,
            ..settings()
        };
        let shown = library(found, &small);
        let categories: Vec<Category> = shown.iter().map(|t| t.category).collect();
        assert_eq!(
            categories,
            vec![
                Category::Movies,
                Category::Anime,
                Category::Games,
                Category::Movies
            ]
        );
    }

    #[test]
    fn every_source_is_asked_once_for_a_search_and_once_per_shelf_to_browse() {
        let searching = asks("dune", None);
        assert_eq!(searching.len(), INDEXERS.len());

        let browsing = asks("", None);
        let shelves: usize = INDEXERS
            .iter()
            .map(|indexer| indexer.categories().len())
            .sum();
        assert_eq!(browsing.len(), shelves);
    }

    #[test]
    fn a_category_only_asks_the_sources_that_serve_it() {
        for (indexer, ask) in asks("dune", Some(Category::Games)) {
            assert!(indexer.categories().contains(&Category::Games));
            assert_eq!(ask, Ask::Words("dune".to_string()));
        }
        for (_, ask) in asks("", Some(Category::Anime)) {
            assert_eq!(ask, Ask::Browse(Category::Anime));
        }
    }

    fn question(words: &str) -> Question {
        Question {
            words: words.to_string(),
            only: None,
            sources: vec!["test"],
        }
    }

    fn answer(title: &str) -> Answer {
        Answer {
            torrents: vec![torrent(title, 1, "a")],
            failures: Vec::new(),
        }
    }

    #[test]
    fn a_repeated_search_is_answered_without_asking_anyone() {
        let searches = Searches::default();
        let life = Duration::from_secs(60);
        remember(&searches, question("dune"), &answer("Dune Part Two"), life);
        let found = remembered(&searches, &question("dune"), life).unwrap();
        assert_eq!(found.torrents[0].title, "Dune Part Two");
    }

    #[test]
    fn a_different_question_is_put_to_the_sources_afresh() {
        let searches = Searches::default();
        let life = Duration::from_secs(60);
        remember(&searches, question("dune"), &answer("Dune Part Two"), life);
        assert!(remembered(&searches, &question("arrival"), life).is_none());

        // The same words asked of different sources is a different question.
        let elsewhere = Question {
            sources: vec!["somewhere else"],
            ..question("dune")
        };
        assert!(remembered(&searches, &elsewhere, life).is_none());
    }

    #[test]
    fn a_stale_answer_is_forgotten_rather_than_shown() {
        let searches = Searches::default();
        let life = Duration::from_millis(20);
        remember(&searches, question("dune"), &answer("Dune Part Two"), life);
        std::thread::sleep(Duration::from_millis(40));
        assert!(remembered(&searches, &question("dune"), life).is_none());
        assert!(searches.lock().unwrap().is_empty());
    }

    #[test]
    fn nothing_is_remembered_when_the_setting_says_never() {
        let searches = Searches::default();
        remember(&searches, question("dune"), &answer("Dune"), Duration::ZERO);
        assert!(searches.lock().unwrap().is_empty());
        assert!(remembered(&searches, &question("dune"), Duration::ZERO).is_none());
    }

    #[test]
    fn only_the_last_handful_of_searches_are_kept() {
        let searches = Searches::default();
        let life = Duration::from_secs(60);
        for n in 0..MOST_REMEMBERED + 5 {
            remember(
                &searches,
                question(&format!("query {n}")),
                &answer("x"),
                life,
            );
        }
        assert_eq!(searches.lock().unwrap().len(), MOST_REMEMBERED);
        // The oldest are the ones that went.
        assert!(remembered(&searches, &question("query 0"), life).is_none());
        let last = format!("query {}", MOST_REMEMBERED + 4);
        assert!(remembered(&searches, &question(&last), life).is_some());
    }

    #[test]
    fn asking_the_same_thing_twice_keeps_one_answer_not_two() {
        let searches = Searches::default();
        let life = Duration::from_secs(60);
        remember(&searches, question("dune"), &answer("First"), life);
        remember(&searches, question("dune"), &answer("Second"), life);
        assert_eq!(searches.lock().unwrap().len(), 1);
        let found = remembered(&searches, &question("dune"), life).unwrap();
        assert_eq!(found.torrents[0].title, "Second");
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
    fn a_base32_hash_and_a_hex_one_come_out_the_same() {
        let hex = hex_hash("magnet:?xt=urn:btih:CAB507494D02EBB1178B38F2E9D7BE299C86B862&dn=x");
        let base32 = hex_hash("magnet:?xt=urn:btih:ZK2QOSKNALV3CF4LHDZOTV56FGOINODC&dn=x");
        assert_eq!(
            hex.as_deref(),
            Some("cab507494d02ebb1178b38f2e9d7be299c86b862")
        );
        assert_eq!(base32, hex);
    }

    #[test]
    fn a_magnet_with_no_hash_in_it_is_not_a_magnet() {
        assert_eq!(hex_hash("magnet:?dn=no+hash+here"), None);
        assert_eq!(hex_hash("magnet:?xt=urn:btih:tooshort"), None);
    }

    #[test]
    fn the_first_magnet_on_a_page_is_the_one_a_result_points_at() {
        let page = r#"<a href="magnet:?xt=urn:btih:abc&#038;dn=Some+Name">get</a>"#;
        assert_eq!(
            first_magnet(page).as_deref(),
            Some("magnet:?xt=urn:btih:abc&dn=Some+Name")
        );
        assert_eq!(first_magnet("<p>no link here</p>"), None);
    }

    #[test]
    fn a_second_page_is_looked_for_on_the_host_that_answered() {
        let listing = "https://mirror.example/search/dune/1/";
        assert_eq!(
            beside(listing, "/torrent/123/dune/"),
            "https://mirror.example/torrent/123/dune/"
        );
        assert_eq!(
            beside(listing, "https://elsewhere.example/x"),
            "https://elsewhere.example/x"
        );
    }

    #[test]
    fn entities_come_back_as_the_characters_they_stand_for() {
        assert_eq!(unescape("Tom &amp; Jerry"), "Tom & Jerry");
        assert_eq!(unescape("a&#038;b"), "a&b");
        assert_eq!(unescape("a&#x26;b"), "a&b");
        assert_eq!(unescape("100% &unknown; safe"), "100% &unknown; safe");
        assert_eq!(unescape("no entities"), "no entities");
    }

    #[test]
    fn sizes_read_the_way_a_person_would_say_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3_285_649_981), "3.1 GiB");
    }

    #[test]
    fn sizes_come_back_in_bytes_whichever_suffix_a_site_prints() {
        assert_eq!(parse_size("812.8 MiB"), 852_282_572);
        assert_eq!(parse_size("1.5 GiB"), 1_610_612_736);
        assert_eq!(parse_size("3 GB"), 3_221_225_472);
        assert_eq!(parse_size("nonsense"), 0);
    }

    #[test]
    fn every_source_answers_for_the_shelves_it_claims() {
        for indexer in INDEXERS {
            assert!(
                !indexer.categories().is_empty(),
                "{} serves nothing",
                indexer.name()
            );
            for category in indexer.categories() {
                let urls = indexer.urls(&Ask::Browse(*category));
                let listed = urls.iter().all(|url| url.starts_with("https://"));
                assert!(listed, "{} browses somewhere odd", indexer.name());
            }
        }
    }

    #[test]
    fn no_two_sources_share_a_name() {
        let names: HashSet<&str> = source_names().collect();
        assert_eq!(names.len(), INDEXERS.len());
    }
}
