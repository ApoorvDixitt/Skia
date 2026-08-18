# Security Policy

## Supported versions

Skia is pre-alpha and has no released versions yet. Once releases begin, only the latest
release will receive security fixes.

| Version | Supported |
|---|---|
| `main` | Yes |
| Released versions | Latest only |

## Reporting a vulnerability

**Please do not open a public issue for a security problem.**

Report it privately through GitHub: go to the
[Security tab](https://github.com/ApoorvDixitt/Skia/security/advisories/new) and use
"Report a vulnerability". That opens a private advisory visible only to the maintainer.

This is a solo-maintained project, so please allow up to 7 days for an initial response.
If you don't hear back in that window, feel free to nudge [@ApoorvDixitt](https://github.com/ApoorvDixitt)
on GitHub.

## What's especially worth reporting

Skia handles material that users expect to stay on their machine, so the areas below matter
most:

- **API key handling** — keys are meant to live in the OS keychain and never touch plaintext
  config, logs, or crash reports.
- **Local data at rest** — transcripts, meeting notes, and the indexed knowledge base.
- **Unintended network traffic** — anything leaving the device that the user didn't ask for.
  Skia is meant to talk only to the model provider you configured and to GitHub for updates.
- **Update integrity** — the updater verifies signatures against a public key baked into the
  app; a bypass of that check is a high-severity issue.
- **Capture and overlay behaviour** — if the overlay leaks in a way the documented
  [platform matrix](README.md#what-quiet-actually-means) says it shouldn't, that's a bug worth
  reporting privately first.

## Scope

Two things are out of scope, because they're documented limitations rather than defects:

- The overlay appearing in screen captures on **macOS 15+**. This is a platform limitation
  Skia states openly; see the matrix in the README.
- Installer and launch warnings caused by Skia being **unsigned**. See
  [docs/RELEASING.md](docs/RELEASING.md).
