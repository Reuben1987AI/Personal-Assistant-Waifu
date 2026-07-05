import { invoke } from "@tauri-apps/api/core";
import { state } from "../stores";
import type {
  CallStateName,
} from "../state/domain/call";
import type {
  WakeStateName,
  WakeStatePayload,
} from "../state/domain/wake";

// Atomic use cases for the voice call. The Tauri event adapter in main.tsx
// calls these; components call `toggleMute`/`endCall` fire-and-forget. No
// try/catch — unexpected payloads throw and bubble to the Error Boundary.

const WAKE_STATES: ReadonlySet<WakeStateName> = new Set([
  "listening", "hearing", "processing", "fired", "rejected", "error",
]);

const CALL_STATES: ReadonlySet<CallStateName> = new Set([
  "idle", "wake_detected", "connecting", "connected", "listening", "speaking", "disconnected",
]);

function assertWakeState(s: string): asserts s is WakeStateName {
  if (!WAKE_STATES.has(s as WakeStateName)) {
    throw new Error(`unknown wake_state: ${s}`);
  }
}

function assertCallState(s: string): asserts s is CallStateName {
  if (!CALL_STATES.has(s as CallStateName)) {
    throw new Error(`unknown qwen_state: ${s}`);
  }
}

export class CallWorkflow {
  // `wake_state` payload is either a bare string or `{ state, score?, msg? }`.
  applyWakeState(payload: string | WakeStatePayload): void {
    const name: string = typeof payload === "string" ? payload : payload.state;
    assertWakeState(name);
    const score = typeof payload !== "string" && typeof payload.score === "number"
      ? payload.score
      : null;
    const msg = typeof payload !== "string" && typeof payload.msg === "string"
      ? payload.msg
      : null;
    state.domain.wake.setWakeState(name, score, msg);

    if (name === "fired" && score !== null) {
      state.domain.call.addMessage("system", `wake score ${score.toFixed(3)} detected`);
    }
    if (name === "error" && msg) {
      // Wake-side error: surface inline (status bar), do not crash the app.
      state.domain.call.setError(msg);
    }
  }

  setWakeRms(v: number): void {
    state.domain.wake.setRms(v);
  }

  setWakeVad(v: number): void {
    state.domain.wake.setVad(v);
  }

  setCallState(s: string): void {
    assertCallState(s);
    state.domain.call.setCallState(s);
  }

  appendUser(text: string): void {
    state.domain.call.appendUser(text);
  }

  appendSystem(text: string): void {
    state.domain.call.appendSystem(text);
  }

  streamKass(delta: string): void {
    state.domain.call.appendKassDelta(delta);
  }

  finalizeKass(text: string): void {
    state.domain.call.finalizeKass(text);
  }

  // `qwen_error`: inline error in the status bar, not a crash.
  setQwenError(text: string): void {
    state.domain.call.setError(text);
  }

  // Components call these fire-and-forget. The invoke return is handled here,
  // never in the component.
  async toggleMute(): Promise<void> {
    const muted = await invoke<boolean>("toggle_mute");
    state.domain.call.setMuted(muted);
  }

  async endCall(): Promise<void> {
    await invoke("end_call");
  }

  // Manual call start — the Call button. Wake word is deferred behind
  // KASSANDRA_WAKE_ENABLED (default false); this is the default entry point.
  // Backend emits qwen_state:connecting on success, which drives the UI.
  async startCall(): Promise<void> {
    await invoke("start_call");
  }

  // Forward an unhandled error to the Rust side (analog of /user/client-logs).
  // Used by the Error Boundary and the adapter. Re-throws so the caller can
  // decide whether to surface via state.view.error.
  async logToRust(message: string): Promise<void> {
    await invoke("console_log", { message });
  }
}
