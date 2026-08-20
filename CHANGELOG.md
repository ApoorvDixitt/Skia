# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the
version is `0.y.z`, anything may change between releases.

## [Unreleased]

### Changed

- The knowledge base now lives in `skia.db` alongside history and settings, instead of its own
  `skia-kb.db`. Its tables were always namespaced and separately versioned for this, so nothing
  about the schema changed — only where it is opened. One file is what lets a backup be a single
  consistent snapshot. An existing `skia-kb.db` is carried across on first launch and renamed
  aside rather than deleted.
- Licensed under Apache-2.0 instead of MIT, for its explicit patent grant and because it
  withholds trademark rights, keeping the project name and logo separate from the code grant.

### Added

- Tauri v2 application shell with a React + TypeScript frontend.
- Project documentation: architecture overview, phased roadmap, and release process.
- Continuous integration for macOS and Windows, and a tag-driven release workflow. CI runs the
  Rust test suite on both platforms, so the tests gate a pull request rather than only being
  runnable by hand.

[Unreleased]: https://github.com/ApoorvDixitt/Skia/commits/main
