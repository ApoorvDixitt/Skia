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

- The microphone half of the audio engine: capture from the default input on a dedicated,
  supervised thread, downmix and resample to the 16 kHz mono every transcription backend
  expects, and rebuild the stream when the default device changes — debounced, because a
  Bluetooth profile switch presents as several changes in quick succession. A new Audio
  section in the dashboard shows a live level meter, the input device list, and records a
  five-second WAV sample of exactly what a transcriber would receive. A recording of pure
  zeros is reported as the permission problem it is (measured: capture without consent on
  macOS succeeds and returns silence), never as success. The macOS bundle now declares
  `NSMicrophoneUsageDescription` and `NSAudioCaptureUsageDescription`, without which the
  first capture would abort the app or silently produce nothing.

- Tauri v2 application shell with a React + TypeScript frontend.
- Project documentation: architecture overview, phased roadmap, and release process.
- Continuous integration for macOS and Windows, and a tag-driven release workflow. CI runs the
  Rust test suite on both platforms, so the tests gate a pull request rather than only being
  runnable by hand.

[Unreleased]: https://github.com/ApoorvDixitt/Skia/commits/main
