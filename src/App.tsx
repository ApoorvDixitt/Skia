// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { getCurrentWindow } from "@tauri-apps/api/window";

import { Dashboard } from "./dashboard/Dashboard";
import { Overlay } from "./overlay/Overlay";
import "./styles/tokens.css";

/**
 * Skia runs two windows from one bundle, so the entry point picks a surface by
 * window label:
 *
 * - `overlay`   the compact always-on-top bar that sits over a call
 * - `dashboard` the full window for the knowledge base, history and settings
 *
 * The label is read at module scope. `getCurrentWindow()` reads it from the
 * page's own context rather than over IPC, so there is no loading state and no
 * flash of the wrong surface.
 */
const label = getCurrentWindow().label;

function App() {
  if (label === "dashboard") return <Dashboard />;
  if (label === "overlay") return <Overlay />;

  // Neither label matched. Say so rather than guessing a surface: a silent
  // fallback would make a config mistake look like a rendering bug.
  return (
    <main className="unknown-surface">
      <p className="legend">Unknown window</p>
      <p>
        This window is labelled <code className="measured">{label}</code>, which
        Skia has no interface for. Expected{" "}
        <code className="measured">overlay</code> or{" "}
        <code className="measured">dashboard</code>.
      </p>
    </main>
  );
}

export default App;
