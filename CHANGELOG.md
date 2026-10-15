# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-10-15

### Added

- Multi-repo dashboard: opens in any folder and lists every git repo below it, dirty
  repos first, with staged, changed and untracked counts, ahead/behind and branch.
- Background `git status` rescans every 3 seconds without taking `index.lock`.
- Unified and side-by-side split diff views with syntax highlighting.
- Stage, unstage and discard whole files or single hunks. Discards ask first and name
  their exact target.
- Commit staged changes from the pane.
- Line and file review comments, sent to the workspace's agent as one bracketed paste
  over herdr's socket API, with control characters stripped.
- Plugin actions `toggle`, `open` and `close`. Release binaries are downloaded and
  checksum-verified on install, with a source build as fallback.
- Responsive layout that stacks the panels in narrow panes.

[Unreleased]: https://github.com/pgen0x/herdr-gitdeck/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/pgen0x/herdr-gitdeck/releases/tag/v0.1.0
