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

import { useState } from "react";
import type { ReactElement } from "react";

import { HistorySection } from "./HistorySection";
import { KnowledgeBase } from "./KnowledgeBase";
import { Prompts } from "./Prompts";
import { Providers } from "./Providers";
import { Status } from "./Status";
import { YourData } from "./YourData";
import {
  BrandMark,
  IconData,
  IconHistory,
  IconKnowledge,
  IconPrompts,
  IconProviders,
  IconStatus,
} from "./icons";
import "./dashboard.css";

type SectionId =
  | "knowledge"
  | "history"
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
  { id: "history", label: "History", icon: <IconHistory /> },
  { id: "providers", label: "Providers", icon: <IconProviders /> },
  { id: "prompts", label: "Prompts", icon: <IconPrompts /> },
  { id: "status", label: "Status", icon: <IconStatus /> },
  { id: "data", label: "Your data", icon: <IconData /> },
];

export function Dashboard() {
  const [section, setSection] = useState<SectionId>("knowledge");

  return (
    <div className="db-shell grain">
      <nav className="db-rail" aria-label="Skia sections">
        <div className="db-brand">
          <BrandMark />
          <div className="db-brand-copy">
            <span className="db-brand-name">Skia</span>
            <span className="legend">Dashboard</span>
          </div>
        </div>

        <ul className="db-nav">
          {SECTIONS.map((entry) => (
            <li key={entry.id}>
              <button
                type="button"
                className="db-nav-item"
                aria-current={entry.id === section ? "page" : undefined}
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

        <p className="db-rail-foot legend">On-device · No telemetry</p>
      </nav>

      {/* Keyed by section so switching remounts: fresh reads, honest data. */}
      <main className="db-main" key={section}>
        {section === "knowledge" ? <KnowledgeBase /> : null}
        {section === "history" ? <HistorySection /> : null}
        {section === "providers" ? <Providers /> : null}
        {section === "prompts" ? <Prompts /> : null}
        {section === "status" ? <Status /> : null}
        {section === "data" ? <YourData /> : null}
      </main>
    </div>
  );
}
