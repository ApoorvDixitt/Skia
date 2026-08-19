// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useId } from "react";
import type { SupportLevel } from "../lib/types";
import "./stealth.css";

interface SupportCopy {
  badge: string;
  caption: string;
}

/**
 * `measured` is never allowed to read like `documented`. It gets weaker wording
 * here, weaker weight in CSS (dashed outline, no fill, muted ink), and it sits a
 * rung lower on the scale below.
 */
const SUPPORT_COPY: Record<SupportLevel, SupportCopy> = {
  documented: {
    badge: "Documented",
    caption:
      "The OS vendor documents and supports this mechanism. This is as strong as a guarantee gets for an app running in user space.",
  },
  measured: {
    badge: "Measured · no guarantee",
    caption:
      "Undocumented behaviour that we measured working on this OS version. It is not promised, the vendor advises against relying on it, and a point release can remove it without notice.",
  },
  unavailable: {
    badge: "Unavailable",
    caption: "This operating system offers no mechanism at all.",
  },
};

/** Strongest first. `unavailable` lands on `none` so the drop is visible. */
const SCALE = [
  { key: "documented", label: "documented" },
  { key: "measured", label: "measured" },
  { key: "none", label: "none" },
] as const;

function activeRung(level: SupportLevel): string {
  return level === "unavailable" ? "none" : level;
}

interface SupportBadgeProps {
  level: SupportLevel;
}

export function SupportBadge({ level }: SupportBadgeProps) {
  return (
    <span className="support-badge" data-level={level}>
      <span className="support-badge-mark" aria-hidden="true" />
      <span className="support-badge-text">{SUPPORT_COPY[level].badge}</span>
    </span>
  );
}

interface SupportScaleProps {
  level: SupportLevel;
}

export function SupportScale({ level }: SupportScaleProps) {
  const titleId = useId();
  const current = activeRung(level);

  return (
    <div className="support-scale" data-level={level}>
      <span className="support-scale-title" id={titleId}>
        Strength of the guarantee
      </span>
      <ol className="support-scale-track" aria-labelledby={titleId}>
        {SCALE.map((rung) => {
          const isCurrent = rung.key === current;
          return (
            <li
              key={rung.key}
              className="support-scale-rung"
              data-current={isCurrent}
              data-rung={rung.key}
              aria-current={isCurrent ? "step" : undefined}
            >
              <span className="support-scale-bar" aria-hidden="true" />
              <span className="support-scale-label">{rung.label}</span>
            </li>
          );
        })}
      </ol>
      <p className="support-scale-caption">{SUPPORT_COPY[level].caption}</p>
    </div>
  );
}
