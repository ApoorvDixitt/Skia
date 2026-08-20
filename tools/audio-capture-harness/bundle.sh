#!/bin/bash
# Copyright 2026 Apoorv Dixit
# SPDX-License-Identifier: Apache-2.0
#
# Wraps a probe in a minimal .app bundle so macOS can grant it audio-capture
# consent.
#
# This is not packaging convenience, it is the difference between a measurement
# and a guess. A loose executable cannot hold audio-capture permission: consent
# is attributed to a bundle identity, and the prompt's wording is read from
# `NSAudioCaptureUsageDescription` in an Info.plist. Without both, a process tap
# is created successfully and then hands back silence — measured on macOS 26.5 as
# 281 real-time callbacks with a peak amplitude of 0.0000 while audio was
# definitely playing.
#
# So every probe that captures audio has to be run from a bundle. Compiling and
# running the .swift file directly will "work" and tell you nothing.
#
# Usage:
#   ./bundle.sh loopback-probe.swift
#   ./bundle.sh dual-probe.swift
#
# Then run the wrapped binary directly, so its stdout still reaches your
# terminal while macOS attributes it to the bundle:
#
#   "build/Skia Audio Probe.app/Contents/MacOS/probe" 10 --exclude-self

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <probe.swift>" >&2
  exit 2
fi

source_file="$1"
if [[ ! -f "$source_file" ]]; then
  echo "$source_file does not exist" >&2
  exit 2
fi

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
app="$here/build/Skia Audio Probe.app"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"

# CFBundleIdentifier is what TCC keys the grant on. It is deliberately distinct
# from Skia's own identifier: a grant made to a diagnostic probe must not be
# mistaken for, or silently satisfy, a grant to the app.
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Skia Audio Probe</string>
    <key>CFBundleIdentifier</key>
    <string>dev.skia.harness.audio-probe</string>
    <key>CFBundleExecutable</key>
    <string>probe</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>14.2</string>
    <!-- Without this key there is no prompt and no possible grant, so a tap
         returns silence. It is the whole reason this bundle exists. -->
    <key>NSAudioCaptureUsageDescription</key>
    <string>Skia's audio harness is measuring whether far-end call audio can be captured on this version of macOS.</string>
    <key>NSMicrophoneUsageDescription</key>
    <string>Skia's audio harness is measuring how much far-end audio leaks into the microphone.</string>
    <!-- LSUIElement, deliberately NOT LSBackgroundOnly. Both hide the dock icon
         for what is really a CLI tool, but a background-only app cannot present
         UI at all -- including a TCC consent dialog. Since provoking exactly
         that dialog is the point of this bundle, background-only would defeat
         it, and the failure would look like "macOS never asks". -->
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

echo "compiling $source_file"
swiftc -O "$source_file" -o "$app/Contents/MacOS/probe"

# Ad-hoc signature. TCC will not attribute a grant to an unsigned bundle, and an
# ad-hoc identity is stable enough for a local harness. It also means the grant
# is invalidated whenever the binary is recompiled, which is a feature here:
# a stale grant would let a broken probe look like a working one.
echo "signing ad-hoc"
codesign --force --sign - --identifier dev.skia.harness.audio-probe "$app" >/dev/null 2>&1 \
  || echo "  codesign failed; TCC may refuse to remember a grant for this bundle"

cat <<EOF

built  $app

Run it directly so stdout still reaches this terminal:

  "$app/Contents/MacOS/probe" 10 --exclude-self

The first capture should prompt for audio-capture permission. If no prompt
appears and the audio is silent, check:

  open "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"

Recompiling invalidates the ad-hoc signature, so expect to grant it again after
every change. That is intentional — see the comment in this script.
EOF
