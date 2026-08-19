# Contributing

## Rules

Read [CLAUDE.md](CLAUDE.md) first. It is written for Claude Code and the rules in it
apply to everyone. The one about em dashes is enforced by CI.

## Before a pull request

| Command | Checks |
| --- | --- |
| `cargo fmt --check` | Formatting |
| `cargo clippy --all-targets -- -D warnings` | Lints, warnings are errors |
| `cargo test` | Unit tests and scraper fixtures, never the network |
| `bash scripts/no-em-dashes.sh` | No em dash anywhere in the tree |

New behaviour needs a test. A new source needs a saved fixture, one file in `src/search/`
and one line in the registry, and nothing else.

## Releasing

A release is a tag. Everything else happens in
[`.github/workflows/release.yml`](.github/workflows/release.yml).

1. `main` is green.
2. Bump `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` follows. Commit both.
3. `git tag v0.2.0 && git push origin v0.2.0`.

The tag refuses to build if it does not match the version in `Cargo.toml`, which is the
one mistake that would publish the wrong number to crates.io.

What the tag then does:

| Job | Result |
| --- | --- |
| `build` | `baka.exe` for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, each zipped with the README and the licence |
| `release` | A GitHub release carrying both archives and `SHA256SUMS.txt` |
| `crates` | `cargo publish`, which is what `cargo install b-baka` reads |
| `winget` | A pull request against `microsoft/winget-pkgs` from the manifests in `packaging/winget/` |
| `scoop` | `bucket/baka.json` on `main`, pointed at the new archives |

The manifests are templates. `scripts/manifests.sh` fills in the version and the two
checksums and refuses to write a file with a placeholder left in it, and CI renders them
on every pull request so a broken template is caught before a release needs them.

### Secrets

| Secret | Used by | Missing means |
| --- | --- | --- |
| `CARGO_REGISTRY_TOKEN` | `crates` | The crates.io publish is skipped, the rest of the release still happens |
| `WINGET_TOKEN` | `winget` | The winget pull request is skipped. It needs a classic PAT with `public_repo` on an account that has forked `microsoft/winget-pkgs` |

### The first release only

- Reserve `b-baka` on crates.io by publishing once. Nothing else in the release can do it
  for you. The package is `b-baka` because `baka` was taken, the binary it installs is
  still `baka`.
- The first winget submission is reviewed by a person, so it lands later than the rest.
- `bucket/baka.json` is written by the release job, so `scoop bucket add baka
  https://gitlab.ramon.moe/4G0NYY/BAKA` only works once there has been a release.

### What a release does not touch

Both winget and Scoop install a portable executable and nothing else, so uninstalling
removes the binary and leaves `config.toml` and the download folder alone. Reinstalling
picks up the settings that were already there. Keep it that way: an installer that writes
to `%APPDATA%` is an installer that can delete it.
