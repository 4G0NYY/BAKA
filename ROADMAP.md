# BAKA Roadmap

BitTorrent Acquisition & Keyword Aggregator.

Goal: a single Rust binary that matches or beats [torlink](https://github.com/baairon/torlink)
in functionality, installs in one command on Windows, and stays small enough to read in an
afternoon.

Status: phases 0 to 6 are done. `baka` with no arguments is the whole product: search,
downloads, seeding and settings in one terminal interface, across every source torlink
has, and the same binary runs headless on a box with no terminal at all. Pushing a `v`
tag builds it for both Windows architectures and for Linux, and publishes it to
crates.io, Scoop, the AUR and the container registry. Phase 7, 1.0, is next.

## Decisions already made

| Question | Answer | Why |
| --- | --- | --- |
| Torrent engine | Embed [`librqbit`](https://crates.io/crates/librqbit) | Only mature pure-Rust engine with DHT, uTP, resume and seeding. Writing our own delays parity by months. |
| Interface | TUI first, CLI subcommands alongside | Matches torlink's shape and keeps headless and scripted use possible. |
| Settings | One Settings page, one TOML file, no environment variables | Every knob is in one place you can find without reading docs. |
| Sources | Same curated list as torlink | Parity out of the box, no setup for the user. |
| Platforms for 1.0 | Windows via Scoop, Arch via the AUR, a container image for the headless modes, plus `cargo install` and `cargo binstall` everywhere | Windows is the primary target. `cargo install` covers Linux and macOS for free, and `cargo binstall` makes that instant where a release archive exists. |
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
- `config.rs` holds the whole settings model: one struct per group, `Default` impls, and
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
It landed in phase 4, and the trait needed no second call: `parse` says which page a
magnet is on and the search loop fetches it.

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
  `s`, `Esc`, `?`, `q`. `?` opens the list of them. One `Esc` closes whatever is open,
  and a second one within the double tap window puts the interface back at its start.
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

## Phase 4: full source list (done)

Target was source parity with torlink. What shipped:

- Eight sources, one file and one fixture each: YTS, The Pirate Bay, 1337x and
  BitTorrented for films, EZTV, The Pirate Bay, 1337x and BitTorrented for shows, Nyaa
  and SubsPlease for anime, FitGirl for games.
- A source is asked an `Ask`: the words typed, or the shelf to list. An empty search
  box is a browse, and a browse is per category, so a source that serves two of them is
  asked for both listings. Nothing else changed shape, so `baka search` with no query
  browses too.
- A browse shows a share of every category in turn rather than sorting the lot by
  seeders, which would hand the whole screen to whichever category has the biggest
  swarms and bury the other three.
- `Indexer` grew from `url()` to `urls()`, which answers with every mirror worth
  trying and with nothing at all when a source has no listing or will not take a query
  that short. That covers 1337x being reachable on one host and blocked on the next,
  and BitTorrented sitting out a browse, without a second mechanism for either.
- The 1337x problem phase 1 predicted turned out to need no trait change. `parse`
  answers `Found::OnPage` for a result whose magnet is on a page of its own, and the
  search loop fetches the best seeded ten of those and reads the first magnet link out
  of each. Reading a magnet out of a page is the same job for every site, so it lives
  in the loop rather than in the source.
- Infohashes are normalised to hex, base32 magnets included, so the same release from
  Nyaa and from SubsPlease collapses into one result instead of two.
- Results carry the shelf they came from, in the TUI and in `baka search`, which is
  what makes a merged browse readable.

Four things found while building:

- 1337x is behind a Cloudflare challenge on `1337x.to` from some networks, including
  the one this was written on, and answers normally on `www.1377x.to`. That is why
  `urls()` returns a list rather than one address. No client without a browser engine
  can pass that challenge, so a host list is the whole of the answer available.
- apibay writes its numbers as strings in a search and as numbers in its top 100 lists.
  Both shapes have to parse or the browse and the search cannot share a scraper.
- 1337x answers a phrase with anything carrying one of its words, so its rows are
  filtered against the query before the second request is made rather than after. The
  request saved is the point: a junk row costs a whole page fetch.
- EZTV has no keyword search at all, only a feed of new releases. Rather than sit out
  every search, it keeps the entries in that feed that match what was typed, which is
  the honest version of what the API can do.

## Phase 5: headless (done)

Target was BAKA being useful on a server with no terminal attached. What shipped:

- `baka watch [dir]` picks up magnet links, bare infohashes and `.torrent` files. A
  handled file is renamed to `.taken`, or to `.failed` when there was nothing in it,
  so the next look leaves it alone and you can see which was which.
- `baka serve`: `POST` a magnet, an infohash or a file path, one per line. `GET` says
  what is running.
- `baka files`: the download folder over HTTP, with directory listings and range
  requests, so a browser or a player can open a finished film in place.
- `--daemon` on any of the three. It starts this same command without the flag,
  detached and writing nowhere. On Windows that is `DETACHED_PROCESS` plus its own
  process group; on Unix it is its own process group. No service, no named pipe.
- All three run the phase 3 queue on a one second tick, so the concurrency limits and
  stop at ratio mean the same thing with nobody watching, and Ctrl+C stops rather than
  kills.
- A Server group on the Settings page: bind address, both ports and the watch folder.
  The bind address defaults to loopback, because the magnet intake downloads whatever
  it is handed and reaching it from the rest of the network should be a decision.
- The HTTP is written here rather than pulled in. It is a request line, the headers
  worth reading, a body, and a response, and it fits in one file with its tests.

Three things found while building:

- Adding a magnet blocks until peers hand over the file list, which phase 2 already
  knew. It matters more here: a watched folder with ten things in it worked through
  them one at a time, and an HTTP client sat on an open request for the whole wait. All
  three modes now hand the add to a task and answer with what they understood.
- A file still being copied into the watched folder is not ready to be read. How long
  ago it was written is the only signal available without watching the filesystem
  itself, so a file has to have been still for one look before it is taken.
- Path safety needs both halves. Refusing `..` in a request stops the obvious climb,
  and comparing the real path both sides resolve to stops a link inside the folder
  pointing out of it.

Left as it is: one BAKA at a time. The interface, `baka get` and the headless modes all
open the same session, which has been true since phase 2. `baka attach` in phase 7 is
what makes a second one useful rather than a conflict.

## Phase 6: packaging (done)

Target was installing in one command. What shipped:

- `.gitlab-ci.yml` on a `v*` tag. It builds `x86_64-pc-windows-msvc` and
  `aarch64-pc-windows-msvc` on a Windows runner and `x86_64-unknown-linux-gnu` in a
  container, packs each with the README and the licence, and attaches all three plus
  `SHA256SUMS.txt` to a GitLab release. `.github/workflows/release.yml` still does the
  Windows half of that for anyone building this on GitHub.
- The tag is compared against the version in `Cargo.toml` before anything is built. A
  mismatch is the one release mistake that cannot be taken back, because crates.io does
  not let a version be republished.
- The Scoop bucket is this repository. The release job writes `bucket/baka.json` on
  `main`, and `scoop bucket add baka https://gitlab.ramon.moe/4G0NYY/BAKA` is what points Scoop
  at it.
- `cargo publish` on the same tag, which is what `cargo install b-baka` reads.
- `[package.metadata.binstall]` in `Cargo.toml`, so `cargo binstall b-baka` takes the
  release archive for the target it is running on and falls back to compiling when there
  is none.
- `packaging/aur/PKGBUILD`, pushed to `aur.archlinux.org/b-baka.git` with a generated
  `.SRCINFO`. It builds from the crates.io tarball rather than a binary, so one PKGBUILD
  covers x86_64 and aarch64 and nothing has to be rebuilt when a release adds a target.
- A `Dockerfile` at the root and a `container` job that pushes the version and `latest`
  to the project registry. It carries the binary and a root store, and its default
  command is `serve`, because a container is where the headless modes belong.
- `scripts/manifests.sh` fills the version and the checksums into the Scoop manifest and
  the PKGBUILD and refuses to write a file with a placeholder left in it. CI renders them
  on every merge request, so a template that no longer matches its script fails then
  rather than during a release.
- Every route installs the executable and nothing else. Unpacking a built archive and
  asking the binary where its settings are answers `%APPDATA%\baka\config.toml`, which
  is not a path any of the uninstalls can reach.
- `CONTRIBUTING.md` has the checks to run, the three steps before a tag, what each job in
  the release does, and which secret each one needs.

Six things found while building:

- This phase was written as archives plus an installer. A portable executable does not
  need one. An MSI would have added a toolchain to CI and a second uninstall path aimed
  at the only thing worth protecting.
- A Scoop bucket does not need a repository of its own. Any repository with a `bucket`
  directory is one, so it lives here: one release job, and no second repository to keep
  in step with this one.
- aarch64 compiles the whole tree, `aws-lc-sys` included, and then needs the ARM64 MSVC
  linker to finish. Without it `rustc` falls back to whatever `link.exe` is on `PATH`,
  which on a machine with Git for Windows is coreutils, and the error it prints has
  nothing to do with the code.
- A missing secret skips its job instead of failing the release. A release that cannot
  reach crates.io should still put archives on the release page.
- A GitLab release carries links, not files, and a link is only reachable at
  `/-/releases/<tag>/downloads/<name>` if it was attached with a `filepath`. Without one
  the release page looks finished and every install route that reads that permalink is
  broken, which is the worst shape a release bug can take.
- A Windows container image is not a Windows build machine. `servercore` ships neither
  rustup nor the MSVC toolchain, and installing them per job costs more than the build,
  so the Windows jobs run on a shell runner with the toolchain on the machine.

Left open: winget, which submits by opening a pull request against a GitHub repository
and therefore left with GitHub, and Chocolatey, which was waiting on winget and now has
nothing to wait for. Its manifests were removed rather than kept as a template nothing
renders.

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
