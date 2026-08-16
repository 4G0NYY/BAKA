# BAKA Roadmap

BitTorrent Acquisition & Keyword Aggregator.

Goal: a single Rust binary that matches or beats [torlink](https://github.com/baairon/torlink)
in functionality, installs in one command on Windows, and stays small enough to read in an
afternoon.

Status: phases 0 to 3 are done. `baka` with no arguments is the whole product: search,
downloads, seeding and settings in one terminal interface. Phase 4 is next.

## Decisions already made

| Question | Answer | Why |
| --- | --- | --- |
| Torrent engine | Embed [`librqbit`](https://crates.io/crates/librqbit) | Only mature pure-Rust engine with DHT, uTP, resume and seeding. Writing our own delays parity by months. |
| Interface | TUI first, CLI subcommands alongside | Matches torlink's shape and keeps headless and scripted use possible. |
| Settings | One Settings page, one TOML file, no environment variables | Every knob is in one place you can find without reading docs. |
| Sources | Same curated list as torlink | Parity out of the box, no setup for the user. |
| Platforms for 1.0 | Windows via winget, Scoop and Chocolatey, plus `cargo install baka` everywhere | Windows is the primary target. `cargo install` covers Linux and macOS for free. |
| Layout | One crate, several modules | Five crates for a tool this size is overhead, not structure. Split only when compile times justify it. |

## Settings model

torlink spreads its behaviour across env vars, one-off keybinds and flags. BAKA does not.

- One file: `config.toml` in the platform config directory.
- One editor: the Settings page inside the TUI, which reads and writes that file.
- Every persistent setting appears on that page. If a knob exists, it is listed there.
- CLI flags exist only for one-shot overrides of a single run. They never write config.
- No environment variables. Not for paths, not for ports, not for anything.
- A missing or partial config file is never an error. Defaults fill the gaps.

The Settings page is not a phase 7 nicety. It shipped with the TUI in phase 3, because a
setting that has no home ends up as an env var, and that is the thing being avoided.

## Parity checklist

Everything torlink does, tracked to the phase that delivers it.

| Feature | Phase |
| --- | --- |
| Keyword search across curated sources | 1 |
| Magnet link input | 2 |
| Bare infohash input | 2 |
| `.torrent` file path input | 2 |
| Empty search browses a curated library | 4 |
| Background downloads while searching continues | 3 |
| Progress, speed and ETA per download | 3 |
| Settings page covering every persistent option | 3 |
| Per-download folder override | 3 |
| Interrupted downloads resume on next start | 2 |
| Auto seed on completion, pausable from a Seeding tab | 3 |
| Keyboard help overlay | 3 |
| `watch <dir>` intake | 5 |
| `serve` HTTP magnet intake | 5 |
| `files` HTTP serving of finished downloads | 5 |
| Background daemon | 5 |
| Attach a TUI to a running daemon | 7 |

Known gap: torlink uses WebTorrent and therefore reaches WebRTC browser peers.
librqbit speaks TCP and uTP only. This affects browser-seeded swarms, not normal ones.
Not planned for 1.0. Revisit only if real swarms turn out to need it.

## Phase 0: foundations (done)

Target was `baka --version`. What shipped:

- Edition 2024, MSRV pinned and checked in CI. It started at 1.85, the edition floor, and
  moved to 1.88 in phase 2 for the reason recorded there.
- Modules stubbed with the rule that governs each one: `search`, `engine`, `tui`, `server`.
- `config.rs` holds the whole settings model: five grouped structs, `Default` impls, and
  load and save to TOML under the platform config dir via `directories`.
- `Settings::fields()` returns every setting with its group, label, description and a
  mutable handle to the value. The phase 3 page renders and edits that list, so a field
  missing from it cannot be reached by a user.
- Missing files, partial files and keys from a newer version all load without an error.
- `baka` prints the art from `stuff/` plus version and settings path, and `baka settings`
  lists every setting as text. Phase 3 replaced both with the interface. `baka settings
  --path` still prints the path alone.
- CI on GitHub Actions: `fmt`, `clippy --all-targets -D warnings`, `test` on Windows and
  Linux, an MSRV job, and `scripts/no-em-dashes.sh`.

## Phase 1: search (done)

Target was `baka search "query"`. What shipped:

- The `Indexer` trait is `url()` plus `parse()`, not one async call. Sources describe a
  request and parse a body, and the requests, timeouts, concurrency and ranking live in
  `search.rs` once. That is what makes every scraper test offline.
- `Torrent`: title, size, seeders, leechers, category, source, infohash and magnet.
- YTS and Nyaa, queried concurrently, each under its own timeout. A source that dies is
  reported by name and the rest of the search still returns.
- Ranking: a full title match first, seeders second. Deduplicated by infohash, keeping
  the best seeded copy.
- `baka search "query" --category anime` limits which sources are asked.
- Fixtures are real saved responses in `src/search/fixtures/`. CI never hits the network.

Two things found while building:

- `yts.mx` does not resolve on every network, and the YTS API itself now points callers at
  `movies-api.accel.li`. That is the base URL BAKA uses.
- YTS reports 0 seeds for most of its catalogue, so `min_seeders` defaults to 0. A floor of
  1 silently hides nearly all of one source.

Phase 4 note: a source that needs a second request per result, 1337x being the obvious one,
does not fit `url()` plus `parse()`. Extend the trait when that source lands, not before.

## Phase 2: engine (done)

Target was `baka get <magnet|infohash|path>`. What shipped:

- `Engine` wraps a librqbit `Session`. Everything above it speaks `Input`, `Progress` and
  `State`, so no librqbit type appears outside `engine.rs`.
- `Input::parse` takes a magnet link, a 40 character hex infohash, a 32 character base32
  infohash, or the path to a `.torrent` file. A bare infohash becomes a magnet carrying the
  same public trackers a search result gets, so it can find peers without DHT.
- Session state lives in the platform local data directory, not beside `config.toml`.
  It is resume data, and deleting it costs progress but never settings.
- Adding a torrent sets `overwrite`, without which librqbit refuses to touch files it did
  not create, and a resumed or finished torrent can then neither continue nor seed.
- `Progress` is a poll, not a subscription. Nothing above the engine holds a channel open.
- `baka get` prints one status line in place, resumes what the last run left behind, seeds
  when the setting says to, and treats Ctrl+C as stop rather than kill at every stage.
- Verified end to end against a real 755 MiB Debian torrent: download, completion, seeding,
  restart, recheck, resume.

Two things found while building:

- MSRV moved from 1.85 to 1.88. librqbit 9 uses let chains, which stabilised in 1.88, so
  1.85 cannot build the dependency tree at all.
- Adding a magnet blocks until peers hand over the file list, and that step can outlast the
  download itself. It gets its own line rather than an empty prompt.

Left for phase 3: maximum concurrent downloads, maximum concurrent seeds and stop at ratio.
All three need a queue watching every torrent, which is phase 3 work because that is where
the list of torrents becomes something a user sees. That queue landed there, and `baka get`
runs it too.

## Phase 3: TUI and settings (done)

Target was `baka` with no arguments being the whole product. What shipped:

- `ratatui` 0.30 with its crossterm backend. Four tabs: Search, Downloads, Seeding,
  Settings. A key thread and a one second ticker feed one event loop, so a search, a
  magnet waiting on peers and the interface never wait on each other.
- Search results, downloads and seeds are three views of state the loop refreshes. A
  search runs in a spawned task and arrives as an event, so typing never stalls.
- A magnet link, a bare infohash or a file path typed into the search box is downloaded
  rather than searched. That is what pasting one into a search box is asking for.
- Downloads tab: progress bar, size, speed, time left, peers and state. Seeding tab:
  ratio, shared bytes, upload speed, peers and state.
- A queue in the engine enforces maximum concurrent downloads, maximum concurrent seeds
  and stop at ratio. It is a pure function over a list of torrents, so every rule in it
  is tested without a session. `baka get` runs the same queue.
- Settings page over `Settings::fields()`, so a field that is not on the page cannot
  exist. Arrow keys change a value, `Enter` types one, and a value that does not parse
  says why and keeps the editor open. Changes apply live where the session allows it and
  save when the row is left. A setting that cannot apply live says so on its own row.
- The Search group lists one row per source, built from the indexer registry rather than
  written out by hand, which is what phase 4 needs it to do.
- `baka settings` opens that page on its own and starts no session, so it can be used on
  a machine already running BAKA. `baka settings --path` still prints the path alone.
- Every key the README promises: `/`, `Enter`, `Tab`, `j` `k`, `d`, `D`, `p`, `x`, `c`,
  `s`, `?`, `q`. `?` opens the list of them.
- Branding lands on the empty Search tab: the art, the BAKA wordmark, the name expansion
  and an accent colour that follows the setting. Nothing is playful anywhere else.

Four things found while building:

- librqbit treats pausing an already paused torrent as an error, so the queue compares
  against the current state instead of issuing orders and hoping. The same check makes
  `enforce` safe to call every second.
- A queue has to tell a torrent the user paused apart from one the queue paused, or
  quitting with something paused hands it back running on the next start. That is what
  the wish per torrent is, and it is why `queued` is a state a user can see.
- The session reads its default output folder once at startup, so the folder is named on
  every add instead. That is what turns the download folder into a live setting.
- The clipboard needs a dependency. `arboard` without its image feature is the whole of
  it, and OSC 52 was passed over because it fails silently on the consoles that do not
  support it, which looks like a broken key.

## Phase 4: full source list

Target: source parity with torlink.

- Movies: YTS, The Pirate Bay, 1337x, BitTorrented.
- TV: EZTV, The Pirate Bay, 1337x, BitTorrented.
- Anime: Nyaa, SubsPlease.
- Games: FitGirl.
- Games results carry a visible warning: they are executables and can run code.
  Video and subtitle results cannot.
- Empty search browses a curated library per category.
- A new source appears in the Settings search group automatically. Phase 3 built that
  list from the indexer registry, so adding a source is still one file and one line.
- Each scraper is one file with its fixture next to it, so a broken site is a one file fix.

## Phase 5: headless

Target: BAKA is useful on a server with no terminal attached.

- `baka watch <dir>`: magnets, infohashes and `.torrent` files dropped in a directory
  are picked up and downloaded.
- `baka serve`: small HTTP endpoint that accepts magnets.
- `baka files`: serves finished downloads over HTTP.
- `--daemon`: detach and survive logout. On Windows this means a detached process plus a
  named pipe for control, not a service, unless a service turns out to be needed.
- Headless modes read the same `config.toml`. `baka settings` opens the page on its own so
  a server can be configured without the full TUI, and `baka settings --path` prints the
  file location for anyone who would rather use an editor.

## Phase 6: packaging

Target: install in one command.

- GitHub Actions release job builds `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`,
  attaches archives and an installer to the release, and publishes checksums.
- winget manifest submitted to `microsoft/winget-pkgs`, automated on tag.
- Scoop manifest in a `scoop-baka` bucket repo. Chocolatey package after winget is live.
- `cargo install baka` published to crates.io on the same tag.
- Uninstall leaves `config.toml` alone. Reinstalling keeps your settings.
- Release process documented in `CONTRIBUTING.md` and reduced to pushing a tag.

## Phase 7: 1.0

Target: it stays out of the way.

- `baka attach`: connect a TUI to a running daemon and survive an SSH drop.
- Config migration: a file written by an older version loads without losing settings.
- Search result caching so a repeated query is instant.
- Polish pass: startup time, memory under load, terminal resize, narrow terminals.

## Non-goals

- Hosting, indexing or mirroring any content. BAKA queries public sites and speaks
  BitTorrent. Nothing else.
- Settings anywhere except the Settings page and the file behind it. No env vars, no
  hidden keybinds that quietly persist state, no second config format.
- A web UI. librqbit ships one, and if a browser is wanted, rqbit is already the answer.
- A plugin system or a scripting language. Adding an indexer means adding a file.
- Configuration for its own sake. Every option must earn its place on the page.
