# macOS capture-exclusion harness

Measures whether `NSWindow.sharingType = .none` actually keeps a window out of screen
captures on the macOS version you're running.

This exists because the answer is **undocumented and version-dependent**, so it has to be
re-measured rather than assumed. Apple's shipping SDK header still says `.none` means "the
content cannot be captured", while [Apple's own documentation](https://developer.apple.com/documentation/appkit/nswindow/sharingtype-swift.enum/none)
now calls it *"A legacy constant that macOS no longer uses"* and says *"Don't use this value to
hide or omit content from being captured."* Those cannot both be true. Run the harness.

Re-run it on every macOS release before trusting the matrix in the root README.

## The files

| File | What it does |
|---|---|
| `tcc-preflight.swift` | Prints whether Screen Recording permission is granted. Uses `CGPreflightScreenCaptureAccess()`, which checks **without** triggering a prompt. Run this first. |
| `capture-probe.swift` | Creates a `.none` window and a `.readOnly` window, then captures the screen three ways: ScreenCaptureKit, legacy `CGWindowListCreateImage` (via `dlsym`), and the window-metadata list. Writes `sck.png` and `cglegacy.png`. |
| `hold-short.swift` | Displays the two windows for 25 s and captures nothing. |
| `hold-long.swift` | Same, for 15 minutes — long enough to start a real screen share. |

## Running it

```bash
swiftc -O tcc-preflight.swift -o tcc-preflight && ./tcc-preflight
swiftc -O capture-probe.swift -o capture-probe && ./capture-probe
swiftc -O hold-long.swift     -o hold-long     && ./hold-long
```

### Grant permission first, or the result is meaningless

**Screen Recording permission is granted to the *responsible process*, not to the binary.**
These are bare CLI executables with no bundle identity, so macOS attributes them to whatever
launched them — your terminal. Grant it to **your terminal app**, then fully quit and reopen
it, because the permission state is cached for a process's lifetime.

```bash
open "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
```

`tcc-preflight` must print `true` before you believe anything else.

### Two traps that will mislead you

**Without permission, ScreenCaptureKit hard-fails while legacy CoreGraphics degrades quietly.**
SCK throws, so `sck.png` is never written; legacy capture still returns an image containing the
wallpaper and *the calling process's own windows*. It is very easy to look at that image, see
the `.none` window correctly excluded, and conclude SCK honoured it — when SCK never ran. If
`sck.png` is missing, you have no ScreenCaptureKit result.

**Without permission, the window metadata lies.** `kCGWindowSharingState` reports `0` for
almost every window, and `0` means `.none`. Since the documented default is `.readOnly` (`1`),
those zeros are TCC redaction, not real settings.

## Testing against a real screen share

Self-capture is only a proxy. The threat model is a *different* process holding a long-lived
`SCStream`, which is the configuration Apple's own open bug describes failing.

Run `hold-long`, then start a meeting and **share your entire screen** — not a single window.
Window-scoped sharing excludes everything else by construction, so both test windows vanish
and it proves nothing. Verify from a second device joined to the same meeting.

Then **stop and restart sharing, and switch between displays, windows, and tabs.**
[FB21115847](https://developer.apple.com/forums/thread/808016) reports `.none` windows being
excluded at first and then appearing once the content filter is rebuilt, so the toggling is
where it is expected to break.

Do **not** use ⌘⇧5 or `/usr/sbin/screencapture` as your ScreenCaptureKit oracle.
`screencaptureui` links SkyLight and CoreGraphics rather than SCK, and `screencapture` imports
both paths, so which one runs is ambiguous.

## Results so far

| Date | macOS | Result |
|---|---|---|
| 2026-08-19 | 26.5 (25F71) | `.none` **excluded** in ScreenCaptureKit (`SCScreenshotManager`) and in legacy `CGWindowListCreateImage`. Also excluded from full-screen shares in **Google Meet and Zoom**, confirmed from a second device, and held across switching displays/windows/tabs. `.readOnly` captured normally throughout. |

**But exclusion applies to pixels only.** In the same run, `SCShareableContent` still
enumerated the `.none` window, and `CGWindowListCopyWindowInfo` still reported its owner PID,
bounds, and `sharingState = 0`. The window's *existence* is fully visible to anything that
asks — and since almost nothing legitimately sets `.none`, that value is an unusual signal
rather than camouflage.
