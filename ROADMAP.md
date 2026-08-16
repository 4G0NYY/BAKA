# BAKA Roadmap

BitTorrent Acquisition & Keyword Aggregator.

Goal: a single Rust binary that matches or beats [torlink](https://github.com/baairon/torlink)
in functionality, installs in one command on Windows, and stays small enough to read in an
afternoon.

Status: nothing is built yet. Phase 0 is the current work.

## Decisions already made

| Question | Answer | Why |
| --- | --- | --- |
| Torrent engine | Embed [`librqbit`](https://crates.io/crates/librqbit) | Only mature pure-Rust engine with DHT, uTP, resume and seeding. Writing our own delays parity by months. |
| Interface | TUI first, CLI subcommands alongside | Matches torlink's shape and keeps headless and scripted use possible. |
| Sources | Same curated list as torlink | Parity out of the box, no setup for the user. |
| Platforms for 1.0 | Windows via winget, Scoop and Chocolatey, plus `cargo install baka` everywhere | Windows is the primary target. `cargo install` covers Linux and macOS for free. |
| Layout | One crate, several modules | Five crates for a tool this size is overhead, not structure. Split only when compile times justify it. |

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
| Downloads folder default, per-download override, changeable default | 3 |
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

## Phase 0: foundations

Target: `baka --version` runs.

- `cargo init`, edition 2024, MSRV pinned in `Cargo.toml`.
- Modules stubbed: `config`, `search`, `engine`, `tui`, `server`.
- `config.rs`: TOML at the platform config dir via `directories`, with defaults so a
  missing file is never an error.
- Error handling: `anyhow` at the binary boundary, `thiserror` for anything a caller
  might want to match on.
- CI on GitHub Actions: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`,
  and the em dash check from CLAUDE.md.

## Phase 1: search

Target: `baka search "query"` prints ranked results.

- `Indexer` trait: takes a query and a category, returns `Vec<Torrent>`.
- `Torrent`: title, size, seeders, leechers, category, source, magnet or torrent URL.
- First two indexers: YTS (clean JSON API, easy) and Nyaa (RSS, easy).
- All indexers queried concurrently, each with its own timeout. One dead site never
  stalls or fails the search.
- Ranking: seeders first, with an obvious title match bonus. Deduplicate by infohash.
- Plain stdout table so search is testable before any TUI exists.
- Fixture-based tests: saved HTML and JSON responses parsed offline, no network in CI.

## Phase 2: engine

Target: `baka get <magnet|infohash|path>` downloads, resumes and seeds.

- Wrap a librqbit `Session` behind a small `Engine` type. Nothing outside `engine.rs`
  knows librqbit exists.
- Accept magnet links, bare 40 character hex or 32 character base32 infohashes, and
  local `.torrent` paths.
- Session state persisted so an interrupted download resumes on the next start.
- Seed after completion by default.
- Progress reported as a stream of snapshots the TUI can poll.

## Phase 3: TUI

Target: `baka` with no arguments is the whole product.

- `ratatui` plus `crossterm`. Tabs: Search, Downloads, Seeding.
- Search runs without blocking the UI. Downloads keep running while the user searches.
- Downloads tab shows progress, speed and ETA. Seeding tab pauses or stops.
- Keys: `/` search, `Enter` run, `Tab` switch tab, `j`/`k` or arrows move, `d` download,
  `D` download to a chosen folder, `o` change the default folder, `p` pause or resume,
  `x` stop, `c` copy magnet, `?` help, `q` quit.
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
- Each scraper is one file with its fixture next to it, so a broken site is a one file fix.

## Phase 5: headless

Target: BAKA is useful on a server with no terminal attached.

- `baka watch <dir>`: magnets, infohashes and `.torrent` files dropped in a directory
  are picked up and downloaded.
- `baka serve`: small HTTP endpoint that accepts magnets.
- `baka files`: serves finished downloads over HTTP.
- `--daemon`: detach and survive logout. On Windows this means a detached process plus a
  named pipe for control, not a service, unless a service turns out to be needed.

## Phase 6: packaging

Target: install in one command.

- GitHub Actions release job builds `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`,
  attaches archives and an installer to the release, and publishes checksums.
- winget manifest submitted to `microsoft/winget-pkgs`, automated on tag.
- Scoop manifest in a `scoop-baka` bucket repo. Chocolatey package after winget is live.
- `cargo install baka` published to crates.io on the same tag.
- Release process documented in `CONTRIBUTING.md` and reduced to pushing a tag.

## Phase 7: 1.0

Target: it stays out of the way.

- `baka attach`: connect a TUI to a running daemon and survive an SSH drop.
- Config for custom download paths, rate limits, port and theme.
- Search result caching so a repeated query is instant.
- Polish pass: startup time, memory under load, terminal resize, narrow terminals.

## Non-goals

- Hosting, indexing or mirroring any content. BAKA queries public sites and speaks
  BitTorrent. Nothing else.
- A web UI. librqbit ships one, and if a browser is wanted, rqbit is already the answer.
- A plugin system or a scripting language. Adding an indexer means adding a file.
- Configuration for its own sake. Every option must earn its place.
