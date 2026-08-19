// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// First-run setup state, persisted in the settings table.
///
/// Skipping counts as done. Setup that reappears because the user declined it is
/// worse than no setup, and the dashboard can always re-run it.

import { invoke } from "@tauri-apps/api/core";

import { describeValue } from "./ipc";

/** Whether setup has been completed or deliberately skipped. */
export async function fetchOnboardingDone(): Promise<boolean> {
  const raw = await invoke<unknown>("onboarding_done");
  if (typeof raw !== "boolean") {
    throw new Error(
      `onboarding_done should return a boolean, got ${describeValue(raw)}.`,
    );
  }
  return raw;
}

export async function setOnboardingDone(done: boolean): Promise<void> {
  await invoke<unknown>("set_onboarding_done", { done });
}
