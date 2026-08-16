//! Every persistent setting BAKA has. One struct, one file, one page in the TUI.
//! Nothing else in the codebase may read the environment for configuration.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use directories::{BaseDirs, UserDirs};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("this user has no config directory")]
    NoConfigDir,
    #[error("could not read {0}")]
    Read(PathBuf, #[source] io::Error),
    #[error("could not write {0}")]
    Write(PathBuf, #[source] io::Error),
    #[error("{0} is not valid TOML")]
    Parse(PathBuf, #[source] toml::de::Error),
    #[error("could not encode settings as TOML")]
    Encode(#[source] toml::ser::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub downloads: Downloads,
    pub seeding: Seeding,
    pub network: Network,
    pub search: Search,
    pub server: Server,
    pub interface: Interface,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Downloads {
    pub folder: PathBuf,
    pub max_concurrent: u32,
    pub rate_limit_kib: u32,
    pub ask_for_folder: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seeding {
    pub after_completion: bool,
    pub max_concurrent: u32,
    pub rate_limit_kib: u32,
    pub stop_at_ratio: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Network {
    pub listen_port: u16,
    pub dht: bool,
    pub upnp: bool,
    pub max_peers_per_torrent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    pub sources: Sources,
    pub timeout_secs: u32,
    pub result_limit: u32,
    pub min_seeders: u32,
}

/// Which sources a search asks. Built from the indexer registry rather than written
/// out here, so adding a source is still one file and one line in that registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sources(BTreeMap<String, bool>);

impl Default for Sources {
    fn default() -> Self {
        Self(
            crate::search::source_names()
                .map(|name| (name.to_string(), true))
                .collect(),
        )
    }
}

impl Sources {
    pub fn enabled(&self, name: &str) -> bool {
        self.0.get(name).copied().unwrap_or(true)
    }

    pub fn toggles(&mut self) -> impl Iterator<Item = (&str, &mut bool)> {
        self.0.iter_mut().map(|(name, on)| (name.as_str(), on))
    }

    /// A source that has been added since the file was written arrives switched on, and
    /// one that has been removed stops taking up a row on the Settings page.
    fn follow_the_registry(&mut self) {
        self.0
            .retain(|name, _| crate::search::source_names().any(|known| known == name));
        for name in crate::search::source_names() {
            self.0.entry(name.to_string()).or_insert(true);
        }
    }
}

/// What the headless modes need. They read this file and nothing else, so a server
/// with no terminal on it is configured the same way a desktop is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    pub bind: IpAddr,
    pub intake_port: u16,
    pub files_port: u16,
    pub watch_folder: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Interface {
    pub accent: Accent,
    pub confirm_before_removing: bool,
    pub game_warnings: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Pink,
    Cyan,
    Green,
    Amber,
    Plain,
}

impl Default for Downloads {
    fn default() -> Self {
        Self {
            folder: default_download_folder(),
            max_concurrent: 3,
            rate_limit_kib: 0,
            ask_for_folder: false,
        }
    }
}

impl Default for Seeding {
    fn default() -> Self {
        Self {
            after_completion: true,
            max_concurrent: 10,
            rate_limit_kib: 0,
            stop_at_ratio: 0.0,
        }
    }
}

impl Default for Network {
    fn default() -> Self {
        Self {
            // 6881 is the classic BitTorrent port and the first thing an ISP throttles.
            listen_port: 4240,
            dht: true,
            upnp: true,
            max_peers_per_torrent: 100,
        }
    }
}

impl Default for Search {
    fn default() -> Self {
        Self {
            sources: Sources::default(),
            timeout_secs: 8,
            result_limit: 50,
            // YTS reports 0 seeds for most of its catalogue, so a floor of 1 would
            // quietly hide a whole source.
            min_seeders: 0,
        }
    }
}

impl Default for Server {
    fn default() -> Self {
        Self {
            // This machine only. `baka serve` downloads whatever it is handed, so
            // reaching it from the rest of the network is a decision, not a default.
            bind: Ipv4Addr::LOCALHOST.into(),
            intake_port: 4241,
            files_port: 4242,
            watch_folder: default_download_folder().join("watch"),
        }
    }
}

impl Default for Interface {
    fn default() -> Self {
        Self {
            accent: Accent::default(),
            confirm_before_removing: true,
            game_warnings: true,
        }
    }
}

impl fmt::Display for Accent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Pink => "pink",
            Self::Cyan => "cyan",
            Self::Green => "green",
            Self::Amber => "amber",
            Self::Plain => "plain",
        };
        f.write_str(name)
    }
}

impl Settings {
    pub fn path() -> Result<PathBuf, ConfigError> {
        let base = BaseDirs::new().ok_or(ConfigError::NoConfigDir)?;
        Ok(base.config_dir().join("baka").join("config.toml"))
    }

    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&Self::path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            // No file is a first run, not a failure.
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Read(path.to_path_buf(), e)),
        };
        let mut settings: Self =
            toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))?;
        settings.search.sources.follow_the_registry();
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::path()?)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        let text = toml::to_string_pretty(self).map_err(ConfigError::Encode)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| ConfigError::Write(parent.to_path_buf(), e))?;
        }
        fs::write(path, text).map_err(|e| ConfigError::Write(path.to_path_buf(), e))
    }

    /// The Settings page renders and edits this list, so a field that is not here
    /// cannot be changed by a user.
    pub fn fields(&mut self) -> Vec<Field<'_>> {
        let mut fields = vec![
            Field::new(
                DOWNLOADS,
                "Folder",
                "Where finished files land.",
                Value::Path(&mut self.downloads.folder),
            ),
            Field::new(
                DOWNLOADS,
                "Concurrent downloads",
                "How many torrents download at the same time.",
                Value::Count(&mut self.downloads.max_concurrent),
            )
            .at_least(1),
            Field::new(
                DOWNLOADS,
                "Rate limit (KiB/s)",
                "Cap on total download speed.",
                Value::Count(&mut self.downloads.rate_limit_kib),
            )
            .zero_means("unlimited")
            .step(64),
            Field::new(
                DOWNLOADS,
                "Ask for a folder",
                "Pick a folder for every download instead of using the default.",
                Value::Flag(&mut self.downloads.ask_for_folder),
            ),
            Field::new(
                SEEDING,
                "Seed after completion",
                "Keep sharing a torrent once it finishes.",
                Value::Flag(&mut self.seeding.after_completion),
            ),
            Field::new(
                SEEDING,
                "Concurrent seeds",
                "How many finished torrents keep sharing at once.",
                Value::Count(&mut self.seeding.max_concurrent),
            )
            .at_least(1),
            Field::new(
                SEEDING,
                "Rate limit (KiB/s)",
                "Cap on total upload speed.",
                Value::Count(&mut self.seeding.rate_limit_kib),
            )
            .zero_means("unlimited")
            .step(64),
            Field::new(
                SEEDING,
                "Stop at ratio",
                "Stop seeding once this much of the torrent has been uploaded.",
                Value::Ratio(&mut self.seeding.stop_at_ratio),
            )
            .zero_means("never"),
            Field::new(
                NETWORK,
                "Listen port",
                "Port other peers connect to.",
                Value::Port(&mut self.network.listen_port),
            )
            .needs_restart(),
            Field::new(
                NETWORK,
                "DHT",
                "Find peers without asking a tracker.",
                Value::Flag(&mut self.network.dht),
            )
            .needs_restart(),
            Field::new(
                NETWORK,
                "UPnP port mapping",
                "Ask the router to forward the listen port.",
                Value::Flag(&mut self.network.upnp),
            )
            .needs_restart(),
            Field::new(
                NETWORK,
                "Peers per torrent",
                "Upper bound on connections for a single torrent.",
                Value::Count(&mut self.network.max_peers_per_torrent),
            )
            .at_least(1)
            .step(10)
            .needs_restart(),
        ];

        // One row per source, taken from the indexer registry, so a new source needs
        // no edit here to become switchable.
        for (name, enabled) in self.search.sources.toggles() {
            fields.push(Field::new(
                SEARCH,
                name,
                "Ask this source when searching.",
                Value::Flag(enabled),
            ));
        }

        fields.extend([
            Field::new(
                SEARCH,
                "Source timeout (seconds)",
                "How long one source may take before it is skipped.",
                Value::Count(&mut self.search.timeout_secs),
            )
            .at_least(1),
            Field::new(
                SEARCH,
                "Result limit",
                "Most results to keep after merging every source.",
                Value::Count(&mut self.search.result_limit),
            )
            .at_least(1)
            .step(10),
            Field::new(
                SEARCH,
                "Minimum seeders",
                "Hide results with fewer seeders than this.",
                Value::Count(&mut self.search.min_seeders),
            )
            .zero_means("no minimum"),
            Field::new(
                SERVER,
                "Bind address",
                "Address the headless modes listen on. 127.0.0.1 is this machine only.",
                Value::Address(&mut self.server.bind),
            )
            .needs_restart(),
            Field::new(
                SERVER,
                "Magnet intake port",
                "Port the magnet intake accepts links on.",
                Value::Port(&mut self.server.intake_port),
            )
            .needs_restart(),
            Field::new(
                SERVER,
                "File serving port",
                "Port finished downloads are served on.",
                Value::Port(&mut self.server.files_port),
            )
            .needs_restart(),
            Field::new(
                SERVER,
                "Watch folder",
                "Where dropped magnets and torrent files are picked up from.",
                Value::Path(&mut self.server.watch_folder),
            )
            .needs_restart(),
            Field::new(
                INTERFACE,
                "Accent colour",
                "Colour used for highlights and the BAKA wordmark.",
                Value::Accent(&mut self.interface.accent),
            ),
            Field::new(
                INTERFACE,
                "Confirm before removing",
                "Ask before a download or a seed is dropped.",
                Value::Flag(&mut self.interface.confirm_before_removing),
            ),
            Field::new(
                INTERFACE,
                "Game source warnings",
                "Flag results that are executables and can run code.",
                Value::Flag(&mut self.interface.game_warnings),
            ),
        ]);

        fields
    }
}

pub const DOWNLOADS: &str = "Downloads";
pub const SEEDING: &str = "Seeding";
pub const NETWORK: &str = "Network";
pub const SEARCH: &str = "Search";
pub const SERVER: &str = "Server";
pub const INTERFACE: &str = "Interface";

pub enum Value<'a> {
    Flag(&'a mut bool),
    Address(&'a mut IpAddr),
    Port(&'a mut u16),
    Count(&'a mut u32),
    Ratio(&'a mut f32),
    Path(&'a mut PathBuf),
    Accent(&'a mut Accent),
}

pub struct Field<'a> {
    pub group: &'static str,
    pub label: &'a str,
    pub description: &'static str,
    pub needs_restart: bool,
    pub zero_means: Option<&'static str>,
    pub value: Value<'a>,
    step: u32,
    at_least: u32,
}

impl<'a> Field<'a> {
    fn new(
        group: &'static str,
        label: &'a str,
        description: &'static str,
        value: Value<'a>,
    ) -> Self {
        Self {
            group,
            label,
            description,
            needs_restart: false,
            zero_means: None,
            value,
            step: 1,
            at_least: 0,
        }
    }

    fn needs_restart(mut self) -> Self {
        self.needs_restart = true;
        self
    }

    fn zero_means(mut self, word: &'static str) -> Self {
        self.zero_means = Some(word);
        self
    }

    fn step(mut self, step: u32) -> Self {
        self.step = step;
        self
    }

    fn at_least(mut self, floor: u32) -> Self {
        self.at_least = floor;
        self
    }

    pub fn display(&self) -> String {
        match &self.value {
            Value::Flag(on) => String::from(if **on { "on" } else { "off" }),
            Value::Address(address) => address.to_string(),
            Value::Port(port) => port.to_string(),
            Value::Count(count) => match self.zero_means {
                Some(word) if **count == 0 => word.to_string(),
                _ => count.to_string(),
            },
            Value::Ratio(ratio) => match self.zero_means {
                Some(word) if **ratio == 0.0 => word.to_string(),
                _ => format!("{ratio:.2}"),
            },
            Value::Path(path) => path.display().to_string(),
            Value::Accent(accent) => accent.to_string(),
        }
    }

    /// One press of an arrow key. A switch flips whichever way it is nudged, because
    /// there is no third position for the other direction to reach.
    pub fn nudge(&mut self, up: bool) {
        let step = self.step;
        let floor = self.at_least;
        match &mut self.value {
            Value::Flag(on) => **on = !**on,
            Value::Accent(accent) => **accent = accent.next(up),
            Value::Port(port) => {
                **port = match up {
                    true => port.saturating_add(1),
                    false => port.saturating_sub(1).max(1),
                }
            }
            Value::Count(count) => {
                **count = match up {
                    true => count.saturating_add(step),
                    false => count.saturating_sub(step).max(floor),
                }
            }
            Value::Ratio(ratio) => {
                let stepped = if up { **ratio + 0.1 } else { **ratio - 0.1 };
                **ratio = (stepped.clamp(0.0, 100.0) * 100.0).round() / 100.0;
            }
            // Neither a folder nor an address has a next one to step to. Both are
            // typed.
            Value::Path(_) | Value::Address(_) => {}
        }
    }

    /// What a typed edit starts from. A switch and a colour have nothing to type, so
    /// they answer `None` and the page never opens an editor on them.
    pub fn typed(&self) -> Option<String> {
        match &self.value {
            Value::Flag(_) | Value::Accent(_) => None,
            Value::Address(address) => Some(address.to_string()),
            Value::Port(port) => Some(port.to_string()),
            Value::Count(count) => Some(count.to_string()),
            Value::Ratio(ratio) => Some(format!("{ratio:.2}")),
            Value::Path(path) => Some(path.display().to_string()),
        }
    }

    pub fn accept(&mut self, typed: &str) -> Result<(), &'static str> {
        let typed = typed.trim();
        let floor = self.at_least;
        match &mut self.value {
            Value::Flag(_) | Value::Accent(_) => Err("this one changes with the arrow keys"),
            Value::Address(address) => match typed.parse() {
                Ok(parsed) => {
                    **address = parsed;
                    Ok(())
                }
                Err(_) => Err("an address like 127.0.0.1 or 0.0.0.0"),
            },
            Value::Port(port) => match typed.parse::<u16>() {
                Ok(0) | Err(_) => Err("a port is a number from 1 to 65535"),
                Ok(parsed) => {
                    **port = parsed;
                    Ok(())
                }
            },
            Value::Count(count) => match typed.parse::<u32>() {
                Ok(parsed) if parsed >= floor => {
                    **count = parsed;
                    Ok(())
                }
                Ok(_) => Err("that is below the lowest this setting goes"),
                Err(_) => Err("this one takes a whole number"),
            },
            Value::Ratio(ratio) => match typed.parse::<f32>() {
                Ok(parsed) if (0.0..=100.0).contains(&parsed) => {
                    **ratio = parsed;
                    Ok(())
                }
                _ => Err("a ratio is a number from 0 to 100"),
            },
            Value::Path(path) => match typed.is_empty() {
                true => Err("a folder cannot be empty"),
                false => {
                    **path = PathBuf::from(typed);
                    Ok(())
                }
            },
        }
    }
}

impl Accent {
    pub const ALL: [Self; 5] = [
        Self::Pink,
        Self::Cyan,
        Self::Green,
        Self::Amber,
        Self::Plain,
    ];

    fn next(self, forward: bool) -> Self {
        let at = Self::ALL.iter().position(|a| *a == self).unwrap_or(0);
        let count = Self::ALL.len();
        let moved = match forward {
            true => at + 1,
            false => at + count - 1,
        };
        Self::ALL[moved % count]
    }
}

/// Resume data, saved torrents and the DHT routing table. State rather than settings, so
/// it keeps its own directory and can be deleted without losing anything a user chose.
pub fn state_dir() -> Result<PathBuf, ConfigError> {
    let base = BaseDirs::new().ok_or(ConfigError::NoConfigDir)?;
    Ok(base.data_local_dir().join("baka"))
}

/// A relative path keeps a headless box working when it has no user directories.
fn default_download_folder() -> PathBuf {
    UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("downloads"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_survive_a_round_trip() {
        let settings = Settings::default();
        let text = toml::to_string_pretty(&settings).unwrap();
        assert_eq!(settings, toml::from_str(&text).unwrap());
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let text = "[network]\nlisten_port = 5000\n";
        let settings: Settings = toml::from_str(text).unwrap();
        assert_eq!(settings.network.listen_port, 5000);
        assert_eq!(settings.network.dht, Network::default().dht);
        assert_eq!(settings.downloads, Downloads::default());
    }

    #[test]
    fn settings_from_a_newer_version_still_load() {
        let text = "[downloads]\nmax_concurrent = 9\nnot_a_setting_yet = true\n";
        let settings: Settings = toml::from_str(text).unwrap();
        assert_eq!(settings.downloads.max_concurrent, 9);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let settings = Settings::load_from(Path::new("no/such/config.toml")).unwrap();
        assert_eq!(settings, Settings::default());
    }

    #[test]
    fn every_field_is_labelled_and_grouped() {
        let mut settings = Settings::default();
        let fields = settings.fields();
        for field in &fields {
            assert!(!field.label.is_empty());
            assert!(!field.group.is_empty());
            assert!(field.description.ends_with('.'));
        }

        // Two fields in one group sharing a label means a copy and paste slip.
        for (i, field) in fields.iter().enumerate() {
            let twin = fields[i + 1..]
                .iter()
                .find(|other| other.group == field.group && other.label == field.label);
            assert!(twin.is_none(), "duplicate label {}", field.label);
        }
    }

    #[test]
    fn zero_reads_as_words_not_as_a_number() {
        let mut settings = Settings::default();
        let fields = settings.fields();
        let limit = fields
            .iter()
            .find(|f| f.group == DOWNLOADS && f.label == "Rate limit (KiB/s)")
            .unwrap();
        assert_eq!(limit.display(), "unlimited");
    }

    #[test]
    fn every_source_gets_a_row_it_can_be_switched_off_from() {
        let mut settings = Settings::default();
        let labels: Vec<String> = settings
            .fields()
            .iter()
            .filter(|field| field.group == SEARCH)
            .map(|field| field.label.to_string())
            .collect();
        for source in crate::search::source_names() {
            assert!(labels.iter().any(|label| label == source), "{source}");
        }
    }

    #[test]
    fn a_source_the_file_has_never_heard_of_arrives_switched_on() {
        let mut sources = Sources(BTreeMap::new());
        sources.follow_the_registry();
        for source in crate::search::source_names() {
            assert!(sources.enabled(source), "{source}");
        }
    }

    #[test]
    fn a_source_that_no_longer_exists_stops_taking_up_a_row() {
        let mut sources = Sources(BTreeMap::from([("demonoid".to_string(), false)]));
        sources.follow_the_registry();
        assert_eq!(
            sources.toggles().count(),
            crate::search::source_names().count()
        );
    }

    #[test]
    fn a_switched_off_source_survives_a_reload() {
        let mut settings = Settings::default();
        let first = crate::search::source_names().next().unwrap();
        *settings.search.sources.0.get_mut(first).unwrap() = false;
        let text = toml::to_string_pretty(&settings).unwrap();
        let mut reloaded: Settings = toml::from_str(&text).unwrap();
        reloaded.search.sources.follow_the_registry();
        assert!(!reloaded.search.sources.enabled(first));
    }

    #[test]
    fn an_arrow_key_flips_a_switch_whichever_way_it_points() {
        let mut settings = Settings::default();
        let mut fields = settings.fields();
        let dht = fields.iter_mut().find(|f| f.label == "DHT").unwrap();
        assert_eq!(dht.display(), "on");
        dht.nudge(false);
        assert_eq!(dht.display(), "off");
        dht.nudge(false);
        assert_eq!(dht.display(), "on");
    }

    #[test]
    fn the_accent_colour_cycles_and_comes_back_round() {
        let mut settings = Settings::default();
        for _ in 0..Accent::ALL.len() {
            let mut fields = settings.fields();
            let accent = fields.iter_mut().find(|f| f.label == "Accent colour");
            accent.unwrap().nudge(true);
        }
        assert_eq!(settings.interface.accent, Accent::default());
    }

    #[test]
    fn a_setting_with_a_floor_will_not_be_nudged_below_it() {
        let mut settings = Settings::default();
        let mut fields = settings.fields();
        let concurrent = fields
            .iter_mut()
            .find(|f| f.group == DOWNLOADS && f.label == "Concurrent downloads")
            .unwrap();
        for _ in 0..10 {
            concurrent.nudge(false);
        }
        assert_eq!(concurrent.display(), "1");
    }

    #[test]
    fn a_typed_value_is_checked_before_it_is_kept() {
        let mut settings = Settings::default();
        let mut fields = settings.fields();
        let port = fields
            .iter_mut()
            .find(|f| f.label == "Listen port")
            .unwrap();
        assert!(port.accept("70000").is_err());
        assert!(port.accept("0").is_err());
        assert!(port.accept("not a port").is_err());
        assert!(port.accept(" 51413 ").is_ok());
        assert_eq!(port.display(), "51413");
    }

    #[test]
    fn a_switch_has_nothing_to_type_into() {
        let mut settings = Settings::default();
        let fields = settings.fields();
        let dht = fields.iter().find(|f| f.label == "DHT").unwrap();
        assert!(dht.typed().is_none());
    }

    #[test]
    fn the_settings_file_sits_where_the_readme_says() {
        let path = Settings::path().unwrap();
        assert!(path.ends_with("baka/config.toml") || path.ends_with("baka\\config.toml"));
    }
}
