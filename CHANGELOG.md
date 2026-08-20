# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the
version is `0.y.z`, anything may change between releases.

## [Unreleased]

### Fixed

- The microphone permission dialog now actually appears. macOS never prompts on its own for
  audio reached through the CoreAudio HAL — it silently delivers zeros instead, which is how
  the first build recorded five seconds of perfect, empty audio without ever asking. Starting
  the meter or a probe now explicitly requests access through `AVCaptureDevice.requestAccess`
  first, and a refusal comes back as words naming the System Settings switch rather than as a
  meter that sits at zero.

### Changed

- The knowledge base now lives in `skia.db` alongside history and settings, instead of its own
  `skia-kb.db`. Its tables were always namespaced and separately versioned for this, so nothing
  about the schema changed — only where it is opened. One file is what lets a backup be a single
  consistent snapshot. An existing `skia-kb.db` is carried across on first launch and renamed
  aside rather than deleted.
- Licensed under Apache-2.0 instead of MIT, for its explicit patent grant and because it
  withholds trademark rights, keeping the project name and logo separate from the code grant.

### Added

- Retrieval understands meaning, not just words. A semantic index can be enabled in the
  Knowledge base section against any provider with an embeddings endpoint — Ollama runs it
  free and local, OpenAI and Gemini work with the key already in the keychain — and keyword
  and semantic results are fused by rank, so “money back” now finds the document that only
  says “refund”. Coverage is reported as a count of embedded chunks, and anything the index
  has not reached is still found by keywords; a broken or unconfigured index degrades to
  keyword-only rather than failing the question.
- The knowledge base reads PDF and Word documents. PDF text extraction runs behind a crash
  boundary so a malformed file is refused with its name rather than taking the app down;
  DOCX paragraphs are read straight out of the document XML. Citations for both quote the
  extracted text. Scanned PDFs with no text layer, and legacy `.doc` files, are refused
  with reasons instead of being indexed as nothing.
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
