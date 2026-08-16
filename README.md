```
 ____    _    _  __    _
| __ )  / \  | |/ /   / \
|  _ \ / _ \ | ' /   / _ \
| |_) / ___ \| . \  / ___ \
|____/_/   \_\_|\_\/_/   \_\

BitTorrent Acquisition & Keyword Aggregator
```

Search torrents and download them without leaving your terminal. One Rust binary, no
runtime, no setup.

> **Status: pre-alpha.** The interface, the downloads, the settings page and every
> source below all work. Nothing is installable yet, so the install commands describe
> the target rather than today. The plan is in [ROADMAP.md](ROADMAP.md).

## Why

[torlink](https://github.com/baairon/torlink) had the right idea and the wrong plumbing.
BAKA is the same idea as a native binary: nothing to install first, nothing left behind,
and fast enough that the terminal feels like the point rather than a limitation.

It also keeps its settings in one place. Press `s`, change what you want, done. No
environment variables to discover, no flags to memorise.

## Install

```powershell
winget install BAKA
```

```powershell
scoop install baka
```

```
cargo install baka
```

winget and Scoop cover Windows. `cargo install` works anywhere Rust does, including Linux
and macOS.

## Use

Run it:

```
baka
```

Type to search. Paste a magnet link, a bare infohash or the path to a `.torrent` file
into the same box and BAKA downloads it instead of searching for it. Press Enter on an
empty box to browse what the sources are listing right now, a share from each category
rather than whichever one has the biggest swarms.

Downloads run in the background while you keep searching. Interrupted downloads resume on
the next launch. Finished downloads seed until you stop them.

### Keys

| Key | Action |
| --- | --- |
| `/` | Focus search |
| `Enter` | Run the search, browse an empty box, or download the selected result |
| `Tab` | Switch between Search, Downloads, Seeding and Settings |
| `j` `k` or arrows | Move, and change the selected setting |
| `d` | Download to the default folder |
| `D` | Download to a folder you pick |
| `p` | Pause or resume |
| `x` | Stop, files already on disk stay there |
| `c` | Copy magnet link |
| `s` | Settings |
| `?` | Help |
| `q` | Quit |

## Settings

Everything configurable lives on one page. Press `s`, change a row with the arrow keys or
press Enter to type a value, and it saves itself when you move on. There are no
environment variables and no persistent flags to hunt for.

| Group | What you can change |
| --- | --- |
| Downloads | Download folder, maximum concurrent downloads, download rate limit, whether to ask for a folder every time |
| Seeding | Seed after completion, maximum concurrent seeds, upload rate limit, stop at ratio |
| Network | Listen port, DHT, UPnP port mapping, peer limit per torrent |
| Search | Which sources are enabled, per source timeout, result limit, minimum seeders |
| Interface | Accent colour, confirm before removing, game source warnings |

Settings apply immediately. The few that cannot, such as the listen port, say so on their
own row rather than pretending otherwise. Rate limits, the download folder and the queue
limits all take effect without a restart.

Behind the page is a single file:

| Platform | Path |
| --- | --- |
| Windows | `%APPDATA%\baka\config.toml` |
| Linux | `~/.config/baka/config.toml` |
| macOS | `~/Library/Application Support/baka/config.toml` |

Edit it by hand if you prefer. `baka settings` opens the page without the rest of the TUI,
and `baka settings --path` prints the location. Deleting the file resets everything to
defaults, and a partial file is fine because missing values fall back.

Command line flags override settings for a single run and never write to the file.

Unfinished downloads are remembered elsewhere, because they are progress rather than
preference:

| Platform | Path |
| --- | --- |
| Windows | `%LOCALAPPDATA%\baka` |
| Linux | `~/.local/share/baka` |
| macOS | `~/Library/Application Support/baka` |

Deleting that folder costs you the progress on anything still downloading, and nothing else.

## Without a terminal

| Command | Does |
| --- | --- |
| `baka get <magnet\|infohash\|file>` | Download one thing and exit |
| `baka search ["<query>"] [--category <name>]` | Print results as text, or browse with no query |
| `baka settings` | Open the settings page on its own |
| `baka watch <dir>` | Download anything dropped into a directory |
| `baka serve` | Accept magnets over HTTP |
| `baka files` | Serve finished downloads over HTTP |
| `baka attach` | Attach a TUI to a running daemon |

Add `--daemon` to keep running after you log out. `baka --help` lists everything.

## Sources

A short, hand-picked list. No indexer proxy to configure.

| Category | Sources |
| --- | --- |
| Movies | YTS, The Pirate Bay, 1337x, BitTorrented |
| TV | EZTV, The Pirate Bay, 1337x, BitTorrented |
| Anime | Nyaa, SubsPlease |
| Games | FitGirl |

Game results are executables and can run code on your machine. BAKA marks them clearly.
Video and subtitle results cannot.

Two of them work differently to the rest, and the Search tab says which source each
result came from so you can tell:

- EZTV publishes new releases rather than a search, so a query keeps whatever in the
  current feed matches it. Browsing is where it shows its whole hand.
- 1337x is blocked or challenged on some networks. BAKA tries its mirrors in turn and
  moves on quietly if none of them answer.

## How it works

Search queries every enabled source at once, each on its own timeout, then merges and
ranks by seeders and title match. A source that is down is skipped, not fatal.

Downloads run through a queue that honours the concurrency limits and stop at ratio. A
torrent past the limit reads as `queued` rather than silently doing nothing, and one you
pause by hand stays paused, including across a restart.

Downloading uses [librqbit](https://github.com/ikatson/rqbit) in-process. Files land on
your disk and nothing routes through a server we control. BAKA talks to the torrent
network and to the indexers, and that is all.

## Build

```
git clone https://github.com/4G0NYY/BAKA
cd BAKA
cargo run
```

Before opening a pull request, read [CLAUDE.md](CLAUDE.md). It is written for Claude Code
but the rules apply to everyone, and one of them is enforced by CI.

## Legal

BAKA is a search tool and a BitTorrent client. It hosts nothing, indexes nothing and
mirrors nothing. What you download with it is on you, and copyright law applies the same
way it does everywhere else.

## License

MIT.
