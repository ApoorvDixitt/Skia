// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * The dashboard: Skia's second surface. The overlay stays a thin bar over a
 * call because everything that needs room — the knowledge base, history,
 * provider keys, prompt editing, the honest status readout, and the data
 * controls — lives here instead.
 *
 * One rail, one pane. The active section is the only one mounted, so every
 * visit re-reads the database instead of trusting whatever was on screen the
 * last time; after a purge or an ingest, what you see is what was read.
 */

import { useLayoutEffect, useRef, useState } from "react";
import type { ReactElement } from "react";

import { Mark } from "../ui/Mark";
import { Audio } from "./Audio";
import { HistorySection } from "./HistorySection";
import { KnowledgeBase } from "./KnowledgeBase";
import { Meetings } from "./Meetings";
import { Prompts } from "./Prompts";
import { Providers } from "./Providers";
import { Status } from "./Status";
import { YourData } from "./YourData";
import {
  IconAudio,
  IconData,
  IconMeetings,
  IconHistory,
  IconKnowledge,
  IconPrompts,
  IconProviders,
  IconStatus,
} from "./icons";
import "./dashboard.css";

type SectionId =
  | "knowledge"
  | "meetings"
  | "history"
  | "audio"
  | "providers"
  | "prompts"
  | "status"
  | "data";

interface SectionDef {
  id: SectionId;
  label: string;
  icon: ReactElement;
}

const SECTIONS: readonly SectionDef[] = [
  { id: "knowledge", label: "Knowledge base", icon: <IconKnowledge /> },
  { id: "meetings", label: "Meetings", icon: <IconMeetings /> },
  { id: "history", label: "History", icon: <IconHistory /> },
  { id: "audio", label: "Audio", icon: <IconAudio /> },
  { id: "providers", label: "Providers", icon: <IconProviders /> },
  { id: "prompts", label: "Prompts", icon: <IconPrompts /> },
  { id: "status", label: "Status", icon: <IconStatus /> },
  { id: "data", label: "Your data", icon: <IconData /> },
];

/** The needle's height in px. Kept in sync with `.db-needle` in dashboard.css. */
const NEEDLE_HEIGHT = 15;

export function Dashboard() {
  const [section, setSection] = useState<SectionId>("knowledge");

  // The amber needle is one element that travels between items rather than a
  // per-item marker that hard-cuts. Its position is measured from the real
  // layout — never derived from index arithmetic — so it cannot drift from
  // what is actually on screen. Selection itself never depends on the needle:
  // `aria-current` still restyles the active item.
  const [needleTop, setNeedleTop] = useState<number | null>(null);
  const frameRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef(new Map<SectionId, HTMLButtonElement>());

  useLayoutEffect(() => {
    const measure = (): void => {
      const item = itemRefs.current.get(section);
      if (item === undefined) return;
      setNeedleTop(item.offsetTop + (item.offsetHeight - NEEDLE_HEIGHT) / 2);
    };
    measure();
    // Re-measure if the rail relayouts (OS font size, window scaling).
    const frame = frameRef.current;
    if (frame === null) return;
    const observer = new ResizeObserver(measure);
    observer.observe(frame);
    return () => {
      observer.disconnect();
    };
  }, [section]);

  return (
    <div className="db-shell grain">
      <nav className="db-rail" aria-label="Skia sections">
        <div className="db-brand">
          <Mark size={22} />
          <div className="db-brand-copy">
            <span className="db-brand-name">Skia</span>
            <span className="legend">Dashboard</span>
          </div>
        </div>

        <div className="db-nav-frame" ref={frameRef}>
          {/* Mounted only once measured, so first paint lands in place
              instead of sliding in from nowhere. */}
          {needleTop === null ? null : (
            <span
              className="db-needle"
              style={{ transform: `translateY(${String(needleTop)}px)` }}
              aria-hidden="true"
            />
          )}
          <ul className="db-nav">
            {SECTIONS.map((entry) => (
              <li key={entry.id}>
                <button
                  type="button"
                  className="db-nav-item"
                  aria-current={entry.id === section ? "page" : undefined}
                  ref={(node) => {
                    if (node === null) {
                      itemRefs.current.delete(entry.id);
                    } else {
                      itemRefs.current.set(entry.id, node);
                    }
                  }}
                  onClick={() => {
                    setSection(entry.id);
                  }}
                >
                  {entry.icon}
                  <span>{entry.label}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>

        <p className="db-rail-foot legend">On-device · No telemetry</p>
      </nav>

      {/* Keyed by section so switching remounts: fresh reads, honest data. */}
      <main className="db-main" key={section}>
        {section === "knowledge" ? <KnowledgeBase /> : null}
        {section === "meetings" ? <Meetings /> : null}
        {section === "history" ? <HistorySection /> : null}
        {section === "audio" ? <Audio /> : null}
        {section === "providers" ? <Providers /> : null}
        {section === "prompts" ? <Prompts /> : null}
        {section === "status" ? <Status /> : null}
        {section === "data" ? <YourData /> : null}
      </main>
    </div>
  );
}
