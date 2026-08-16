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

> **Status: pre-alpha.** Nothing is installable yet. The plan is in [ROADMAP.md](ROADMAP.md).
> Everything below describes the target, not the present.

## Why

[torlink](https://github.com/baairon/torlink) had the right idea and the wrong plumbing.
BAKA is the same idea as a native binary: nothing to install first, nothing left behind,
and fast enough that the terminal feels like the point rather than a limitation.

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

Type to search. Press Enter on an empty box to browse a curated library. Paste a magnet
link, a bare infohash, or the path to a `.torrent` file and BAKA takes it directly.

Downloads run in the background while you keep searching. Interrupted downloads resume on
the next launch. Finished downloads seed until you stop them.

### Keys

| Key | Action |
| --- | --- |
| `/` | Focus search |
| `Enter` | Run search, or browse when empty |
| `Tab` | Switch between Search, Downloads and Seeding |
| `j` `k` or arrows | Move |
| `d` | Download to the default folder |
| `D` | Download to a folder you pick |
| `o` | Change the default download folder |
| `p` | Pause or resume |
| `x` | Stop |
| `c` | Copy magnet link |
| `?` | Help |
| `q` | Quit |

### Without a terminal

| Command | Does |
| --- | --- |
| `baka get <magnet\|infohash\|file>` | Download one thing and exit |
| `baka search "<query>"` | Print results as text |
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

## How it works

Search queries every relevant source at once, each on its own timeout, then merges and
ranks by seeders and title match. A source that is down is skipped, not fatal.

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
