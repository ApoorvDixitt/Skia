// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

/**
 * Step 1 — the mark, the name, one line on what Skia is, one action. Nothing
 * to decide yet, so nothing else is offered.
 */

import { Mark } from "../ui/Mark";

interface WelcomeProps {
  onBegin: () => void;
}

export function Welcome({ onBegin }: WelcomeProps) {
  return (
    <>
      <div className="ob-hello">
        <Mark size={40} label="Skia" className="ob-hello-mark" />
        <h1 className="ob-title ob-title--hello">Skia</h1>
      </div>
      <p className="ob-lede">
        A local-first meeting copilot: answers grounded in your own documents,
        from a model you choose, on this device.
      </p>
      <p className="ob-hint">
        Setup takes about two minutes and can be skipped — everything here can
        be changed later.
      </p>
      <div className="ob-actions">
        <button type="button" className="ob-button" onClick={onBegin}>
          Set up Skia
        </button>
      </div>
    </>
  );
}
