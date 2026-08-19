// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import type { Presence } from "../lib/types";
import "./stealth.css";

interface PresenceRow {
  label: string;
  detail: string;
  /** `null` = structural, not a runtime flag the backend can report on. */
  applied: boolean | null;
}

function rowsFor(presence: Presence): PresenceRow[] {
  return [
    {
      label: "No dock icon",
      detail: "Skia does not appear alongside your running applications.",
      applied: presence.noDockIcon,
    },
    {
      label: "No taskbar entry",
      detail: "Nothing is added to the taskbar or the window list.",
      applied: presence.noTaskbarEntry,
    },
    {
      label: "Not in the app switcher",
      detail: "Alt-Tab and Command-Tab walk straight past it.",
      applied: presence.noAltTab,
    },
    {
      label: "Never steals focus",
      detail:
        "Your typing keeps going where you were typing. The overlay does not take the keyboard.",
      applied: presence.neverStealsFocus,
    },
    {
      label: "No bot joins the call",
      detail:
        "Skia runs on this machine. There is no server-side participant to appear in the meeting.",
      applied: null,
    },
  ];
}

interface PresenceTierProps {
  presence: Presence;
}

export function PresenceTier({ presence }: PresenceTierProps) {
  const headingId = useId();
  const rows = rowsFor(presence);
  const allApplied = rows.every((row) => row.applied !== false);

  return (
    <section
      className="tier tier--presence"
      data-tone={allApplied ? "strong" : "alarm"}
      aria-labelledby={headingId}
    >
      <header className="tier-header">
        <div className="tier-heading-group">
          <p className="tier-eyebrow">Tier B · same on every operating system</p>
          <h3 className="tier-title" id={headingId}>
            Stays out of the way
          </h3>
        </div>
        <span className="tier-chip" data-tone={allApplied ? "strong" : "alarm"}>
          {allApplied ? "All applied" : "Partly applied"}
        </span>
      </header>

      <p className="tier-detail">
        Ordinary window configuration rather than a capture trick, which is why
        it does not vary by OS or need measuring.
      </p>

      <ul className="presence-list">
        {rows.map((row) => (
          <li
            key={row.label}
            className="presence-row"
            data-state={
              row.applied === null
                ? "structural"
                : row.applied
                  ? "applied"
                  : "missing"
            }
          >
            <span className="presence-mark" aria-hidden="true" />
            <span className="presence-copy">
              <span className="presence-label">
                {row.label}
                {row.applied === false ? (
                  <span className="presence-flag">not applied</span>
                ) : (
                  <span className="visually-hidden">
                    {row.applied === null ? " — by design" : " — confirmed"}
                  </span>
                )}
              </span>
              <span className="presence-detail">{row.detail}</span>
            </span>
          </li>
        ))}
      </ul>

      {allApplied ? null : (
        <p className="tier-warning" role="alert">
          Something above is not applied even though it should hold on every OS.
          That is a bug — the overlay is more noticeable than intended.
        </p>
      )}
    </section>
  );
}
