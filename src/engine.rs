//! The only module that knows librqbit exists. Everything above it speaks BAKA types.

use std::fs;
use std::net::Ipv6Addr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
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
    Paused,
    Failed,
}

/// One poll of one torrent. Callers ask for these on a timer rather than subscribing,
/// so nothing above the engine has to keep a channel alive.
#[derive(Debug, Clone)]
pub struct Progress {
    pub name: Option<String>,
    pub folder: PathBuf,
    pub state: State,
    pub finished: bool,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub uploaded_bytes: u64,
    pub download_bps: u64,
    pub upload_bps: u64,
    pub peers: u32,
    pub eta: Option<Duration>,
    pub error: Option<String>,
}

pub struct Engine {
    session: Arc<Session>,
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
        Ok(Self { session })
    }

    pub async fn add(&self, input: &Input) -> Result<DownloadId> {
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
            ..Default::default()
        };

        self.session
            .add_torrent(add, Some(options))
            .await?
            .into_handle()
            .map(|handle| handle.id())
            .context("the torrent was listed but never added")
    }

    pub fn progress(&self, id: DownloadId) -> Option<Progress> {
        self.session.get(id.into()).map(|handle| progress(&handle))
    }

    pub fn snapshot(&self) -> Vec<Progress> {
        self.session
            .with_torrents(|torrents| torrents.map(|(_, handle)| progress(handle)).collect())
    }

    pub async fn pause(&self, id: DownloadId) -> Result<()> {
        let handle = self.session.get(id.into()).context("no such download")?;
        self.session.pause(&handle).await
    }

    pub async fn shutdown(self) {
        self.session.stop().await;
    }
}

fn progress(handle: &ManagedTorrent) -> Progress {
    let stats = handle.stats();
    let live = stats.live.as_ref();
    let download_bps = live.map_or(0, |live| live.download_speed.as_bytes());
    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);

    Progress {
        name: handle.name(),
        // A single file torrent gets no subfolder, and the empty one librqbit joins on
        // leaves a trailing separator that reads like a mistake when it is printed.
        folder: handle.output_folder().components().collect(),
        state: match stats.state {
            TorrentStatsState::Initializing { .. } => State::Checking,
            TorrentStatsState::Live => State::Active,
            TorrentStatsState::Paused => State::Paused,
            TorrentStatsState::Error => State::Failed,
        },
        finished: stats.finished,
        done_bytes: stats.progress_bytes,
        total_bytes: stats.total_bytes,
        uploaded_bytes: stats.uploaded_bytes,
        download_bps,
        upload_bps: live.map_or(0, |live| live.upload_speed.as_bytes()),
        peers: live.map_or(0, |live| live.snapshot.peer_stats.live),
        eta: match stats.finished || download_bps == 0 {
            true => None,
            false => Some(Duration::from_secs(remaining / download_bps)),
        },
        error: stats.error,
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
}
