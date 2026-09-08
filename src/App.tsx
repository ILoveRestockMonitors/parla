import { useEffect, useState } from "react";

/**
 * Parla HUD placeholder (Phase 1 wires real events via Tauri IPC).
 * States: idle -> recording -> finalizing -> inserting -> idle
 */
type HudState = "idle" | "recording" | "finalizing" | "inserting";

export default function App() {
  const [state, setState] = useState<HudState>("idle");

  useEffect(() => {
    // TODO(Phase 1): listen("hud_state") from ipc.rs; show review_suggested cue.
    return () => {};
  }, []);

  return (
    <div style={{ fontFamily: "system-ui", padding: 16 }}>
      <h1 style={{ fontSize: 18 }}>Parla</h1>
      <p>
        Hold <kbd>Ctrl</kbd>+<kbd>Win</kbd> and speak. Release to insert cleaned
        text at your cursor.
      </p>
      <div aria-live="polite">
        state: <strong>{state}</strong>
        {" "}
        <button onClick={() => setState(state === "idle" ? "recording" : "idle")}>
          toggle (placeholder)
        </button>
      </div>
      {/* TODO(Phase 6): waveform, review_suggested cue, permission fix-it card */}
    </div>
  );
}
