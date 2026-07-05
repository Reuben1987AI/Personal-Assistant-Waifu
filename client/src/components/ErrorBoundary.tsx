import { Component, type ErrorInfo, type ReactNode } from "react";
import { state } from "../stores/stores";
import { workflows } from "../stores/stores";

// Root Error Boundary — the React equivalent of naturaleza's GlobalErrorHandler.
// Catches render errors and unhandled throws from workflows called during
// render/lifecycle. Writes to `state.view.error` and forwards to the Rust side
// via `console_log` (analog of /user/client-logs), then renders the error screen.
type Props = { children: ReactNode };
type State = { hasError: boolean };

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false };

  static getDerivedStateFromError(): State {
    return { hasError: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    state.view.error.setError(error);
    void workflows.call.logToRust(
      `render error: ${error.message}\n${info.componentStack ?? ""}`,
    );
  }

  render(): ReactNode {
    if (this.state.hasError) {
      const msg = state.view.error.message ?? "Something went wrong";
      return (
        <div id="error-screen">
          <h1>Kassandra crashed</h1>
          <pre>{msg}</pre>
          <button onClick={() => { state.view.error.clear(); this.setState({ hasError: false }); }}>
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
