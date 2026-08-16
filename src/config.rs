//! Every persistent setting BAKA has. One struct, one file, one page in the TUI.
//! Nothing else in the codebase may read the environment for configuration.

use std::fmt;
use std::fs;
use std::io;
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
    pub timeout_secs: u32,
    pub result_limit: u32,
    pub min_seeders: u32,
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
            timeout_secs: 8,
            result_limit: 50,
            // YTS reports 0 seeds for most of its catalogue, so a floor of 1 would
            // quietly hide a whole source.
            min_seeders: 0,
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
        toml::from_str(&text).map_err(|e| ConfigError::Parse(path.to_path_buf(), e))
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
        vec![
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
            ),
            Field::new(
                DOWNLOADS,
                "Rate limit (KiB/s)",
                "Cap on total download speed.",
                Value::Count(&mut self.downloads.rate_limit_kib),
            )
            .zero_means("unlimited"),
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
            ),
            Field::new(
                SEEDING,
                "Rate limit (KiB/s)",
                "Cap on total upload speed.",
                Value::Count(&mut self.seeding.rate_limit_kib),
            )
            .zero_means("unlimited"),
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
            ),
            Field::new(
                SEARCH,
                "Source timeout (seconds)",
                "How long one source may take before it is skipped.",
                Value::Count(&mut self.search.timeout_secs),
            ),
            Field::new(
                SEARCH,
                "Result limit",
                "Most results to keep after merging every source.",
                Value::Count(&mut self.search.result_limit),
            ),
            Field::new(
                SEARCH,
                "Minimum seeders",
                "Hide results with fewer seeders than this.",
                Value::Count(&mut self.search.min_seeders),
            )
            .zero_means("no minimum"),
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
        ]
    }
}

pub const DOWNLOADS: &str = "Downloads";
pub const SEEDING: &str = "Seeding";
pub const NETWORK: &str = "Network";
pub const SEARCH: &str = "Search";
pub const INTERFACE: &str = "Interface";

pub enum Value<'a> {
    Flag(&'a mut bool),
    Port(&'a mut u16),
    Count(&'a mut u32),
    Ratio(&'a mut f32),
    Path(&'a mut PathBuf),
    Accent(&'a mut Accent),
}

pub struct Field<'a> {
    pub group: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub needs_restart: bool,
    pub zero_means: Option<&'static str>,
    pub value: Value<'a>,
}

impl<'a> Field<'a> {
    fn new(
        group: &'static str,
        label: &'static str,
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

    pub fn display(&self) -> String {
        match &self.value {
            Value::Flag(on) => String::from(if **on { "on" } else { "off" }),
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
    fn the_settings_file_sits_where_the_readme_says() {
        let path = Settings::path().unwrap();
        assert!(path.ends_with("baka/config.toml") || path.ends_with("baka\\config.toml"));
    }
}
