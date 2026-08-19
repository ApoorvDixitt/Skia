// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The full honest stealth panel. Everything here renders what the backend
 * measured, without upgrading it:
 *
 * - Until the status call answers, nothing is claimed. If it fails, the
 *   window is treated as fully visible — unknown is not the same as
 *   protected.
 * - The window's enumerability is always surfaced as a limitation. Capture
 *   exclusion withholds pixels, never presence.
 * - Every caveat the backend sends is rendered, in the flow, never collapsed.
 *
 * One scoping fact, stated rather than glossed: the status commands operate
 * on the window that calls them, so this section reports on the dashboard
 * window you are reading it in. The request itself is stored once for the
 * whole app.
 */

import { useId } from "react";

import { useStealthStatus } from "../lib/useStealthStatus";
import { CaptureTier, PresenceTier } from "./StatusTiers";
import { FailNote } from "./notes";
import "./status.css";

function platformLabel(platform: string): string {
  if (platform === "macos") return "macOS";
  if (platform === "windows") return "Windows";
  if (platform.trim().length === 0) return "unknown platform";
  return platform;
}

interface EnumerationNoteProps {
  enumerable: boolean;
}

function EnumerationNote({ enumerable }: EnumerationNoteProps) {
  const headingId = useId();
  return (
    <section className="st-limitation" aria-labelledby={headingId}>
      <h3 className="st-limitation-title" id={headingId}>
        Pixels are hidden. Presence is not.
      </h3>
      {enumerable ? (
        <p className="st-limitation-text">
          Any app that asks the OS can still see that this window exists,
          which process owns it, and its size and position — on every OS,
          whatever the tier above says. Capture exclusion withholds what the
          window shows, never that it is there.
        </p>
      ) : (
        <p className="st-limitation-text">
          The backend reported this window as not enumerable. No operating
          system offers that, so read it as a bug in the status code rather
          than as cover.
        </p>
      )}
      <p className="st-limitation-scope">
        Nor does any of this defend against device management, kernel-level
        monitoring, or a camera pointed at your screen. No user-space app can,
        and Skia does not claim to.
      </p>
    </section>
  );
}

interface CaveatListProps {
  caveats: string[];
}

function CaveatList({ caveats }: CaveatListProps) {
  const headingId = useId();
  if (caveats.length === 0) return null;

  return (
    <section className="st-caveats" aria-labelledby={headingId}>
      <h3 className="st-caveats-title" id={headingId}>
        Caveats <span className="st-caveats-count">{caveats.length}</span>
      </h3>
      <ul className="st-caveats-list">
        {caveats.map((caveat, index) => (
          <li key={`${String(index)}:${caveat}`}>{caveat}</li>
        ))}
      </ul>
    </section>
  );
}

export function Status() {
  const { state, pending, actionError, refresh, setCaptureExclusion } =
    useStealthStatus();

  return (
    <>
      <header className="db-head">
        <div className="db-head-copy">
          <h2 className="db-title">Stealth status</h2>
          <p className="db-subtitle">
            What is actually active for this window — not what we would like
            to promise.
          </p>
        </div>
        <div className="db-head-side">
          {state.kind === "ready" ? (
            <p className="st-os">
              <span className="st-os-name">
                {platformLabel(state.status.platform)}
              </span>
              <span className="measured">
                {state.status.osVersion.trim().length > 0
                  ? state.status.osVersion
                  : "version unreported"}
              </span>
            </p>
          ) : null}
          <button
            type="button"
            className="db-button db-button--ghost"
            onClick={refresh}
            disabled={pending || state.kind === "loading"}
          >
            Re-check
          </button>
        </div>
      </header>

      <div className="db-body">
        <div className="db-body-inner">
          {state.kind === "loading" ? (
            <div className="st-pending" role="status">
              <span className="db-spinner" aria-hidden="true" />
              <div className="st-pending-copy">
                <p className="st-pending-headline">
                  Checking what is actually active…
                </p>
                <p className="st-pending-detail">
                  Nothing is claimed until the system answers.
                </p>
              </div>
            </div>
          ) : null}

          {state.kind === "failed" ? (
            <FailNote
              headline="Stealth status is unknown"
              detail="The status call failed, so treat this window as fully visible. Unknown is not the same as protected."
              message={state.message}
              onRetry={refresh}
              retryLabel="Re-check"
            />
          ) : null}

          {state.kind === "ready" ? (
            <>
              {actionError === null ? null : (
                <FailNote
                  headline="Could not change capture exclusion"
                  detail="The panel below still shows the last status the backend confirmed, not the change you asked for. Re-check before trusting it."
                  message={actionError}
                  onRetry={refresh}
                  retryLabel="Re-check"
                />
              )}
              <div className="st-tiers db-stagger">
                <CaptureTier
                  exclusion={state.status.captureExclusion}
                  pending={pending}
                  onChange={setCaptureExclusion}
                />
                <PresenceTier presence={state.status.presence} />
              </div>
              <EnumerationNote enumerable={state.status.windowEnumerable} />
              <CaveatList caveats={state.status.caveats} />
            </>
          ) : null}
        </div>
      </div>
    </>
  );
}
