# Contributing

Thanks for your interest in Skia. The project is pre-alpha and the architecture is still
moving, so the most useful thing you can do before writing code is **open an issue** and
check that the direction is right — it saves you from building against a design that's about
to change.

## Development setup

Fork the repository on GitHub first (the **Fork** button, top right), then:

```bash
git clone https://github.com/YOUR-USERNAME/Skia.git
cd Skia
git remote add upstream https://github.com/ApoorvDixitt/Skia.git
pnpm install
pnpm tauri dev
```

Adding `upstream` now means you can pull in changes later without re-cloning.

You'll need:

- [Rust](https://rustup.rs) (stable) — `rustup` is the easiest way in
- [Node.js](https://nodejs.org) 20+ and [pnpm](https://pnpm.io) 9
- **macOS:** Xcode Command Line Tools — `xcode-select --install`
- **Windows:** Microsoft C++ Build Tools, plus the WebView2 runtime (already present on Windows 11)

Before opening a pull request, run what CI runs:

```bash
pnpm lint --max-warnings 0
pnpm typecheck
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Warnings are treated as errors on both sides — `--max-warnings 0` for TypeScript,
`-D warnings` for Rust — so CI fails on anything ESLint or Clippy flags.

## Where code goes

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the module layout. The short version:
platform-specific behaviour belongs in Rust behind `#[cfg(target_os = "...")]`, and the
frontend should stay platform-agnostic.

Skia targets both macOS and Windows from the same commit. If you add platform-specific code,
say in the PR which platforms you actually tested on — CI type-checks both, but it can't tell
you whether an overlay behaves correctly.

## Workflow

Nobody pushes to `main` — it's protected, and all changes arrive as pull requests from a
branch or a fork.

```bash
# 1. Start from an up-to-date main
git checkout main
git pull upstream main

# 2. Branch. Use feat/ for features, fix/ for bug fixes
git checkout -b feat/short-description

# 3. Commit as you go, using Conventional Commits
git commit -m "feat(rag): add reciprocal rank fusion"

# 4. Run what CI runs (see above), then push to your fork
git push -u origin feat/short-description
```

Then open a pull request against `ApoorvDixitt/Skia`'s `main` branch. GitHub will offer a
"Compare & pull request" button after you push. Fill in the template and link the issue it
addresses.

A few conventions worth knowing:

- **Commit messages** follow [Conventional Commits](https://www.conventionalcommits.org):
  `feat(rag): add reciprocal rank fusion`, `fix: rebuild audio stream on device change`,
  `docs: clarify the macOS capture matrix`. Keep each commit to one logical change, and use the
  body to explain *why* rather than what.
- **CI must pass** on both macOS and Windows before a PR can merge. It runs ESLint, TypeScript,
  the frontend build, `cargo fmt`, and Clippy with warnings treated as errors.
- **PRs are squash-merged**, so your branch becomes a single commit on `main` and history stays
  linear. Commit as messily as you like on your branch — it gets collapsed.
- **Draft PRs are welcome** if you want feedback before the work is finished.

If review takes a while, feel free to nudge — this is a solo-maintained project, not a
signal that the contribution isn't wanted.

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
