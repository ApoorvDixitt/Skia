// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// Prompt editing IPC.
///
/// The backend rejects a template referring to variables it cannot fill, so a
/// broken prompt fails when it is saved rather than silently mid-call. Surface
/// that error verbatim — it names the offending placeholder and its position.

import { invoke } from "@tauri-apps/api/core";

import { asRecord, describeValue } from "./ipc";

/** Which surface is asking, and therefore which default prompt applies. */
export type Mode = "ask" | "live" | "listen";

/** What the user is using Skia for right now. */
export type Profile = "general" | "interview" | "meeting" | "sales" | "study";

export const MODES: readonly Mode[] = ["ask", "live", "listen"];
export const PROFILES: readonly Profile[] = [
  "general",
  "interview",
  "meeting",
  "sales",
  "study",
];

export const MODE_LABELS: Record<Mode, string> = {
  ask: "Ask",
  live: "Live",
  listen: "Listen",
};

export const PROFILE_LABELS: Record<Profile, string> = {
  general: "General",
  interview: "Interview",
  meeting: "Meeting",
  sales: "Sales",
  study: "Study",
};

/** The four placeholders a template may reference. Anything else is rejected. */
export const VARIABLES: readonly string[] = [
  "{kb_context}",
  "{transcript}",
  "{question}",
  "{profile}",
];

/** A stable key for one (mode, profile) cell. */
export function pairKey(mode: Mode, profile: Profile): string {
  return `${mode}:${profile}`;
}

export async function fetchTemplate(
  mode: Mode,
  profile: Profile,
): Promise<string> {
  const raw = await invoke<unknown>("prompts_template", { mode, profile });
  if (typeof raw !== "string" || raw.length === 0) {
    throw new Error(
      `prompts_template should return a non-empty string, got ${describeValue(raw)}.`,
    );
  }
  return raw;
}

export async function saveTemplate(
  mode: Mode,
  profile: Profile,
  template: string,
): Promise<void> {
  await invoke<unknown>("prompts_set_override", { mode, profile, template });
}

export async function resetTemplate(
  mode: Mode,
  profile: Profile,
): Promise<void> {
  await invoke<unknown>("prompts_reset", { mode, profile });
}

/**
 * Which (mode, profile) cells currently hold a user override rather than the
 * shipped default — this drives whether Reset does anything.
 *
 * The backend serialises overrides nested as `{mode: {profile: template}}`, so
 * both levels are walked and flattened to `pairKey` values. Returning only mode
 * names would make Reset look available for every profile under an edited mode.
 */
export async function fetchOverriddenPairs(): Promise<Set<string>> {
  const raw = await invoke<unknown>("prompts_get");
  const bundle = asRecord(raw, "prompts_get");
  const overrides = bundle.overrides;
  if (overrides === undefined || overrides === null) return new Set();

  const byMode = asRecord(overrides, "prompts_get.overrides");
  const pairs = new Set<string>();
  for (const [mode, profiles] of Object.entries(byMode)) {
    const at = `prompts_get.overrides.${mode}`;
    for (const profile of Object.keys(asRecord(profiles, at))) {
      pairs.add(`${mode}:${profile}`);
    }
  }
  return pairs;
}
