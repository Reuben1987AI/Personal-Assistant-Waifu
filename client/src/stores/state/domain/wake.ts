import { makeAutoObservable } from "mobx";

// Wake-word detection state. Driven entirely by the Tauri `wake_*` events
// fanned out from the adapter in main.tsx. No business logic, no invokes.

export type WakeStateName =
  | "listening"
  | "hearing"
  | "processing"
  | "fired"
  | "rejected"
  | "error";

export interface WakeStatePayload {
  state: WakeStateName;
  score?: number;
  msg?: string;
}

export class WakeState {
  wakeState: WakeStateName = "listening";
  score: number | null = null;
  lastRms = 0;
  lastVad = 0;
  errorMsg: string | null = null;

  constructor() {
    makeAutoObservable(this);
  }

  setWakeState(state: WakeStateName, score: number | null = null, msg: string | null = null): void {
    this.wakeState = state;
    this.score = score;
    this.errorMsg = msg;
  }

  setRms(v: number): void {
    this.lastRms = v;
  }

  setVad(v: number): void {
    this.lastVad = v;
  }

  reset(): void {
    this.wakeState = "listening";
    this.score = null;
    this.lastRms = 0;
    this.lastVad = 0;
    this.errorMsg = null;
  }
}
