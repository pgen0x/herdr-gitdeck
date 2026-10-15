# Contributing

Issues and pull requests are welcome. For anything larger than a small fix, please open an
issue first so we can agree on the approach.

## Setup

```sh
GITDECK_FROM_SOURCE=1 bash herdr/install.sh   # build into bin/
herdr plugin link "$PWD"                      # run this checkout inside herdr
```

## Before you open a pull request

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three commands on Linux and macOS.

- Keep each commit to one change, with a [Conventional Commits](https://www.conventionalcommits.org)
  subject (`fix(diff): ...`, `feat(ui): ...`).
- Add a test for every bug fix. Tests that need git create a throwaway repo; see `TempRepo` in
  `src/git.rs`.
- Note user-facing changes under `## [Unreleased]` in `CHANGELOG.md`.

### Useful tools

- `GITDECK_LOG=/tmp/gitdeck.log` writes per-frame timings while the TUI runs.
- `GITDECK_ROOT=<dir> cargo test preview -- --ignored --nocapture` prints a 110×24 text
  render of the UI, which is how the README preview is made.

## Releasing

1. Bump `version` in both `herdr-plugin.toml` and `Cargo.toml` (they must match).
2. Move the `Unreleased` notes into a new `## [x.y.z] - YYYY-MM-DD` section of `CHANGELOG.md`.
3. Commit, then tag and push: `git tag vX.Y.Z && git push origin main vX.Y.Z`.

The `release` workflow checks that the tag matches the manifest. It then builds Linux (musl)
and macOS binaries for x86_64 and arm64, attaches them to the GitHub release with SHA-256
checksums and build provenance, and publishes the release once every asset is uploaded.
`herdr/install.sh` downloads exactly those assets.
