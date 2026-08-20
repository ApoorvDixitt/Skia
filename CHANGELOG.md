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

- Modes decide what Skia can read, not just how it answers. Documents can be put into named
  collections in the Knowledge base section, and each use case — interview, meeting, sales,
  study, general — can be pointed at the collections it should see. Interview mode reaching
  your resume while meeting mode does not is enforced in retrieval itself, in both the keyword
  and the semantic arm, rather than asked of the model. Choosing no collections means every
  collection, said plainly, so a mode never narrows silently. Meeting transcripts stay out of
  scope regardless — no mode setting widens that.
- Backup and restore. One file holds everything — history, documents, embeddings, meetings —
  and it is taken with `VACUUM INTO` while Skia keeps running, so a backup is never a
  half-written copy missing its most recent work. A manifest beside it records the schema
  versions, a checksum, and what is deliberately *not* included: API keys stay in the OS
  keychain and must be re-entered after restoring on a new machine. Restoring validates the
  folder the moment you pick it, then applies at the next launch when nothing holds the
  database open; your previous data is moved aside rather than deleted, and a queued restore
  can be cancelled until then. A backup from a newer version of Skia, or a damaged one, is
  refused before anything is replaced.
- The overlay is a non-activating panel on macOS, which closes the two limitations it has
  shipped with: the dock icon can now be hidden, and the overlay no longer takes focus when it
  opens. It is ordered on screen before the app is demoted, and the dock icon is only hidden
  once the panel confirms it is visible — hiding it while the overlay is invisible was the
  failure this replaces. Typing into Ask keeps working, and the status panel now reports what
  actually took effect, including the native mechanism behind it, instead of a fixed answer.
- Meetings become memory. A new Meetings section starts a meeting with a title, profile and
  attendees, and shows a pre-meeting brief the moment it starts: prior meetings with these
  people and their still-open action items, assembled from data with no model call — facts
  must not wait on a provider. Notes typed during a meeting go through the transcript
  pipeline itself (append-only windows in the knowledge base, exact offsets, citable), and a
  meeting's transcript is retrievable from that meeting alone: a generic Ask never quotes a
  private meeting. People are recognised across meetings by email; commitments survive the
  deletion of a contact, unassigned rather than erased. Export and purge cover it all.
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
