# Architecture

This document describes how Skia is put together and where new code belongs. It describes the
**target** design — most of it is not built yet. See the [roadmap](ROADMAP.md) for what
actually exists.

## Shape of the thing

Skia is a single [Tauri v2](https://tauri.app) desktop application. The frontend is React +
TypeScript in a webview; everything that needs native access, real-time performance, or the
filesystem lives in Rust and is reached over Tauri's IPC.

```
┌──────────────────────────────────────────────────────────────┐
│  Skia (Tauri v2)                                             │
│                                                              │
│  React / TS UI  ──IPC──►  Rust core                          │
│  (overlay,                ├─ window + overlay manager        │
│   panels,                 ├─ audio engine  ◄── isolated      │
│   settings)               │   (mic + far end, 2 streams)     │
│                           ├─ speech to text                  │
│                           ├─ retrieval (sqlite-vec + FTS5)   │
│                           ├─ model gateway client            │
│                           ├─ prompt orchestrator             │
│                           ├─ secrets (OS keychain)           │
│                           └─ updater (minisign)              │
│                                                              │
│  Local SQLite: history, sessions, KB vectors, settings       │
└──────────────────────────────────────────────────────────────┘
     ▲ microphone     ▲ system / far-end audio     ▲ HTTPS
   CoreAudio/WASAPI  ScreenCaptureKit/WASAPI loop  providers, GitHub
```

One repository builds both platforms from the same commit. Platform differences are handled
with `#[cfg(target_os = "...")]` and per-target Cargo dependencies, not separate branches.

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri v2 (Rust) |
| Frontend | React, TypeScript, Vite |
| Storage | SQLite with FTS5 |
| Vector search | sqlite-vec |
| Embedding and reranking | bge-m3, bge-reranker (local) |
| Speech to text | Deepgram Nova-3 (cloud) or whisper-rs (local) |
| Audio capture | cpal, WASAPI loopback, ScreenCaptureKit / CoreAudio |
| Resampling | rubato |
| Model access | OpenAI-compatible providers, OpenRouter, Ollama |
| Secrets | OS keychain |
| Updates | tauri-plugin-updater with minisign |

The frontend deliberately stays platform-agnostic. Heavy or native work belongs in Rust.

## Target layout

```
Skia/
├─ src/                     # React + TypeScript frontend
│  ├─ overlay/  ask/  live/  kb/
│  ├─ providers/            # model catalog, gateway client, routing
│  ├─ prompts/              # default system prompts and profiles
│  └─ lib/testing/          # dev panel, mock provider, retrieval eval
├─ src-tauri/src/
│  ├─ overlay.rs            # window and capture behaviour, cfg-gated per OS
│  ├─ audio/                # capture, device hot-swap, resampling
│  ├─ stt/                  # transcription backends and endpointing
│  ├─ rag/                  # sqlite-vec + FTS5 + reranking
│  ├─ updater.rs
│  └─ lib.rs                # setup(), IPC commands, worker supervision
├─ src-tauri/tauri.conf.json
└─ .github/workflows/
```

Directories are created as the code that needs them lands, rather than up front as empty
placeholders.

## Design decisions worth knowing

These are the non-obvious constraints. Changing them affects the whole app.

**Two audio streams, never merged.** The microphone and the far-end/loopback capture stay
independent all the way to transcription. That separation is what makes speaker labelling
possible, so OS echo cancellation must not be applied — it strips the loopback signal.
Microphone and loopback arrive at different sample rates and are both resampled to 16 kHz mono
before transcription.

**Audio device hot-swap is the main crash risk.** Users switch to headphones mid-call. The
audio engine subscribes to default-device-change callbacks and rebuilds its streams instead of
assuming a stable device.

**The audio engine is isolated from the UI.** It runs on its own thread or sidecar process
behind an IPC boundary and is supervised, so a panic in real-time audio code cannot take the
webview down with it.

**Endpointing dominates perceived latency, not retrieval.** Deciding that the speaker has
finished costs 150–500 ms; local retrieval costs well under 100 ms. So retrieval is fired
speculatively against partial transcripts — off the critical path — and effort goes into
endpointing and speculative generation instead of shaving milliseconds off the database.

Latency budget for the fast path, question to first visible token, targeting under one second:

| Stage | Target (P50) |
|---|---|
| Transcription partial | 40–80 ms |
| End-of-utterance detection | 150–500 ms |
| Retrieval | ≤ 80 ms |
| Reranking | ≤ 30 ms |
| Model time to first token | 0.3–0.9 s |

Network round-trip to a cloud provider is outside our control and is surfaced honestly in the
in-app latency view rather than hidden.

**Retrieval fuses keyword and vector search.** FTS5 BM25 and sqlite-vec run in parallel, their
results are combined with reciprocal rank fusion, and the top results are reranked. Chunks
keep character offsets so an answer can cite the exact source span. A lightweight gate decides
whether a turn needs the knowledge base at all, so small talk skips lookup entirely.

**Capture protection is per-OS and honest.** Windows uses `WDA_EXCLUDEFROMCAPTURE`; macOS 14
and earlier use `NSWindow.sharingType = .none`. On macOS 15+ the flag is ignored by modern
capture APIs. The UI must reflect what is actually active on the current OS and never present
a single boolean implying more. See the matrix in the [README](../README.md#what-quiet-actually-means).

**Nothing leaves the device unless the user configured it.** No backend, no accounts, no
telemetry. Outbound traffic goes only to the model provider the user chose and to GitHub for
update checks. API keys live in the OS keychain, never in config files or logs.

## Open unknowns

Two questions are load-bearing enough to answer before building on top of them, and both are
Phase 0 work:

1. ~~**What exactly leaks on current macOS?**~~ **Measured on macOS 26.5** — see the
   [harness](../tools/macos-capture-harness). `sharingType = .none` *is* honoured by
   ScreenCaptureKit, by legacy CoreGraphics capture, and by full-screen shares in Google Meet
   and Zoom. But it is undocumented, Apple's own docs advise against relying on it, and there
   is an open Apple bug where exclusion breaks after a capture filter is rebuilt — so it is a
   bonus, not a guarantee, and the harness must be re-run each macOS release.
   **Still open:** exclusion covers *pixels only*. The window remains enumerable via
   `SCShareableContent` and `CGWindowListCopyWindowInfo`, which expose its owner process,
   geometry, and `sharingState`. There is no public way to hide a window's existence.
2. **Does the unsigned in-place update survive?** Ad-hoc signatures are fragile, and a botched
   in-place replacement produces a macOS "app is damaged" error. This needs testing on current
   macOS before anyone relies on auto-update.

A third, smaller one: macOS 14.4+ offers a CoreAudio process-tap API that captures per-process
audio without the alarming screen-recording permission prompt. ScreenCaptureKit is the default
path; the process tap is worth evaluating as a cleaner replacement.
