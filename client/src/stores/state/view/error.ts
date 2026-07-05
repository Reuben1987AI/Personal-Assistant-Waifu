import { makeAutoObservable } from "mobx";

// Current unhandled error. Written by the root Error Boundary's
// `componentDidCatch` and by the Tauri adapter when a throw would escape
// React's render cycle (event-listener fire-and-forget). Read by the error
// screen rendered from this state.

export class ErrorViewState {
  error: Error | null = null;
  message: string | null = null;

  constructor() {
    makeAutoObservable(this);
  }

  setError(e: Error | null, message: string | null = null): void {
    this.error = e;
    this.message = message ?? e?.message ?? null;
  }

  clear(): void {
    this.error = null;
    this.message = null;
  }
}
