# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

BAKA (BitTorrent Acquisition & Keyword Aggregator) is a terminal torrent search and
download tool written in Rust. It is a from-scratch answer to
[torlink](https://github.com/baairon/torlink), which is Node based. Same job, one binary,
no runtime to install.

See `ROADMAP.md` for what is built and what is next. Read it before starting work.

## Hard rules

These are not preferences. Breaking one means the change gets reverted.

### 1. No em dashes anywhere

Not in documentation, not in comments, not in commit messages, not in CLI output, not in
TUI strings. Use a comma, a colon, parentheses, or two sentences.

CI enforces this. To check locally:

```
rg -n --hidden -g '!.git' '\x{2014}'
```

No output means clean. The pattern is written as a hex escape so this file does not trip
its own check. Do not paste the literal character to "fix" the pattern.

### 2. Simplicity is the point

If code needs a comment explaining what it does, rewrite it until it does not.
The rewrite is the fix. The comment is not.

- No abstraction with a single implementation "for later".
- No trait, generic or macro that does not remove real duplication today.
- No configuration option nobody asked for.
- Prefer a longer, obvious function over a clever short one.

### 3. Comments explain why, never what

```rust
// Trackers rate limit aggressively, so one failure is expected and not worth surfacing.
let results = join_all(queries).await;
```

Not:

```rust
// Run all the queries and wait for them.
let results = join_all(queries).await;
```

Delete any comment that restates the line below it.

### 4. Docs are concise, then complete

Short enough to read in full, detailed enough to act on. No marketing voice, no filler
intros, no "in today's fast paced world". If a section can be a table, make it a table.

### 5. Every setting lives on the Settings page

One `Settings` struct in `config.rs`, one `config.toml`, one page in the TUI that edits it.
No environment variables. No persistent flags. No second config file.

A CLI flag may override a setting for one run, but it must never write to disk. If you add
a field to `Settings`, it appears on the page in the same change, or the change is not done.

### 6. The UI carries the name

BAKA is the product name and the tone. The interface is playful in its branding and
completely serious in its behaviour. No joke output during errors, transfers or anything
destructive.

## Layout

One crate, modules by responsibility. Do not split into a workspace without a reason
that shows up in a build log.

```
src/
  main.rs      clap subcommands, dispatch, nothing else
  config.rs    TOML config and platform paths
  search/      Indexer trait plus one file per source
  engine.rs    the only module that knows librqbit exists
  tui/         ratatui views and key handling
  server.rs    watch, serve and files modes
```

Rules that follow from this:

- librqbit types never leave `engine.rs`. The rest of the codebase uses our own types.
- A new source is one new file in `search/` and one line in its registry. If adding a
  source needs changes elsewhere, the trait is wrong.
- `tui/` renders state and emits intents. It does not perform network or disk work.

## Conventions

- `cargo fmt` and `cargo clippy -- -D warnings` both pass before anything is considered done.
- No `unwrap` or `expect` outside tests and `main`. Use `anyhow::Result` at the boundary,
  `thiserror` for errors a caller might match on.
- Scrapers are tested against saved fixtures. CI never touches the network.
- One dead source never fails a search. Timeout it, drop it, keep the rest.
- User facing strings live next to the code that shows them. No string table.
- Dependencies are added deliberately. If a dep is used for one function, write the function.

## Commands

```
cargo run              launch the TUI
cargo test             unit and fixture tests
cargo clippy -- -D warnings
cargo fmt --check
rg -n --hidden -g '!.git' '\x{2014}'
```

## Definition of done

1. It compiles, `clippy` is clean and `fmt` is clean.
2. New behaviour has a test. Scrapers have a fixture.
3. The em dash check passes.
4. `ROADMAP.md` reflects reality if a phase item moved.
5. `README.md` documents the feature if a user can see it.

## Scope

BAKA searches public indexers and speaks BitTorrent. It does not host, index or mirror
content. Keep it that way. Do not add trackers of our own, content databases or anything
that turns a client into a service.
