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

> **Status: pre-alpha.** Everything below works: the interface, the downloads, the
> settings page, every source, the headless modes and attaching to one of them. Pushing
> a tag builds and publishes a release, and the install commands below work from the
> first tagged release onwards. The plan, and what each phase actually shipped, is in
> [ROADMAP.md](ROADMAP.md).

## Why

[torlink](https://github.com/baairon/torlink) had the right idea and the wrong plumbing.
BAKA is the same idea as a native binary: nothing to install first, nothing left behind,
and fast enough that the terminal feels like the point rather than a limitation.

It also keeps its settings in one place. Press `s`, change what you want, done. No
environment variables to discover, no flags to memorise.

## Install

Windows, x64 and arm64:

```powershell
scoop bucket add baka https://gitlab.ramon.moe/4G0NYY/BAKA
scoop install baka
```

Anywhere, without compiling, if you already have
[cargo-binstall](https://github.com/cargo-bins/cargo-binstall):

```
cargo binstall b-baka
```

Anywhere, from source:

```
cargo install b-baka
```

Arch Linux, from the AUR:

```
paru -S b-baka
```

The crates.io name stutters because `baka` was already taken there, but the command every
one of these installs is still `baka`.

`cargo binstall` takes the release archive when there is one for your target, which today
means Windows on x64 and arm64 and Linux on x64, and falls back to compiling when there
is not. The Linux archive is built against a current glibc, so on an older distribution
use `cargo install` or the AUR package instead.

For the headless modes there is a container image:

```
docker run --rm -v "$PWD/config:/home/baka/.config/baka" \
                -v "$PWD/downloads:/home/baka/downloads" \
                -p 4241:4241 -p 4242:4242 \
                registry.ramon.moe/4g0nyy/baka:latest serve
```

The bind address defaults to loopback, which inside a container means nothing outside it
can reach the intake. Set `bind` to `0.0.0.0` in the mounted `config.toml`, and mean it:
the magnet intake downloads whatever it is handed.

Every release also carries the archives on their own, with a `SHA256SUMS.txt` next to
them, if you would rather unpack the binary and put it somewhere yourself:

```powershell
(Get-FileHash baka-0.1.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256).Hash
```

Every route installs a portable executable and nothing else. Uninstalling removes the
binary and leaves `config.toml` and your downloads alone, so reinstalling finds the
settings you already had.

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
| `Esc` | Close what is open, twice to go back to the start |
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
| Search | Which sources are enabled, per source timeout, result limit, minimum seeders, how long results are remembered |
| Server | Bind address, magnet intake port, file serving port, watch folder |
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

A value BAKA cannot read costs that one setting and nothing else. It goes back to its
default, BAKA names it once on the status line, and the next save writes the file back
clean. That is what makes a file written by an older version, or a typo in a hand
edit, cost one line rather than the lot. A file that is not TOML at all is refused with
the reason, because quietly starting over is worse than saying so.

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
| `baka watch [dir]` | Download anything dropped into a directory |
| `baka serve` | Accept magnets over HTTP |
| `baka files` | Serve finished downloads over HTTP |
| `baka attach [address]` | Open the interface on a BAKA that is already running |

Add `--daemon` to `watch`, `serve` or `files` to keep running after you close the
terminal or log out. `baka --help` lists everything.

`baka watch` picks up magnet links, bare infohashes and `.torrent` files. Drop a file
in, and a moment later it is renamed to `.taken` or, if there was nothing usable in it,
to `.failed`. With no directory given it uses the watch folder from the Settings page.

`baka serve` answers `POST` with a magnet link, an infohash or a file path in the body,
one per line, and `GET` with what is currently running:

```
curl -d "magnet:?xt=urn:btih:..." http://127.0.0.1:4241
curl http://127.0.0.1:4241
```

It answers a few more things, which is what `baka attach` drives it with:

| Request | Does |
| --- | --- |
| `GET /torrents` | The same list as JSON |
| `POST /torrents/<id>/pause` | Pause one |
| `POST /torrents/<id>/resume` | Start it again |
| `POST /torrents/<id>/remove` | Stop it, leaving the files on disk |

A `Folder:` header on a `POST` says where that download should land, as a path on the
machine running the session. Without one it uses that machine's download folder.

`baka files` serves the download folder over HTTP, with directory listings and range
requests, so a browser or a player can open a finished film without copying it first.

Both listen on `127.0.0.1` by default, which is this machine only. The Settings page has
the bind address and both ports if you want them reachable from elsewhere. Think before
you widen the magnet intake: it downloads whatever it is handed, into whatever folder it
is told, and it asks nobody for a password.

Run one session at a time. The interface, `baka get` and the headless modes all open the
same session, and two of them at once fight over it. That is what `baka attach` is for:

```
baka serve --daemon
baka attach
```

The second command opens the usual interface on the session the first one started.
Downloads, seeding, pausing, stopping and starting new ones all work the way they do
locally. Quitting the attached interface leaves the session running, and so does losing
the connection it was attached over, which is what makes it safe over SSH.

With no address it uses the bind address and the magnet intake port from the Settings
page. Give it one to reach a BAKA somewhere else:

```
baka attach nas.lan:4241
```

The Settings page is still this machine's settings while attached, and it says so. The
session elsewhere reads its own.

## Sources

A short, hand-picked list. No indexer proxy to configure.

| Category | Sources |
| --- | --- |
| Movies | YTS, The Pirate Bay, 1337x, BitTorrented |
| TV | EZTV, The Pirate Bay, 1337x, BitTorrented |
| Anime | Nyaa, SubsPlease |
| Games | FitGirl |
| Books | The Pirate Bay, 1337x, Nyaa |
| Audiobooks | The Pirate Bay, 1337x |

Game results are executables and can run code on your machine. BAKA marks them clearly.
Video and subtitle results cannot.

Some of them work differently to the rest, and the Search tab says which source each
result came from so you can tell:

- EZTV publishes new releases rather than a search, so a query keeps whatever in the
  current feed matches it. Browsing is where it shows its whole hand.
- 1337x is blocked or challenged on some networks. BAKA tries its mirrors in turn and
  moves on quietly if none of them answer.
- Nyaa files manga, light novels and the audiobooks read from them all under one
  heading, so they arrive on the Books shelf rather than split between the two.

## How it works

Search queries every enabled source at once, each on its own timeout, then merges and
ranks by seeders and title match. A source that is down is skipped, not fatal. The same
search asked again inside the same run is answered from what the sources already said,
for as long as the Remember results setting says, and that answer is never written to
disk.

Downloads run through a queue that honours the concurrency limits and stop at ratio. A
torrent past the limit reads as `queued` rather than silently doing nothing, and one you
pause by hand stays paused, including across a restart.

Downloading uses [librqbit](https://github.com/ikatson/rqbit) in-process. Files land on
your disk and nothing routes through a server we control. BAKA talks to the torrent
network and to the indexers, and that is all.

## Build

```
git clone https://gitlab.ramon.moe/4G0NYY/BAKA
cd BAKA
cargo run
```

Before opening a pull request, read [CLAUDE.md](CLAUDE.md). It is written for Claude Code
but the rules apply to everyone, and one of them is enforced by CI.
[CONTRIBUTING.md](CONTRIBUTING.md) has the checks to run and how a release is cut.

## Legal

BAKA is a search tool and a BitTorrent client. It hosts nothing, indexes nothing and
mirrors nothing. What you download with it is on you, and copyright law applies the same
way it does everywhere else.

## License

MIT.
