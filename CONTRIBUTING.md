# Contributing

Thanks for your interest in Skia. The project is pre-alpha and the architecture is still
moving, so the most useful thing you can do before writing code is **open an issue** and
check that the direction is right — it saves you from building against a design that's about
to change.

## Development setup

```bash
git clone https://github.com/ApoorvDixitt/Skia.git
cd Skia
pnpm install
pnpm tauri dev
```

You'll need:

- [Rust](https://rustup.rs) (stable) — `rustup` is the easiest way in
- [Node.js](https://nodejs.org) 20+ and [pnpm](https://pnpm.io) 9
- **macOS:** Xcode Command Line Tools — `xcode-select --install`
- **Windows:** Microsoft C++ Build Tools, plus the WebView2 runtime (already present on Windows 11)

Before opening a pull request, run what CI runs:

```bash
pnpm typecheck
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

## Where code goes

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the module layout. The short version:
platform-specific behaviour belongs in Rust behind `#[cfg(target_os = "...")]`, and the
frontend should stay platform-agnostic.

Skia targets both macOS and Windows from the same commit. If you add platform-specific code,
say in the PR which platforms you actually tested on — CI type-checks both, but it can't tell
you whether an overlay behaves correctly.

## Workflow

1. Branch off `main`: `feat/short-description` or `fix/short-description`.
2. Keep commits atomic and use [Conventional Commits](https://www.conventionalcommits.org):
   `feat(rag): add reciprocal rank fusion`, `fix: rebuild audio stream on device change`,
   `docs: clarify the macOS capture matrix`.
3. Make sure lint, typecheck, and build pass locally.
4. Open a pull request, fill in the template, and link the related issue.
5. Pull requests are squash-merged once checks pass, so `main` stays linear.

## Reporting bugs and requesting features

Use the [issue templates](https://github.com/ApoorvDixitt/Skia/issues/new/choose). For bugs,
your OS version matters more than usual here — a lot of Skia's behaviour is
platform-dependent, especially anything involving audio capture or the overlay.

Please don't file security problems as public issues; see [SECURITY.md](SECURITY.md).

## A note on scope

Skia deliberately has no cloud backend, no accounts, and no telemetry. Contributions that
introduce a hosted service, phone home, or collect usage data are out of scope regardless of
how they're implemented.

## Code of Conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md).
