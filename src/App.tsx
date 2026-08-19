// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useCallback, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { Dashboard } from "./dashboard/Dashboard";
import { Onboarding } from "./onboarding/Onboarding";
import { Overlay } from "./overlay/Overlay";
import { fetchOnboardingDone } from "./lib/onboarding";
import "./styles/tokens.css";

/**
 * Skia runs two windows from one bundle, so the entry point picks a surface by
 * window label:
 *
 * - `overlay`   the compact always-on-top bar that sits over a call
 * - `dashboard` the full window: first-run setup, then the knowledge base,
 *               history, providers, prompts and status
 *
 * The label is read at module scope. `getCurrentWindow()` reads it from the
 * page's own context rather than over IPC, so there is no loading state and no
 * flash of the wrong surface.
 */
const label = getCurrentWindow().label;

function App() {
  if (label === "dashboard") return <DashboardSurface />;
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

type SetupState =
  | { kind: "checking" }
  | { kind: "failed"; message: string }
  | { kind: "ready"; done: boolean };

/**
 * The dashboard window shows first-run setup until it has been completed or
 * deliberately skipped, and the dashboard proper afterwards.
 *
 * The gate lives here rather than inside `Dashboard` so that setup is a peer of
 * the dashboard rather than a section of it — it owns the whole window while it
 * is running.
 */
function DashboardSurface() {
  const [state, setState] = useState<SetupState>({ kind: "checking" });

  useEffect(() => {
    let live = true;
    fetchOnboardingDone().then(
      (done) => {
        if (live) setState({ kind: "ready", done });
      },
      (cause: unknown) => {
        if (live) {
          setState({
            kind: "failed",
            message:
              cause instanceof Error
                ? cause.message
                : "could not read whether setup has been completed",
          });
        }
      },
    );
    return () => {
      live = false;
    };
  }, []);

  const finish = useCallback(() => {
    setState({ kind: "ready", done: true });
  }, []);

  if (state.kind === "checking") {
    // Deliberately blank: a spinner for a single local database read flashes
    // more than it informs.
    return <main className="surface-wait" aria-busy="true" />;
  }

  if (state.kind === "failed") {
    // Falling through to the dashboard would hide a real failure, and forcing
    // setup on someone who already finished it would be worse. Say what broke.
    return (
      <main className="unknown-surface">
        <p className="legend">Could not start</p>
        <p data-selectable>{state.message}</p>
      </main>
    );
  }

  return state.done ? <Dashboard /> : <Onboarding onDone={finish} />;
}

export default App;
