import { makeAutoObservable } from "mobx";

// Qwen call state. `callState` mirrors the `qwen_state` event enum; `messages`
// is the chat transcript; `streamingKass` is the in-flight assistant bubble
// being built from `qwen_transcript` deltas until `qwen_response` finalizes it.

export type CallStateName =
  | "idle"
  | "wake_detected"
  | "connecting"
  | "connected"
  | "listening"
  | "speaking"
  | "disconnected";

export type MessageKind = "user" | "kass" | "system";

export interface Message {
  id: number;
  kind: MessageKind;
  text: string;
}

export class CallState {
  callState: CallStateName = "idle";
  messages: Message[] = [];
  streamingKass: Message | null = null;
  muted = false;
  errorMsg: string | null = null;
  private nextId = 1;

  constructor() {
    makeAutoObservable(this);
  }

  setCallState(s: CallStateName): void {
    this.callState = s;
  }

  addMessage(kind: MessageKind, text: string): Message {
    const msg: Message = { id: this.nextId++, kind, text };
    this.messages.push(msg);
    return msg;
  }

  // A user/system turn ends any in-flight assistant stream so the next
  // qwen_transcript delta starts a fresh bubble below this message, not
  // appended to the previous turn's still-open bubble above it. Mirrors
  // `currentKassBubble = null` in the original main.js.
  appendUser(text: string): void {
    this.messages.push({ id: this.nextId++, kind: "user", text });
    this.streamingKass = null;
  }

  appendSystem(text: string): void {
    this.messages.push({ id: this.nextId++, kind: "system", text });
    this.streamingKass = null;
  }

  // Begin a streaming assistant bubble (or reuse the in-flight one). Returns
  // the bubble so the workflow can append deltas to it.
  beginStreamingKass(): Message {
    if (!this.streamingKass) {
      this.streamingKass = { id: this.nextId++, kind: "kass", text: "" };
      this.messages.push(this.streamingKass);
    }
    return this.streamingKass;
  }

  appendKassDelta(delta: string): void {
    const bubble = this.beginStreamingKass();
    bubble.text += delta;
  }

  finalizeKass(finalText: string): void {
    if (this.streamingKass) {
      this.streamingKass.text = finalText;
      this.streamingKass = null;
    } else {
      this.addMessage("kass", finalText);
    }
  }

  setMuted(m: boolean): void {
    this.muted = m;
  }

  setError(msg: string): void {
    this.errorMsg = msg;
    this.callState = "disconnected";
  }

  reset(): void {
    this.callState = "idle";
    this.messages.length = 0;
    this.streamingKass = null;
    this.muted = false;
    this.errorMsg = null;
  }
}
