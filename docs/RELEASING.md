# Releasing

Skia ships through GitHub Releases only. There is no package registry involved — nobody
`install`s Skia as a dependency, they download an installer.

## Versioning

[Semantic Versioning](https://semver.org). While the version starts with `0.`, the app is
unstable by definition and anything may change between releases. `1.0.0` is the point at which
the behaviour and data format become something users can rely on.

The version lives in **`package.json`**, and `src-tauri/tauri.conf.json` reads it from there, so
there is one number to bump. `src-tauri/Cargo.toml` carries its own version for the Rust crate;
keep it in step to avoid confusion even though nothing publishes it.

## Cutting a release

```bash
# 1. Bump the version in package.json (and src-tauri/Cargo.toml to match)
# 2. Move CHANGELOG.md's Unreleased entries under a new heading with today's date
git commit -am "chore(release): v0.1.0"

# 3. Tag it. Annotated tags only — the workflow triggers on v*
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin main
git push origin v0.1.0
```

Pushing the tag runs [`.github/workflows/release.yml`](../.github/workflows/release.yml), which
builds a **universal macOS bundle** (one binary covering Apple Silicon and Intel, built on an
Apple Silicon runner) and a **Windows x64** installer, then attaches them to a **draft
prerelease**.

The draft is deliberate. Download both installers and confirm they actually launch before
publishing — an unsigned macOS app that was mangled in bundling fails in ways CI cannot detect.

## What being unsigned means

Skia is distributed without paid Apple or Windows code-signing certificates. That's a
deliberate trade to keep distribution free, and it has consequences worth stating plainly:

- **macOS.** The app is ad-hoc signed (`signingIdentity: "-"`), which is the minimum needed for
  an Apple Silicon build to launch at all. On first open, users must right-click the app and
  choose **Open** to get past Gatekeeper.
- **Windows.** SmartScreen shows a warning; users click **More info → Run anyway**. Where
  **Smart App Control** is enabled — the default on some clean Windows 11 installs — it can
  block an unsigned app outright with no override. There is no workaround short of buying a
  certificate; treat those users as unreachable for now.

If this materially hurts adoption, Azure Trusted Signing (roughly $120/year) is the cheapest
escape hatch on the Windows side.

## Auto-update (not yet wired up)

Tauri's updater signs releases with its own free [minisign](https://jedisct1.github.io/minisign/)
key, which is unrelated to OS code-signing — so auto-update works without paying for a
certificate. It is not enabled yet because `tauri-plugin-updater` isn't installed.

To turn it on:

1. Generate a keypair: `pnpm tauri signer generate -w ~/.tauri/skia.key`
2. Put the **private** key in the repository's Actions secrets as `TAURI_SIGNING_PRIVATE_KEY`
   (`gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/skia.key`). It must never be committed.
3. Put the **public** key in `src-tauri/tauri.conf.json` under `plugins.updater.pubkey`, along
   with the release endpoint. The public key is meant to ship inside the app.
4. Flip `uploadUpdaterJson` to `true` in the release workflow so `latest.json` is published.

Back up the private key somewhere safe. Losing it means existing installs can no longer verify
updates and users have to reinstall by hand.

One caveat specific to this setup: the in-app updater replaces the app in place, so macOS
Gatekeeper generally isn't re-triggered on update. But ad-hoc signatures are fragile, and a
failed in-place replacement surfaces as "app is damaged". Test the update path on current macOS
before relying on it — this is tracked as Phase 0 work in the [roadmap](ROADMAP.md).
