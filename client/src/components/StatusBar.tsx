import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";
import type { CallStateName } from "../stores/state/domain/call";
import type { TeacherPhase } from "../stores/state/domain/teacher";

const STATE_LABELS: Record<CallStateName, string> = {
  idle: "Click Call to start",
  wake_detected: "Listening...",
  connecting: "Connecting...",
  connected: "Connected",
  listening: "Listening to you",
  speaking: "Kassandra is speaking",
  disconnected: "Disconnected",
};

const PHASE_LABELS: Record<TeacherPhase, string> = {
  idle: "teacher",
  learn: "learning",
  practice_one: "practicing",
  practice_all: "review",
};

const TARGET_LABEL = (t: { hanzi: string; en: string }): string => `${t.hanzi} — ${t.en}`;

export const StatusBar = observer(function StatusBar() {
  const call = state.domain.call;
  const teacher = state.domain.teacher;
  const label = call.errorMsg
    ? `Error: ${call.errorMsg}`
    : STATE_LABELS[call.callState];

  return (
    <header id="status-bar">
      <div id="status-indicator" className={`status ${call.callState}`} />
      <div id="status-text">{label}</div>
      {teacher.active && (
        <div id="teacher-badge" title="Chinese teacher mode — say 'stop teaching' to exit">
          <span id="teacher-badge-dot" />
          {teacher.target && (
            <span id="teacher-badge-target">{TARGET_LABEL(teacher.target)}</span>
          )}
          <span id="teacher-badge-phase">{PHASE_LABELS[teacher.phase]}</span>
          {teacher.total > 0 && (
            <span id="teacher-badge-progress">
              {Math.min(teacher.position + 1, teacher.total)}/{teacher.total}
            </span>
          )}
        </div>
      )}
    </header>
  );
});