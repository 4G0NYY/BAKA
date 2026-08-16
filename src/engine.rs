//! The only module that knows librqbit exists. Everything above it speaks BAKA types.

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::net::Ipv6Addr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use librqbit::dht::DhtPersistenceConfig;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, DhtSessionConfig, ListenerOptions, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};
use thiserror::Error;

use crate::config::{self, Settings};
use crate::search::magnet_link;

pub type DownloadId = usize;

/// What `baka get` accepts, once it has been made sense of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Magnet(String),
    File(PathBuf),
}

#[derive(Debug, Error)]
#[error("{0} is not a magnet link, an infohash or a file that exists")]
pub struct UnrecognisedInput(String);

impl Input {
    pub fn parse(raw: &str) -> Result<Self, UnrecognisedInput> {
        let raw = raw.trim();
        if raw.starts_with("magnet:") {
            return Ok(Self::Magnet(raw.to_string()));
        }
        if let Some(hash) = info_hash(raw) {
            return Ok(Self::Magnet(magnet_link(&hash, "")));
        }
        if Path::new(raw).is_file() {
            return Ok(Self::File(PathBuf::from(raw)));
        }
        Err(UnrecognisedInput(raw.to_string()))
    }
}

/// The two shapes a site hands you when it does not hand you a magnet link. Hex is
/// normalised lowercase and base32 uppercase because that is what each decoder wants.
fn info_hash(raw: &str) -> Option<String> {
    if raw.len() == 40 && raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(raw.to_ascii_lowercase());
    }
    let upper = raw.to_ascii_uppercase();
    if upper.len() == 32
        && upper
            .bytes()
            .all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b))
    {
        return Some(upper);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Checking,
    Active,
    Queued,
    Paused,
    Failed,
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Checking => "checking",
            Self::Active => "running",
            Self::Queued => "queued",
            Self::Paused => "paused",
            Self::Failed => "failed",
        })
    }
}

/// Whether the user wants this torrent running. The queue may stop a torrent it is
/// still waiting to run, and only this says whether it may start it again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wish {
    Run,
    Hold,
}

/// One poll of one torrent. Callers ask for these on a timer rather than subscribing,
/// so nothing above the engine has to keep a channel alive.
#[derive(Debug, Clone)]
pub struct Progress {
    pub id: DownloadId,
    pub name: Option<String>,
    pub info_hash: String,
    pub folder: PathBuf,
    pub state: State,
    pub finished: bool,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub peers: u32,
    pub ratio: f32,
    pub eta: Option<Duration>,
    pub error: Option<String>,
}

pub struct Engine {
    session: Arc<Session>,
    wishes: Mutex<HashMap<DownloadId, Wish>>,
}

impl Engine {
    pub async fn start(settings: &Settings) -> Result<Self> {
        let state = config::state_dir()?;
        let options = SessionOptions {
            dht: settings.network.dht.then(|| DhtSessionConfig {
                persistence: Some(DhtPersistenceConfig {
                    config_filename: Some(state.join("dht.json")),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            fastresume: true,
            persistence: Some(SessionPersistenceConfig::Json {
                folder: Some(state),
            }),
            listen: Some(ListenerOptions {
                listen_addr: (Ipv6Addr::UNSPECIFIED, settings.network.listen_port).into(),
                enable_upnp_port_forwarding: settings.network.upnp,
                ..Default::default()
            }),
            peer_limit: Some(settings.network.max_peers_per_torrent as usize),
            ratelimits: LimitsConfig {
                download_bps: bytes_per_second(settings.downloads.rate_limit_kib),
                upload_bps: bytes_per_second(settings.seeding.rate_limit_kib),
            },
            client_name_and_version: Some(concat!("BAKA ", env!("CARGO_PKG_VERSION")).to_string()),
            ..Default::default()
        };

        let session = Session::new_with_opts(settings.downloads.folder.clone(), options)
            .await
            .context("could not start the torrent engine")?;
        Ok(Self {
            session,
            wishes: Mutex::new(HashMap::new()),
        })
    }

    pub async fn add(&self, input: &Input, folder: &Path) -> Result<DownloadId> {
        let add = match input {
            Input::Magnet(link) => AddTorrent::from_url(link.as_str()),
            Input::File(path) => AddTorrent::from_bytes(
                fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
            ),
        };

        let options = AddTorrentOptions {
            // Without this librqbit refuses to touch files it did not create, which means
            // a resumed or an already finished torrent can neither continue nor seed.
            overwrite: true,
            // Naming the folder on every add is what lets the setting change without a
            // restart, since the session only reads its own default once.
            output_folder: Some(folder.display().to_string()),
            ..Default::default()
        };

        let id = self
            .session
            .add_torrent(add, Some(options))
            .await?
            .into_handle()
            .map(|handle| handle.id())
            .context("the torrent was listed but never added")?;

        self.wish(id, Wish::Run);
        Ok(id)
    }

    pub fn progress(&self, id: DownloadId) -> Option<Progress> {
        let held = self.held(id);
        self.session
            .get(id.into())
            .map(|handle| progress(id, &handle, held))
    }

    pub fn snapshot(&self) -> Vec<Progress> {
        let mut found: Vec<Progress> = self.session.with_torrents(|torrents| {
            torrents
                .map(|(id, handle)| progress(id, handle, self.held(id)))
                .collect()
        });
        found.sort_by_key(|entry| entry.id);
        found
    }

    /// Pausing by hand outranks the queue, so the queue will not start it again.
    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        self.wish(id, Wish::Hold);
        self.set_paused(id, true).await
    }

    pub async fn resume(&self, id: DownloadId) -> Result<()> {
        self.wish(id, Wish::Run);
        self.set_paused(id, false).await
    }

    /// Files stay on disk. Nothing in BAKA deletes what a user already downloaded.
    pub async fn remove(&self, id: DownloadId) -> Result<()> {
        self.wishes.lock().expect("wishes").remove(&id);
        self.session.delete(id.into(), false).await
    }

    /// The settings the session reads afresh every time. The rest say so on their row.
    pub fn apply(&self, settings: &Settings) {
        let limits = &self.session.ratelimits;
        limits.set_download_bps(bytes_per_second(settings.downloads.rate_limit_kib));
        limits.set_upload_bps(bytes_per_second(settings.seeding.rate_limit_kib));
    }

    /// Runs the concurrency limits and the ratio cap. Called on a timer, because a
    /// torrent finishing is what pushes it from the download queue into the seed queue.
    pub async fn enforce(&self, settings: &Settings) -> Result<()> {
        let slots = self.slots();
        for order in plan(&slots, settings) {
            match order {
                Order::Start(id) => self.set_paused(id, false).await?,
                Order::Wait(id) => self.set_paused(id, true).await?,
                Order::Stop(id) => {
                    self.wish(id, Wish::Hold);
                    self.set_paused(id, true).await?;
                }
            }
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.session.stop().await;
    }

    /// A torrent restored from the last run keeps the state it was left in, so quitting
    /// with something paused does not hand it back running.
    fn slots(&self) -> Vec<Slot> {
        let mut slots: Vec<Slot> =
            self.session.with_torrents(|torrents| {
                torrents
                    .map(|(id, handle)| {
                        let stats = handle.stats();
                        let paused = matches!(stats.state, TorrentStatsState::Paused);
                        let wish = *self.wishes.lock().expect("wishes").entry(id).or_insert(
                            match paused {
                                true => Wish::Hold,
                                false => Wish::Run,
                            },
                        );
                        Slot {
                            id,
                            finished: stats.finished,
                            paused,
                            failed: matches!(stats.state, TorrentStatsState::Error),
                            held: wish == Wish::Hold,
                            ratio: ratio(stats.uploaded_bytes, stats.total_bytes),
                        }
                    })
                    .collect()
            });
        slots.sort_by_key(|slot| slot.id);
        slots
    }

    fn wish(&self, id: DownloadId, wish: Wish) {
        self.wishes.lock().expect("wishes").insert(id, wish);
    }

    fn held(&self, id: DownloadId) -> bool {
        self.wishes.lock().expect("wishes").get(&id) == Some(&Wish::Hold)
    }

    /// librqbit treats pausing something already paused as an error, so the current
    /// state decides whether there is anything to do.
    async fn set_paused(&self, id: DownloadId, paused: bool) -> Result<()> {
        let handle = self.session.get(id.into()).context("no such download")?;
        if handle.is_paused() == paused {
            return Ok(());
        }
        match paused {
            true => self.session.pause(&handle).await,
            false => self.session.unpause(&handle).await,
        }
    }
}

/// What the queue needs to know about one torrent, and nothing librqbit shaped.
#[derive(Debug, Clone, Copy)]
struct Slot {
    id: DownloadId,
    finished: bool,
    paused: bool,
    failed: bool,
    held: bool,
    ratio: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    Start(DownloadId),
    Wait(DownloadId),
    Stop(DownloadId),
}

/// Oldest first, because a queue that reshuffles itself never finishes anything.
fn plan(slots: &[Slot], settings: &Settings) -> Vec<Order> {
    let cap = settings.seeding.stop_at_ratio;
    let mut orders = Vec::new();
    let mut downloading = 0;
    let mut seeding = 0;

    for slot in slots {
        if slot.held || slot.failed {
            continue;
        }

        if slot.finished {
            let done_sharing =
                !settings.seeding.after_completion || (cap > 0.0 && slot.ratio >= cap);
            if done_sharing {
                orders.push(Order::Stop(slot.id));
                continue;
            }
            seeding += 1;
            orders.extend(schedule(slot, seeding <= settings.seeding.max_concurrent));
        } else {
            downloading += 1;
            orders.extend(schedule(
                slot,
                downloading <= settings.downloads.max_concurrent,
            ));
        }
    }

    orders
}

fn schedule(slot: &Slot, allowed: bool) -> Option<Order> {
    match (allowed, slot.paused) {
        (true, true) => Some(Order::Start(slot.id)),
        (false, false) => Some(Order::Wait(slot.id)),
        _ => None,
    }
}

fn progress(id: DownloadId, handle: &ManagedTorrent, held: bool) -> Progress {
    let stats = handle.stats();
    let live = stats.live.as_ref();
    let download_bps = live.map_or(0, |live| live.download_speed.as_bytes());
    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);

    Progress {
        id,
        name: handle.name(),
        info_hash: handle.info_hash().as_string(),
        // A single file torrent gets no subfolder, and the empty one librqbit joins on
        // leaves a trailing separator that reads like a mistake when it is printed.
        folder: handle.output_folder().components().collect(),
        state: match stats.state {
            TorrentStatsState::Initializing { .. } => State::Checking,
            TorrentStatsState::Live => State::Active,
            TorrentStatsState::Paused if held => State::Paused,
            TorrentStatsState::Paused => State::Queued,
            TorrentStatsState::Error => State::Failed,
        },
        finished: stats.finished,
        done_bytes: stats.progress_bytes,
        total_bytes: stats.total_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        download_bps,
        upload_bps: live.map_or(0, |live| live.upload_speed.as_bytes()),
        peers: live.map_or(0, |live| live.snapshot.peer_stats.live),
        ratio: ratio(stats.uploaded_bytes, stats.total_bytes),
        eta: match stats.finished || download_bps == 0 {
            true => None,
            false => Some(Duration::from_secs(remaining / download_bps)),
        },
        error: stats.error,
    }
}

pub fn human_eta(eta: Option<Duration>) -> String {
    let Some(eta) = eta else {
        return "unknown".to_string();
    };
    let seconds = eta.as_secs();
    match (seconds / 3600, seconds / 60 % 60, seconds % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

/// A torrent whose size is not known yet has shared nothing measurable, and dividing
/// by that size would be a crash rather than a number.
fn ratio(uploaded: u64, total: u64) -> f32 {
    match total {
        0 => 0.0,
        total => uploaded as f32 / total as f32,
    }
}

/// Settings speak KiB/s because that is what a person reads. Zero is no limit at all.
fn bytes_per_second(kib: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(kib.saturating_mul(1024))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_magnet_link_is_taken_as_it_stands() {
        let link = "magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862";
        assert_eq!(Input::parse(link).unwrap(), Input::Magnet(link.to_string()));
    }

    #[test]
    fn a_bare_hex_infohash_becomes_a_magnet_with_trackers() {
        let Ok(Input::Magnet(link)) = Input::parse("CAB507494D02EBB1178B38F2E9D7BE299C86B862")
        else {
            panic!("a hex infohash should parse");
        };
        assert!(link.starts_with("magnet:?xt=urn:btih:cab507494d02ebb1178b38f2e9d7be299c86b862"));
        assert!(link.contains("&tr="));
    }

    #[test]
    fn a_bare_base32_infohash_keeps_the_case_its_decoder_wants() {
        let Ok(Input::Magnet(link)) = Input::parse("zk2qospnaltlcf43hd5o5v56focynocc") else {
            panic!("a base32 infohash should parse");
        };
        assert!(link.starts_with("magnet:?xt=urn:btih:ZK2QOSPNALTLCF43HD5O5V56FOCYNOCC"));
    }

    #[test]
    fn surrounding_whitespace_from_a_paste_is_ignored() {
        let pasted = "  cab507494d02ebb1178b38f2e9d7be299c86b862\n";
        assert!(matches!(Input::parse(pasted), Ok(Input::Magnet(_))));
    }

    #[test]
    fn an_existing_file_is_taken_as_a_torrent_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let raw = path.to_str().unwrap();
        assert_eq!(Input::parse(raw).unwrap(), Input::File(path.clone()));
    }

    #[test]
    fn anything_else_says_so_rather_than_guessing() {
        assert!(Input::parse("dune part two").is_err());
        assert!(Input::parse("not/a/file.torrent").is_err());
        // Forty characters, but g is not a hex digit.
        assert!(Input::parse("gab507494d02ebb1178b38f2e9d7be299c86b862").is_err());
    }

    #[test]
    fn a_rate_limit_of_zero_means_no_limiter_at_all() {
        assert_eq!(bytes_per_second(0), None);
        assert_eq!(bytes_per_second(500), NonZeroU32::new(512_000));
    }

    fn slot(id: DownloadId) -> Slot {
        Slot {
            id,
            finished: false,
            paused: false,
            failed: false,
            held: false,
            ratio: 0.0,
        }
    }

    fn settings() -> Settings {
        let mut settings = Settings::default();
        settings.downloads.max_concurrent = 2;
        settings.seeding.max_concurrent = 1;
        settings
    }

    #[test]
    fn downloads_past_the_limit_are_told_to_wait() {
        let slots = [slot(1), slot(2), slot(3)];
        assert_eq!(plan(&slots, &settings()), vec![Order::Wait(3)]);
    }

    #[test]
    fn a_queued_download_starts_when_a_slot_opens() {
        let mut waiting = slot(2);
        waiting.paused = true;
        let slots = [slot(1), waiting];
        assert_eq!(plan(&slots, &settings()), vec![Order::Start(2)]);
    }

    #[test]
    fn the_oldest_torrent_keeps_its_slot() {
        let mut first = slot(1);
        first.paused = true;
        let mut second = slot(2);
        second.paused = true;
        let slots = [first, second, slot(3)];
        // Two slots. The pair that gets them is by age, so the newest gives one up
        // even though it is the one already running.
        assert_eq!(
            plan(&slots, &settings()),
            vec![Order::Start(1), Order::Start(2), Order::Wait(3)]
        );
    }

    #[test]
    fn a_paused_torrent_is_left_alone_by_the_queue() {
        let mut held = slot(1);
        held.held = true;
        held.paused = true;
        assert!(plan(&[held], &settings()).is_empty());
    }

    #[test]
    fn a_failed_torrent_is_never_restarted() {
        let mut broken = slot(1);
        broken.failed = true;
        broken.paused = true;
        assert!(plan(&[broken], &settings()).is_empty());
    }

    #[test]
    fn downloads_and_seeds_have_separate_limits() {
        let mut seed = slot(1);
        seed.finished = true;
        let mut second_seed = slot(2);
        second_seed.finished = true;
        let slots = [seed, second_seed, slot(3), slot(4)];
        // One seed slot and two download slots, so only the second seed waits.
        assert_eq!(plan(&slots, &settings()), vec![Order::Wait(2)]);
    }

    #[test]
    fn seeding_stops_once_the_ratio_is_reached() {
        let mut seed = slot(1);
        seed.finished = true;
        seed.ratio = 2.0;
        let mut settings = settings();
        settings.seeding.stop_at_ratio = 2.0;
        assert_eq!(plan(&[seed], &settings), vec![Order::Stop(1)]);
    }

    #[test]
    fn a_ratio_of_zero_seeds_forever() {
        let mut seed = slot(1);
        seed.finished = true;
        seed.ratio = 99.0;
        assert!(plan(&[seed], &settings()).is_empty());
    }

    #[test]
    fn seeding_switched_off_stops_a_torrent_the_moment_it_finishes() {
        let mut seed = slot(1);
        seed.finished = true;
        let mut settings = settings();
        settings.seeding.after_completion = false;
        assert_eq!(plan(&[seed], &settings), vec![Order::Stop(1)]);
    }

    #[test]
    fn an_eta_reads_the_way_a_person_would_say_it() {
        assert_eq!(human_eta(None), "unknown");
        assert_eq!(human_eta(Some(Duration::from_secs(45))), "45s");
        assert_eq!(human_eta(Some(Duration::from_secs(125))), "2m 5s");
        assert_eq!(human_eta(Some(Duration::from_secs(7300))), "2h 1m");
    }

    #[test]
    fn a_torrent_with_no_size_yet_has_no_ratio_to_divide_by() {
        assert_eq!(ratio(0, 0), 0.0);
        assert_eq!(ratio(512, 1024), 0.5);
    }
}
