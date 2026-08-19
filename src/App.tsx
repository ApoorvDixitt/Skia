// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { AskBar } from "./components/AskBar";
import { StealthPanel } from "./components/StealthPanel";
import "./App.css";

function App() {
  return (
    <main className="container">
      <header className="app-header">
        <div className="app-identity">
          <span className="app-mark" aria-hidden="true" />
          <h1 className="app-name">Skia</h1>
          <span className="app-stage">pre-alpha</span>
        </div>
        <p className="tagline">
          A local-first meeting copilot. The overlay is real; almost everything
          behind it is not built yet.
        </p>
      </header>

      <StealthPanel />
      <AskBar />

      <footer className="app-footer">
        <p>
          Runs entirely on this device. No account, no server, no telemetry.
        </p>
      </footer>
    </main>
  );
}

export default App;
