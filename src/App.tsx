// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [greetMsg, setGreetMsg] = useState("");
  const [name, setName] = useState("");

  // Placeholder round-trip into the Rust core, kept as a reference for how the
  // frontend calls native code. Replace once real commands exist.
  async function greet() {
    setGreetMsg(await invoke("greet", { name }));
  }

  return (
    <main className="container">
      <h1>Skia</h1>
      <p className="tagline">
        A local-first meeting copilot. Nothing is wired up yet — this is the
        application shell.
      </p>

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          greet();
        }}
      >
        <input
          id="greet-input"
          onChange={(e) => setName(e.currentTarget.value)}
          placeholder="Enter a name..."
        />
        <button type="submit">Greet</button>
      </form>
      <p>{greetMsg}</p>
    </main>
  );
}

export default App;
