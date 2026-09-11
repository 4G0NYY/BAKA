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

A release is a tag. Everything except winget happens in
[`.gitlab-ci.yml`](.gitlab-ci.yml).

1. `main` is green.
2. Bump `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` follows. Commit both.
3. `git tag v0.2.0 && git push origin v0.2.0`.

The tag refuses to build if it does not match the version in `Cargo.toml`, which is the
one mistake that would publish the wrong number to crates.io.

What the tag then does:

| Job | Runner | Result |
| --- | --- | --- |
| `version` | docker | Refuses the tag if it disagrees with `Cargo.toml`, and hands the number to every job below |
| `build:windows` | windows | `baka.exe` for `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, each zipped with the README and the licence |
| `build:linux` | docker | `baka` for `x86_64-unknown-linux-gnu`, the same contents as a `.tar.gz` |
| `release` | docker | Uploads all three archives and `SHA256SUMS.txt` to the generic package registry and creates the release pointing at them |
| `crates` | docker | `cargo publish`, which is what `cargo install b-baka` and `cargo binstall b-baka` read |
| `scoop` | docker | `bucket/baka.json` on `main`, pointed at the new archives |
| `aur` | docker | `PKGBUILD` and `.SRCINFO` pushed to `aur.archlinux.org/b-baka.git`, built from the crates.io tarball |
| `container` | docker | `registry.ramon.moe/4g0nyy/baka` tagged with the version and `latest` |

Each release asset is attached with a `filepath`, which is what makes
`/-/releases/<tag>/downloads/<file>` resolve. Scoop and the `binstall` metadata in
`Cargo.toml` are both written against that permalink, so dropping the `filepath` breaks
both installs while leaving the release looking correct.

winget only accepts pull requests from a GitHub account, so it is the one job on GitHub.
[`.github/workflows/winget.yml`](.github/workflows/winget.yml) runs when the mirror
brings the tag over. It waits for the GitLab release, checks the archives against its
`SHA256SUMS.txt`, copies them to a GitHub release (the winget manifest points there),
and has `wingetcreate` open the pull request against `microsoft/winget-pkgs`. It builds
nothing.

The manifests are templates. `scripts/manifests.sh` fills in the version and the
checksums and refuses to write a file with a placeholder left in it, and CI renders them
on every merge request so a broken template is caught before a release needs them.

### Secrets

| Secret | Used by | Missing means |
| --- | --- | --- |
| `CARGO_REGISTRY_TOKEN` | `crates` | The crates.io publish is skipped, the rest of the release still happens |
| `BAKA_BUCKET_TOKEN` | `scoop` | The bucket is not updated. It needs a project access token with `write_repository` |
| `AUR_SSH_KEY` | `aur` | The AUR package is not updated. It needs a private key whose public half is on an AUR account that maintains `b-baka`, added as a **File** variable: GitLab cannot mask a multi line value, so a key in an ordinary variable is a key waiting to be printed |

`container` needs no secret: `CI_JOB_TOKEN` is what it logs in to the registry with.

One secret lives on GitHub instead, under the repository's Actions secrets:

| Secret | Used by | Missing means |
| --- | --- | --- |
| `WINGET_TOKEN` | `winget.yml` | The GitHub release is still created, but no winget pull request is opened. It needs a classic personal access token with `public_repo` from the account that owns the `winget-pkgs` fork |

### Runners

`build:windows` and `test:windows` are tagged `windows` and run on a shell runner, not a
containerised one. A `servercore` image carries neither rustup nor the MSVC toolchain,
and installing several gigabytes of build tools per job costs more than the build does,
so the toolchain lives on the machine. What that runner's service account needs on its
`PATH`: `cargo`, `rustup`, and the MSVC linker for both `x86_64` and `aarch64`.

### The first release only

- Reserve `b-baka` on crates.io by publishing once. Nothing else in the release can do it
  for you. The package is `b-baka` because `baka` was taken, the binary it installs is
  still `baka`.
- Reserve `b-baka` on the AUR the same way. The `aur` job pushes to a repository AUR
  creates on first submission, but it will not claim a name for you.
- `cargo binstall` reads its metadata from the published crate, so it only starts working
  from the first version published after that metadata landed.
- `bucket/baka.json` is written by the release job, so `scoop bucket add baka
  https://gitlab.ramon.moe/4G0NYY/BAKA` only works once there has been a release.

### What a release does not touch

Every route installs a portable executable and nothing else, so uninstalling removes the
binary and leaves `config.toml` and the download folder alone. Reinstalling picks up the
settings that were already there. Keep it that way: an installer that writes to
`%APPDATA%` is an installer that can delete it.
