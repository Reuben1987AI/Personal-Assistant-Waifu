import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";
import type { MessageKind } from "../stores/state/domain/call";

// Each list item is its own observer so a single message mutation re-renders
// only that bubble, not the whole list (AGENTS.md → "List items need their
// own observer").
const MessageRow = observer(function MessageRow({ id }: { id: number }) {
  const msg = state.domain.call.messages.find((m) => m.id === id);
  if (!msg) return null;
  return (
    <div className={`msg ${msg.kind}`}>
      <div className={`bubble ${msg.kind}`}>{msg.text}</div>
    </div>
  );
});

export const Messages = observer(function Messages() {
  const ids = state.domain.call.messages.map((m) => m.id);
  return (
    <div id="messages">
      {ids.map((id) => (
        <MessageRow key={id} id={id} />
      ))}
    </div>
  );
});

// Kept for parity with the kind union; used by CSS classes.
export type { MessageKind };
