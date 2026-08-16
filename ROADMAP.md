# BAKA Roadmap

BitTorrent Acquisition & Keyword Aggregator.

Goal: a single Rust binary that matches or beats [torlink](https://github.com/baairon/torlink)
in functionality, installs in one command on Windows, and stays small enough to read in an
afternoon.

Status: phases 0 and 1 are done. `baka search` returns real results. Phase 2 is next.

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

The Settings page is not a phase 7 nicety. It ships with the TUI in phase 3, because a
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

- Edition 2024, MSRV 1.85 pinned and checked in CI. 1.85 is both the edition floor and the
  highest MSRV any dependency asks for.
- Modules stubbed with the rule that governs each one: `search`, `engine`, `tui`, `server`.
- `config.rs` holds the whole settings model: five grouped structs, `Default` impls, and
  load and save to TOML under the platform config dir via `directories`.
- `Settings::fields()` returns every setting with its group, label, description and a
  mutable handle to the value. The phase 3 page renders and edits that list, so a field
  missing from it cannot be reached by a user.
- Missing files, partial files and keys from a newer version all load without an error.
- `baka` prints the art from `stuff/` plus version and settings path. `baka settings`
  lists every setting and writes the file on first run. `baka settings --path` prints
  the path alone.
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

## Phase 2: engine

Target: `baka get <magnet|infohash|path>` downloads, resumes and seeds.

- Wrap a librqbit `Session` behind a small `Engine` type. Nothing outside `engine.rs`
  knows librqbit exists.
- Accept magnet links, bare 40 character hex or 32 character base32 infohashes, and
  local `.torrent` paths.
- The engine takes its limits from `Settings`. It never reads env vars.
- Session state persisted so an interrupted download resumes on the next start.
- Seed after completion by default.
- Progress reported as a stream of snapshots the TUI can poll.

## Phase 3: TUI and settings

Target: `baka` with no arguments is the whole product.

- `ratatui` plus `crossterm`. Tabs: Search, Downloads, Seeding, Settings.
- Search runs without blocking the UI. Downloads keep running while the user searches.
- Downloads tab shows progress, speed and ETA. Seeding tab pauses or stops.
- Settings tab covers, at minimum:

| Group | Settings |
| --- | --- |
| Downloads | Download folder, maximum concurrent downloads, download rate limit, ask for a folder per download |
| Seeding | Seed after completion, maximum concurrent seeds, upload rate limit, stop at ratio |
| Network | Listen port, DHT on or off, UPnP port mapping, peer limit per torrent |
| Search | Enabled sources, per source timeout, result limit, hide results below a seeder count |
| Interface | Accent colour, confirm before removing, show game source warnings |

- Changes save when you leave the row and apply live wherever the engine allows it.
  Anything that needs a restart says so on the row instead of failing quietly.
- Keys: `/` search, `Enter` run, `Tab` switch tab, `j`/`k` or arrows move, `d` download,
  `D` download to a chosen folder, `p` pause or resume, `x` stop, `c` copy magnet,
  `s` settings, `?` help, `q` quit.
- Branding lands here: BAKA wordmark on the empty state, a name expansion line, and a
  consistent accent colour. Loud enough to be recognisable, quiet enough to use daily.

## Phase 4: full source list

Target: source parity with torlink.

- Movies: YTS, The Pirate Bay, 1337x, BitTorrented.
- TV: EZTV, The Pirate Bay, 1337x, BitTorrented.
- Anime: Nyaa, SubsPlease.
- Games: FitGirl.
- Games results carry a visible warning: they are executables and can run code.
  Video and subtitle results cannot.
- Empty search browses a curated library per category.
- A new source appears in the Settings search group automatically. Adding a source must
  never mean hand editing the settings list.
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
