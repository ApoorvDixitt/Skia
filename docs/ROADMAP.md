# Roadmap

An honest picture of what exists and what doesn't. Nothing below is a delivery commitment —
this is a solo project.

**Where things stand today:** a Tauri v2 shell that compiles on macOS and Windows and opens a
window. That's it. Everything in Phase 1 onward is unbuilt.

Priorities are **P0** (needed for a first usable release), **P1** (soon after), and **P2** (later).

## Phase 0 — de-risk

Two unknowns could change the design or the way features are described, so they get answered
before anything is built on top of them.

- [x] Screen-capture test harness on current macOS — **done**, see
      [`tools/macos-capture-harness`](../tools/macos-capture-harness). Measured on macOS 26.5:
      `.none` excludes overlay pixels from ScreenCaptureKit, legacy CoreGraphics, and
      full-screen shares in Meet and Zoom. Undocumented and unguaranteed, so re-run per
      macOS release. Pixel exclusion does **not** hide the window from enumeration.
- [~] Far-end audio capture proof of concept — **partly done**, see
      [`tools/audio-capture-harness`](../tools/audio-capture-harness). Measured on macOS 26.5:
      a CoreAudio process tap delivers 48 kHz stereo float at real time and needs **no Screen
      Recording permission**, which settles the capture path. **Still open:** every sample was
      zero, because audio-capture consent is not granted — a tap without it succeeds and
      returns silence rather than failing. Finishing the measurement needs the bundled probe
      run with the grant accepted, which is a human with a dialog, not a test.
- [~] Audio device hot-swap proof of concept — harness built (`hotswap-probe`), which watches
      default-device and device-list notifications against a live mic stream and reports how
      many arrive per physical action. **Not yet run across wired, Bluetooth, and external
      devices**, and Bluetooth is the case that matters: an A2DP↔HFP switch presents as several
      notifications in a burst, so the debounce window has to come from observed timings.
- [ ] Unsigned in-place update test — confirm an ad-hoc-signed app survives being replaced by
      its own updater without tripping "app is damaged".

**Done when:** the real macOS capture behaviour is written down, audio survives a device swap,
and an update replaces the app cleanly.

## Phase 1 — minimum viable app

- [~] Overlay window that never steals focus and has no dock, taskbar, or alt-tab presence (P0)
      — always-on-top, visible on all workspaces, no taskbar entry on Windows, reliably visible
      (5/5 cold launches). **Two parts are not done and are reported as such in-app:** on macOS
      the dock icon is still shown, and the overlay takes focus once when it opens.
      Both need a non-activating `NSPanel` — see below. Menu-bar/tray item not added yet.
- [ ] Non-activating overlay panel via `tauri-nspanel` (P0) — the blocker for both remaining
      Tier-B properties. An accessory activation policy is what hides the dock icon, but it was
      measured to leave the window invisible in every ordering (0/5 launches on screen either at
      startup or after showing; 5/5 without it). An `NSPanel` with `.nonactivatingPanel` is the
      documented way to have a visible overlay that neither activates nor appears in the dock.
- [x] Capture exclusion where the OS supports it, with in-app status that states what is
      actually active (P0) — verified end to end: the running app's overlay is absent from a
      ScreenCaptureKit capture while remaining enumerable, and the UI renders `documented`
      (Windows) distinctly from `measured` (macOS) so the weaker case can never read as a promise.
- [~] Silent, remappable global hotkeys (P0) — silent global hotkey works
      (⌘⇧Space / Ctrl⇧Space toggles the overlay). **Remapping is not implemented.**
- [ ] Ask mode: hotkey, region capture, OCR, streamed Markdown answer (P0) — UI shell only, inert
- [ ] One model provider working end to end, with the key in the OS keychain (P0)
- [~] Local SQLite history, searchable, exportable, deletable (P0) — storage layer done
      (WAL, versioned migrations, FTS5 search, export, purge, 14 tests) and the capture-exclusion
      preference persists through it. **No history UI yet**, and export/purge are backend commands
      with nothing calling them.
- [ ] First-run permission flow with plain-language explanations and a re-check button (P0)
- [ ] GitHub Release with working auto-update (P0) — release pipeline proven, updater not wired

**Done when:** a fresh install reaches a working state through a guided flow, Ask mode answers
end to end, and the app can update itself.

## Phase 2 — live meetings

- [~] Dual-stream capture: microphone and far-end audio, kept separate (P0) — **microphone
      half done**: capture on a supervised engine thread, downmix + resample to 16 kHz mono,
      debounced device hot-swap, a live meter and a recordable WAV probe in the dashboard's
      Audio section, and silence reported as the consent state the harness measured it to be.
      **Far end not built**: needs the CoreAudio process-tap path (macOS) and WASAPI loopback
      (Windows) from the audio harness findings.
- [ ] Streaming transcription with speaker labels, cloud and local backends (P0)
- [ ] End-of-utterance detection, acoustic and lexical (P0)
- [ ] Speculative retrieval and generation, cancelled on barge-in (P0)
- [ ] Live answers under one second to first token on the fast path (P0)
- [ ] Question and objection detection surfacing answer cards (P0)
- [ ] Post-call pack: summary and action items (P0)
- [ ] Follow-up email draft (P1)
- [ ] Missed-opportunity highlights (P1)
- [ ] Custom note templates (P1)
- [ ] Visible listening indicator and consent reminder (P0)

**Done when:** a real meeting produces a usable transcript with speaker labels, sub-second live
answers, and a post-call summary with action items.

## Phase 3 — knowledge base

- [~] Ingest PDF, DOCX, TXT, and Markdown with structure-aware chunking (P0) — **TXT and
      Markdown done**, with heading-aware sections and exact byte offsets. PDF and DOCX return an
      explicit `Unsupported` error rather than failing quietly; both need heavy parsers.
- [~] Local embeddings with incremental re-indexing on file change (P0) — **incremental
      re-indexing done** via SHA-256 per document (unchanged file is a no-op, changed file
      replaces only its own chunks). **No embeddings**: `bge-m3` is a multi-gigabyte download and
      is not wired up.
- [~] Hybrid retrieval: BM25 and vector search fused, then reranked (P0) — **BM25 arm done**
      via FTS5. The vector arm and reranker are absent, so retrieval matches words rather than
      meaning: "money back" will not find a document that only says "refund". The fusion point
      is marked in `retrieve()` with the reciprocal-rank-fusion formula.
- [x] Citations resolving to the exact source passage (P0) — byte offsets are stored per chunk
      and verified by slicing the original document, including multi-byte UTF-8. A mismatch is an
      error, never a wrong quotation. Not yet clickable in the UI.
- [ ] Always-on context field (P0)
- [x] Needs-retrieval gate so small talk skips lookup (P0) — heuristic, no ML.
- [x] Model catalog, OpenRouter, custom providers, and local models via Ollama (P0) — nine
      providers (Ollama, LM Studio, Groq, Cerebras, OpenAI, Anthropic, Gemini, OpenRouter, plus an
      offline mock), all through one OpenAI-compatible streaming client. Keys live in the OS
      keychain. **Verified against a local socket, not against real vendor APIs** — that needs
      your own keys.
- [x] Editable system prompts per mode, with profiles and reset-to-default (P0) — three shipped
      defaults, five profiles, tone and length presets, strict single-pass interpolation that
      rejects unknown variables and cannot be used to inject template syntax. No settings UI yet.
- [ ] Bounded agentic tools with a visible trace, off by default in live mode (P1)

**Done when:** an answer cites a document and the citation resolves to the right passage, and
the app runs usefully on entirely local models.

## Phase 4 — polish

- [ ] Developer panel: provider ping, smoke tests, retrieval eval, latency view, mock provider (P1)
- [ ] Update card showing version and release notes (P0)
- [ ] Calendar integration to arm listening for upcoming meetings (P1)
- [ ] Overlay theming, opacity, and sizing (P1)
- [ ] Optional encryption at rest (P1)
- [ ] Custom live action buttons bound to prompts (P1)
- [ ] CRM and ATS export (P2)
- [ ] Mobile companion (P2)

## Explicitly not planned

- A hosted backend, user accounts, or billing — Skia is local-first and bring-your-own-key
- Server-side recording or a bot that joins calls
- Telemetry or analytics of any kind
- A general-purpose chatbot

## Open questions

- Should the default transcription backend be local Whisper (free, private) or a cloud service (fastest)?
- Follow-up email: deep link to the default mail client, clipboard, or an optional API integration?
- Calendar: read the local OS calendar, or OAuth into Google and Outlook?
- Ship the CoreAudio process-tap capture path, or stay on ScreenCaptureKit until it proves out?
- What should the project actually be called? `Skia` collides with Google's graphics library.
