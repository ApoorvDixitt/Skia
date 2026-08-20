// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The audio section: see that the microphone is heard, before anything
 * depends on it.
 *
 * This exists because the measured failure mode of audio capture is silence
 * that looks like success — real-time callbacks, zero in every sample, no
 * error anywhere. So the section leads with a live meter (moving bar = the
 * pipeline hears you) and a recordable probe (listen to exactly what a
 * transcriber would receive). A silent probe is rendered as the finding it
 * is: check permission, check the device — never "done!".
 */

import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";

import {
  audioDevices,
  audioMeterStart,
  audioMeterStop,
  audioProbe,
  audioStatus,
  meterFraction,
  parseLevel,
  parseStatus,
  LEVEL_EVENT,
  STATUS_EVENT,
} from "../lib/audio";
import type { AudioDevice, AudioStatus, LevelUpdate } from "../lib/audio";
import { describeIpcError } from "../lib/stealth";
import { IconAudio } from "./icons";
import "./sections.css";

type DevicesState =
  | { kind: "loading" }
  | { kind: "loaded"; devices: AudioDevice[] }
  | { kind: "failed"; message: string };

type ProbeState =
  | { kind: "idle" }
  | { kind: "recording" }
  | {
      kind: "done";
      path: string;
      seconds: number;
      peak: number;
      silent: boolean;
    }
  | { kind: "failed"; message: string };

const PROBE_SECONDS = 5;

export function Audio() {
  const [devices, setDevices] = useState<DevicesState>({ kind: "loading" });
  const [status, setStatus] = useState<AudioStatus | null>(null);
  const [level, setLevel] = useState<LevelUpdate | null>(null);
  const [probe, setProbe] = useState<ProbeState>({ kind: "idle" });
  const [meterError, setMeterError] = useState<string | null>(null);

  // The meter is stopped on unmount so leaving the section releases the
  // microphone — an overlay app that holds the mic open when nobody asked is
  // exactly what Skia promises not to be. The ref mirrors `listening` for the
  // cleanup closure, and is written in an effect because writing a ref during
  // render is the one thing refs must never do.
  const listening = status?.state === "listening" || status?.state === "recording";
  const listeningRef = useRef(false);
  useEffect(() => {
    listeningRef.current = listening;
  }, [listening]);

  useEffect(() => {
    void audioDevices().then(
      (list) => {
        setDevices({ kind: "loaded", devices: list });
      },
      (error: unknown) => {
        setDevices({ kind: "failed", message: describeIpcError(error) });
      },
    );
    void audioStatus().then(setStatus, () => {
      // The engine reports its own death through the status event; a failed
      // initial read renders as "no status yet" rather than a broken section.
    });

    const unlistenLevel = listen(LEVEL_EVENT, (event) => {
      try {
        setLevel(parseLevel(event.payload, LEVEL_EVENT));
      } catch (error: unknown) {
        setMeterError(describeIpcError(error));
      }
    });
    const unlistenStatus = listen(STATUS_EVENT, (event) => {
      try {
        setStatus(parseStatus(event.payload, STATUS_EVENT));
      } catch (error: unknown) {
        setMeterError(describeIpcError(error));
      }
    });

    return () => {
      void unlistenLevel.then((stop) => {
        stop();
      });
      void unlistenStatus.then((stop) => {
        stop();
      });
      if (listeningRef.current) {
        void audioMeterStop().catch(() => {
          // Closing anyway; the engine also stops when the app does.
        });
      }
    };
  }, []);

  const toggleMeter = (): void => {
    setMeterError(null);
    const call = listening ? audioMeterStop : audioMeterStart;
    void call().then(setStatus, (error: unknown) => {
      setMeterError(describeIpcError(error));
    });
  };

  const runProbe = (): void => {
    setProbe({ kind: "recording" });
    void audioProbe(PROBE_SECONDS).then(
      (outcome) => {
        setProbe({
          kind: "done",
          path: outcome.path,
          seconds: outcome.seconds,
          peak: outcome.peak,
          silent: outcome.silent,
        });
        void audioStatus().then(setStatus, () => undefined);
      },
      (error: unknown) => {
        setProbe({ kind: "failed", message: describeIpcError(error) });
      },
    );
  };

  const fraction = level === null ? 0 : meterFraction(level.rms);
  const peakFraction = level === null ? 0 : meterFraction(level.peak);

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Audio</h2>
          <p className="db-subtitle">
            The microphone half of live capture. A moving bar means the
            pipeline hears you; a five-second sample is exactly what a
            transcriber would receive.
          </p>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {/* ------------------------------------------------ level meter -- */}
          <section className="au-block">
            <div className="db-row">
              <span className="db-row-icon">
                <IconAudio />
              </span>
              <div className="db-row-copy">
                <h3 className="db-row-title">Level meter</h3>
                <p className="db-row-sub">
                  {listening
                    ? statusLine(status)
                    : "Nothing is captured until you start it, and nothing is stored either way."}
                </p>
              </div>
              <div className="db-row-control">
                <button
                  type="button"
                  className="db-button"
                  disabled={status?.state === "recording"}
                  onClick={toggleMeter}
                >
                  {listening ? "Stop listening" : "Start listening"}
                </button>
              </div>
            </div>

            <div
              className="au-meter"
              role="meter"
              aria-label="Microphone level"
              aria-valuemin={0}
              aria-valuemax={1}
              aria-valuenow={Math.round(fraction * 100) / 100}
              data-active={listening}
              data-clipped={level?.clipped === true}
            >
              <span
                className="au-meter-fill"
                style={{ transform: `scaleX(${String(fraction)})` }}
              />
              <span
                className="au-meter-peak"
                style={{ left: `${String(peakFraction * 100)}%` }}
              />
            </div>

            {listening && level !== null && level.peak < 0.00001 ? (
              <p className="au-warning" role="status">
                The stream is delivering audio, but every sample is zero. On
                macOS that is what missing microphone permission looks like —
                it does not error, it hands over silence. Check System
                Settings → Privacy &amp; Security → Microphone.
              </p>
            ) : null}
            {status?.lastError != null ? (
              <p className="db-fail-error">
                <code>{status.lastError}</code>
                {status.rebuilds > 0
                  ? ` — the stream has been rebuilt ${String(status.rebuilds)} time(s).`
                  : null}
              </p>
            ) : null}
            {meterError !== null ? (
              <p className="db-fail-error">
                <code>{meterError}</code>
              </p>
            ) : null}
          </section>

          {/* ------------------------------------------------ sample probe -- */}
          <section className="au-block">
            <div className="db-row">
              <span className="db-row-icon">
                <IconAudio />
              </span>
              <div className="db-row-copy">
                <h3 className="db-row-title">Record a sample</h3>
                <p className="db-row-sub">
                  Five seconds, saved as 16 kHz mono WAV — the exact shape
                  transcription will consume. Listen to it; a meter can look
                  right while the audio is wrong.
                </p>
              </div>
              <div className="db-row-control">
                <button
                  type="button"
                  className="db-button"
                  disabled={probe.kind === "recording"}
                  data-busy={probe.kind === "recording"}
                  aria-busy={probe.kind === "recording"}
                  onClick={runProbe}
                >
                  {probe.kind === "recording"
                    ? "Recording…"
                    : `Record ${String(PROBE_SECONDS)} seconds`}
                </button>
              </div>
            </div>

            {probe.kind === "done" ? (
              <div
                className="au-probe-result"
                data-tone={probe.silent ? "alarm" : "ok"}
                role="status"
              >
                {probe.silent ? (
                  <>
                    <p className="au-probe-headline">
                      Recorded {probe.seconds.toFixed(1)} s — and every sample
                      is zero.
                    </p>
                    <p className="au-probe-detail">
                      That is not a quiet room; a real microphone in a real
                      room never records exact zeros. It is what a denied or
                      never-asked permission produces. Check System Settings →
                      Privacy &amp; Security → Microphone, make sure the right
                      input device is the default, then record again.
                    </p>
                  </>
                ) : (
                  <>
                    <p className="au-probe-headline">
                      Recorded {probe.seconds.toFixed(1)} s, peak{" "}
                      {Math.round(probe.peak * 100)}% of full scale.
                    </p>
                    <p className="au-probe-detail">
                      Saved to{" "}
                      <code className="measured" data-selectable="">
                        {probe.path}
                      </code>{" "}
                      — open it and listen. What you hear is what a
                      transcriber would get.
                    </p>
                  </>
                )}
              </div>
            ) : null}
            {probe.kind === "failed" ? (
              <p className="db-fail-error">
                <code>{probe.message}</code>
              </p>
            ) : null}
          </section>

          {/* ------------------------------------------------ device list -- */}
          <section className="au-block">
            <h3 className="db-row-title">Input devices</h3>
            {devices.kind === "loading" ? (
              <p className="db-row-sub">Reading the device list…</p>
            ) : null}
            {devices.kind === "failed" ? (
              <p className="db-fail-error">
                <code>{devices.message}</code>
              </p>
            ) : null}
            {devices.kind === "loaded" ? (
              devices.devices.length === 0 ? (
                <p className="db-row-sub">
                  No input devices. Plug in or enable a microphone, then
                  reopen this section.
                </p>
              ) : (
                <ul className="au-devices">
                  {devices.devices.map((device) => (
                    <li key={device.name} className="au-device">
                      <span className="au-device-name">{device.name}</span>
                      <span className="au-device-meta">
                        {device.sampleRateHz.toLocaleString()} Hz ·{" "}
                        {device.channels === 1
                          ? "mono"
                          : `${String(device.channels)} ch`}
                        {device.isDefault ? " · default" : ""}
                      </span>
                    </li>
                  ))}
                </ul>
              )
            ) : null}
            <p className="legend au-foot">
              Captures follow the system default input. Swap devices while the
              meter runs — the stream rebuilds itself, and says so above.
            </p>
          </section>
        </div>
      </div>
    </>
  );
}

/** The status line under the meter while it runs: device, rate, honesty. */
function statusLine(status: AudioStatus | null): string {
  if (status === null) return "Listening.";
  const parts: string[] = [];
  if (status.device !== null) parts.push(status.device);
  if (status.nativeRateHz !== null) {
    parts.push(`${status.nativeRateHz.toLocaleString()} Hz in, 16,000 Hz out`);
  }
  if (status.state === "recording") parts.push("recording");
  return parts.length > 0 ? parts.join(" · ") : "Listening.";
}
