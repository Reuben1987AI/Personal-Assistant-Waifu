import { listen } from "@tauri-apps/api/event";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { state, workflows } from "./stores/stores";
import type { WakeStatePayload } from "./stores/state/domain/wake";
import "./styles.css";

// The single place `listen()` is called. Components/workflows never register
// listeners directly. Each handler validates shape/enum (fail fast) and routes
// to a workflow. A throw in an event-listener fire-and-forget cannot reach the
// React Error Boundary, so the adapter catches solely to surface the error via
// `state.view.error` + `console_log` — this is surfacing, not swallowing.
async function route(label: string, fn: () => void): Promise<void> {
  try {
    fn();
  } catch (e) {
    const err = e instanceof Error ? e : new Error(String(e));
    state.view.error.setError(err, `event ${label}: ${err.message}`);
    await workflows.call.logToRust(`event ${label}: ${err.message}`);
  }
}

void listen("wake_state", (e) =>
  void route("wake_state", () => workflows.call.applyWakeState(e.payload as string | WakeStatePayload)));

void listen("wake_rms", (e) =>
  void route("wake_rms", () => workflows.call.setWakeRms(e.payload as number)));

void listen("wake_vad", (e) =>
  void route("wake_vad", () => workflows.call.setWakeVad(e.payload as number)));

void listen("qwen_state", (e) =>
  void route("qwen_state", () => workflows.call.setCallState(e.payload as string)));

void listen("user_transcript", (e) =>
  void route("user_transcript", () => workflows.call.appendUser(e.payload as string)));

void listen("qwen_transcript", (e) =>
  void route("qwen_transcript", () => workflows.call.streamKass(e.payload as string)));

void listen("qwen_response", (e) =>
  void route("qwen_response", () => workflows.call.finalizeKass(e.payload as string)));

void listen("qwen_error", (e) =>
  void route("qwen_error", () => workflows.call.setQwenError(e.payload as string)));

void workflows.call.logToRust("frontend: listeners registered");

ReactDOM.createRoot(document.getElementById("app")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
