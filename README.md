# Skia

> A local-first meeting copilot. Live transcription, structured notes, and answers grounded in your own documents — running on your machine, with your own API key or a fully local model.

[![CI](https://github.com/ApoorvDixitt/Skia/actions/workflows/ci.yml/badge.svg)](https://github.com/ApoorvDixitt/Skia/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform: macOS · Windows](https://img.shields.io/badge/platform-macOS%20%C2%B7%20Windows-lightgrey.svg)](#building-from-source)

> [!IMPORTANT]
> **Status: pre-alpha. There is nothing to install yet.**
> What exists today is a Tauri v2 shell that compiles and opens a window. None of the
> features below are implemented. The [roadmap](docs/ROADMAP.md) is the honest picture of
> what is built and what isn't — please read it before opening a feature request.
>
> `Skia` is a provisional codename (Greek *σκιά*, "shadow") and is likely to change before
> the first release; it collides with [Google's Skia graphics library](https://skia.org),
> with which this project has no affiliation.

## What it will do

Skia is meant to help *during* a conversation rather than after it:

- **Live transcription** with speaker labels, from your microphone and the far end of the call.
- **Answers while you talk** — trigger by hotkey and get a streamed answer that cites your own material.
- **A first-class knowledge base** — drop in PDF, DOCX, TXT, or Markdown; answers link back to the exact passage they came from.
- **A post-call pack** — summary, action items, and a follow-up draft.
- **Your choice of model** — OpenAI, Anthropic, Gemini, Groq, or OpenRouter with your own key, or fully local via Ollama and Whisper. The local path costs nothing to run.
- **A quiet overlay** — no dock icon, no taskbar button, no bot joining your meeting, no notification sounds.

Everything lives on your device in SQLite. There is no Skia server, no account, and no telemetry.

## What "quiet" actually means

Skia keeps itself out of your way, but that means different things on different operating
systems, and we would rather state the limits plainly than imply a guarantee the OS does not
give us:

| Capability | Windows 10 2004+ / 11 | macOS ≤ 14 | macOS 15+ |
|---|---|---|---|
| Excluded from screen capture and screen sharing | Yes | Yes (legacy capture paths) | **No — not guaranteed** |
| No dock, taskbar, menu-bar, or alt-tab presence | Yes | Yes | Yes |
| No bot joins the call | Yes | Yes | Yes |
| Silent, remappable hotkeys | Yes | Yes | Yes |
| Overlay never steals focus | Yes | Yes | Yes |

On **macOS 15 and later**, modern screen-capture APIs ignore the window-exclusion flag, so
Skia's overlay *will* appear in a screen share or recording. The app will tell you this
directly rather than showing a switch that implies otherwise. Confirming the exact behaviour
on current macOS is [tracked work](docs/ROADMAP.md#phase-0--de-risk), not a settled question.

Separately: none of this is protection against kernel-level monitoring, corporate device
management, or a second camera. No user-space application can offer that, and Skia does not
claim to.

## Intended use

Skia is for keeping a better record of, and having better recall during, your own
conversations: sales and customer calls, user interviews and discovery, recruiting screens,
consulting sessions, and studying from your own notes.

Two things are yours to handle, not the app's:

- **Consent.** Recording and transcription laws differ by jurisdiction — many places require
  every participant to agree — and meeting platforms have their own policies on top of that.
  Skia shows a visible indicator whenever it is listening, but obtaining consent is your
  responsibility.
- **Where you use it.** Please don't use Skia anywhere you've agreed not to use assistance.
  That isn't what this project is for.

## Building from source

There are no releases yet, so building is the only way to run it.

**Prerequisites**

- [Rust](https://rustup.rs) (stable)
- [Node.js](https://nodejs.org) 20 or newer, and [pnpm](https://pnpm.io) 9
- **macOS:** Xcode Command Line Tools (`xcode-select --install`)
- **Windows:** [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and the WebView2 runtime (preinstalled on Windows 11)

```bash
git clone https://github.com/ApoorvDixitt/Skia.git
cd Skia
pnpm install
pnpm tauri dev      # run in development
pnpm tauri build    # produce a bundle in src-tauri/target/release/bundle
```

Distribution will be through GitHub Releases, unsigned — see [docs/RELEASING.md](docs/RELEASING.md)
for what that means for install warnings and updates.

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — how the app is put together and where code belongs
- [Roadmap](docs/ROADMAP.md) — phases, priorities, and the open unknowns
- [Releasing](docs/RELEASING.md) — versioning, tags, and the release pipeline

## Contributing

Contributions are welcome, though the architecture is still moving quickly — please open an
issue to discuss anything substantial before writing code. See
[CONTRIBUTING.md](CONTRIBUTING.md) and the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE) © 2026 Apoorv Dixit
