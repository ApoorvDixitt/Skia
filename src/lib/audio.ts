// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import {
  asBoolean,
  asNullableNumber,
  asNullableString,
  asNumber,
  asRecord,
  asString,
  mapArray,
} from "./ipc";

/**
 * The audio contract, mirrored from Rust — same rule as every other IPC
 * surface: everything arrives as `unknown` and is checked, never cast.
 *
 * The one payload with a meaning worth restating here is `silent` on a probe:
 * the backend measured that capture without consent on macOS *succeeds and
 * returns zeros*, so a silent recording is a permission-or-device finding the
 * UI must present as a problem, never as a quiet room.
 */

export interface AudioDevice {
  name: string;
  isDefault: boolean;
  sampleRateHz: number;
  channels: number;
}

export type EngineState = "idle" | "listening" | "recording";

export interface AudioStatus {
  state: EngineState;
  device: string | null;
  nativeRateHz: number | null;
  nativeChannels: number | null;
  rebuilds: number;
  lastError: string | null;
}

export interface LevelUpdate {
  /** Linear 0..1 — convert for display, keep raw for reasoning. */
  rms: number;
  peak: number;
  clipped: boolean;
}

export interface ProbeOutcome {
  path: string;
  seconds: number;
  sampleRateHz: number;
  peak: number;
  silent: boolean;
}

/** Event names the engine emits; `Audio.tsx` subscribes to both. */
export const LEVEL_EVENT = "audio:level";
export const STATUS_EVENT = "audio:status";

function parseDevice(value: unknown, at: string): AudioDevice {
  const source = asRecord(value, at);
  return {
    name: asString(source, at, "name"),
    isDefault: asBoolean(source, at, "isDefault"),
    sampleRateHz: asNumber(source, at, "sampleRateHz"),
    channels: asNumber(source, at, "channels"),
  };
}

export function parseStatus(value: unknown, at: string): AudioStatus {
  const source = asRecord(value, at);
  const state = asString(source, at, "state");
  if (state !== "idle" && state !== "listening" && state !== "recording") {
    throw new Error(`${at}.state is ${JSON.stringify(state)}, which is not a state.`);
  }
  return {
    state,
    device: asNullableString(source, at, "device"),
    nativeRateHz: asNullableNumber(source, at, "nativeRateHz"),
    nativeChannels: asNullableNumber(source, at, "nativeChannels"),
    rebuilds: asNumber(source, at, "rebuilds"),
    lastError: asNullableString(source, at, "lastError"),
  };
}

export function parseLevel(value: unknown, at: string): LevelUpdate {
  const source = asRecord(value, at);
  return {
    rms: asNumber(source, at, "rms"),
    peak: asNumber(source, at, "peak"),
    clipped: asBoolean(source, at, "clipped"),
  };
}

function parseProbe(value: unknown, at: string): ProbeOutcome {
  const source = asRecord(value, at);
  return {
    path: asString(source, at, "path"),
    seconds: asNumber(source, at, "seconds"),
    sampleRateHz: asNumber(source, at, "sampleRateHz"),
    peak: asNumber(source, at, "peak"),
    silent: asBoolean(source, at, "silent"),
  };
}

export async function audioDevices(): Promise<AudioDevice[]> {
  const raw: unknown = await invoke("audio_devices");
  return mapArray(raw, "audio_devices", parseDevice);
}

export async function audioStatus(): Promise<AudioStatus> {
  const raw: unknown = await invoke("audio_status");
  return parseStatus(raw, "audio_status");
}

export async function audioMeterStart(): Promise<AudioStatus> {
  const raw: unknown = await invoke("audio_meter_start");
  return parseStatus(raw, "audio_meter_start");
}

export async function audioMeterStop(): Promise<AudioStatus> {
  const raw: unknown = await invoke("audio_meter_stop");
  return parseStatus(raw, "audio_meter_stop");
}

export async function audioProbe(seconds: number): Promise<ProbeOutcome> {
  const raw: unknown = await invoke("audio_probe", { seconds });
  return parseProbe(raw, "audio_probe");
}

/**
 * A meter position for a linear level, as a fraction of the bar.
 *
 * Perception is logarithmic: linear 0.05 RMS is normal speech, and a linear
 * bar would leave it hugging the left edge. Mapped over -60 dB..0 dB, which is
 * the useful range of a microphone that is working.
 */
export function meterFraction(linear: number): number {
  if (linear <= 0) return 0;
  const db = 20 * Math.log10(linear);
  return Math.min(1, Math.max(0, (db + 60) / 60));
}
