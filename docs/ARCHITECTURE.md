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
| Vector search | brute-force cosine in Rust over BLOB embeddings (linear scan is inside the latency budget at personal-KB scale; sqlite-vec deferred until measured otherwise) |
| Embeddings | any OpenAI-compatible `/embeddings` endpoint — Ollama locally, OpenAI/Gemini via BYOK; reranking deferred (needs a local model runtime) |
| Speech to text | Deepgram Nova-3 (cloud) or whisper-rs (local) |
| Audio capture | cpal (mic), CoreAudio process taps (macOS far end), WASAPI loopback (Windows far end) |
| Resampling | rubato |
| Model access | OpenAI-compatible providers, OpenRouter, Ollama |
| Secrets | OS keychain |
| Updates | tauri-plugin-updater with minisign |

The frontend deliberately stays platform-agnostic. Heavy or native work belongs in Rust.

## Target layout

```
Skia/
├─ src/                     # React + TypeScript, one bundle, two windows
│  ├─ overlay/              # the compact always-on-top bar
│  ├─ dashboard/            # knowledge base, history, providers, prompts, status
│  ├─ lib/                  # IPC layer, validated not cast
│  └─ styles/tokens.css     # the design system
├─ src-tauri/src/
│  ├─ stealth.rs            # capture exclusion + presence, and honest reporting of both
│  ├─ catalog.rs            # the bring-your-own-key provider catalog
│  ├─ providers/            # one OpenAI-compatible streaming client + mock
│  ├─ secrets/              # OS keychain, behind a trait so it is testable
│  ├─ prompts/              # shipped defaults, profiles, strict interpolation
│  ├─ rag/                  # chunking, FTS5 retrieval, citations
│  ├─ storage/              # sessions, messages, settings
│  ├─ audio/                # not built: capture, device hot-swap, resampling
│  ├─ stt/                  # not built: transcription and endpointing
│  └─ lib.rs                # setup(), IPC commands, window wiring
├─ src-tauri/tauri.conf.json
└─ .github/workflows/
```

## Two windows, on purpose

The frontend is one bundle that renders a different surface depending on the
window label, chosen in `src/App.tsx`:

- **`overlay`** — frameless, transparent, always on top, ~680×92 and resized from
  the frontend as an answer grows. This is the in-call surface, so it holds only
  what can be read mid-conversation.
- **`dashboard`** — an ordinary 1040×720 window, created hidden and shown on
  demand. Everything that needs room lives here.

The split exists because a single window cannot be both. An earlier version put
the stealth status, Ask, and history in one 720×620 window, and the result read as
a diagnostics page rather than an app: the honest capture-status panel, which is
essential but verbose, crowded out the thing the user actually came to do. Moving
it to a dashboard section lets the overlay compress the same information into a
status dot with a tooltip, without dropping any of it.

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

**Capture protection needs no hand-written native code.** Tauri's `set_content_protected`
already maps to `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` on Windows and
`NSWindow.sharingType = .none` on macOS — verified by reading `tao`'s platform implementations,
and exactly the two mechanisms the [harness](../tools/macos-capture-harness) measured. So Skia
depends on neither `objc2` nor the `windows` crate for this, and carries no `unsafe` blocks.
The same applies to presence invisibility: `set_activation_policy(Accessory)`,
`set_skip_taskbar`, `set_always_on_top`, and `set_visible_on_all_workspaces` are all Tauri APIs.

**The support level is part of the data model, not a UI detail.** `stealth.rs` reports
`documented` for Windows and `measured` for macOS, because one is a vendor contract and the
other is an observation a point release could undo. The status a command returns describes what
actually took effect, never what was requested, so a caller cannot accidentally present a
capability the OS did not deliver — and a requested-but-inactive exclusion produces an explicit
warning rather than silence. See the matrix in the
[README](../README.md#what-quiet-actually-means).

**Pixels are not presence.** `window_enumerable` is hardcoded `true` and always surfaced. No
public API on either platform hides a window's *existence*, owning process, or geometry.

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

A third one is now answered. ~~macOS 14.4+ offers a CoreAudio process-tap API that captures
per-process audio without the alarming screen-recording permission prompt. ScreenCaptureKit is
the default path; the process tap is worth evaluating as a cleaner replacement.~~
**Measured on macOS 26.5** — see the [audio harness](../tools/audio-capture-harness). The tap
is the default path, not a replacement to evaluate: `AudioHardwareCreateProcessTap` plus a
private aggregate device delivers 48 kHz stereo float at real time (375 callbacks, 4.000 s
captured in 4.0 s, worst gap 10.8 ms) and **never involves Screen Recording permission at
all**. ScreenCaptureKit is the fallback for macOS before 14.2.

It carries its own trap, which is worse than a prompt. **Without audio-capture consent a tap
does not fail — it succeeds and returns silence**: 281 callbacks at real-time pacing, peak
amplitude 0.0000, with audio definitely playing. There is no error to catch, Apple ships no
public API to check the grant, and the prompt only exists for a bundle carrying
`NSAudioCaptureUsageDescription`. So Skia's Info.plist needs that key, and the audio engine has
to treat all-zero input as a consent state and report it the way `stealth.rs` reports capture
exclusion — what actually happened, never what was requested.

The trap turned out to be wider than taps, and it bit the shipped microphone path: the first
real build recorded five seconds of exact zeros, prompt-free, with the usage key correctly in
place. **No consent dialog is ever triggered implicitly.** Apple's AVFoundation documentation
confines the auto-prompt to `AVCaptureDeviceInput` creation; cpal reaches the microphone
through the CoreAudio HAL, which *"will vend silent audio samples"* until access is granted.
Consent is therefore requested explicitly before any stream opens — the microphone through the
public `AVCaptureDevice.requestAccess` (see `audio/consent.rs`, the crate's only `unsafe`
code), and the far end, when it lands, through the only route that exists for
`kTCCServiceAudioCapture`, which is not public.
