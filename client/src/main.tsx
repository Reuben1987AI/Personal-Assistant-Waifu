import { listen } from "@tauri-apps/api/event";
import React from "react";
import ReactDOM from "react-dom/client";
import { App } from "./App";
import { state, workflows } from "./stores/stores";
import type { WakeStatePayload } from "./stores/state/domain/wake";
import type { TeacherSnapshotPayload } from "./stores/state/domain/teacher";
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

// Teacher mode events. `app_mode` carries the bare active flag (boolean);
// `teacher_state` carries the full Rust-side snapshot (mode + phase + target
// + learned set + curriculum position/total). Both are validated at the
// adapter boundary — a malformed snapshot throws and surfaces via
// state.view.error, never silently accepted into a store.
void listen("app_mode", (e) =>
  void route("app_mode", () => workflows.teacher.setModeActive(e.payload as boolean)));

void listen("teacher_state", (e) =>
  void route("teacher_state", () => workflows.teacher.applySnapshot(e.payload as TeacherSnapshotPayload)));

void workflows.call.logToRust("frontend: listeners registered");

ReactDOM.createRoot(document.getElementById("app")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
