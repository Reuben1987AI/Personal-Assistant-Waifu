import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";
import type { CallStateName } from "../stores/state/domain/call";

const STATE_LABELS: Record<CallStateName, string> = {
  idle: "Click Call to start",
  wake_detected: "Listening...",
  connecting: "Connecting...",
  connected: "Connected",
  listening: "Listening to you",
  speaking: "Kassandra is speaking",
  disconnected: "Disconnected",
};

export const StatusBar = observer(function StatusBar() {
  const call = state.domain.call;
  const label = call.errorMsg
    ? `Error: ${call.errorMsg}`
    : STATE_LABELS[call.callState];
  return (
    <header id="status-bar">
      <div id="status-indicator" className={`status ${call.callState}`} />
      <div id="status-text">{label}</div>
    </header>
  );
});
