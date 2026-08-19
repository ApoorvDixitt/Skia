// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/// Window control. Skia has two surfaces: a compact always-on-top `overlay` bar
/// that sits over a call, and a `dashboard` window for everything that needs
/// room. Keeping them separate is what lets the overlay stay small.

import { invoke } from "@tauri-apps/api/core";

/** Brings the dashboard up. It is created hidden at startup, so this is warm. */
export async function openDashboard(): Promise<void> {
  await invoke<unknown>("open_dashboard");
}

export async function hideDashboard(): Promise<void> {
  await invoke<unknown>("hide_dashboard");
}

export async function hideOverlay(): Promise<void> {
  await invoke<unknown>("hide_overlay");
}

/**
 * Asks the backend to make the overlay `height` logical pixels tall.
 *
 * Only the frontend knows how tall its content is, so the resize has to start
 * here. The backend clamps the value, so a measurement bug cannot produce a
 * window taller than the screen.
 */
export async function resizeOverlay(height: number): Promise<void> {
  if (!Number.isFinite(height) || height <= 0) {
    throw new Error(`refusing to resize the overlay to ${height}px`);
  }
  await invoke<unknown>("resize_overlay", { height });
}
