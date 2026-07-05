import { observer } from "mobx-react-lite";
import { state, workflows } from "../stores/stores";

export const Controls = observer(function Controls() {
  const call = state.domain.call;
  const inCall = !["idle", "disconnected"].includes(call.callState);
  return (
    <footer id="controls">
      <button
        className={inCall ? "hidden" : ""}
        onClick={() => void workflows.call.startCall()}
      >
        Call
      </button>
      <button
        className={inCall ? "" : "hidden"}
        onClick={() => void workflows.call.toggleMute()}
      >
        {call.muted ? "Unmute" : "Mute"}
      </button>
      <button
        className={inCall ? "" : "hidden"}
        onClick={() => void workflows.call.endCall()}
      >
        End Call
      </button>
    </footer>
  );
});
