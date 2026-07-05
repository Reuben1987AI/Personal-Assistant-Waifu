import { state } from "../stores";
import type { TeacherSnapshotPayload } from "../state/domain/teacher";

// Atomic use cases for the chinese teacher mode. The Tauri event adapter in
// main.tsx calls these; components read `state.domain.teacher` for display.
// No try/catch — unexpected payloads throw and bubble to the Error Boundary.
// The Rust side is authoritative for teacher state (AppState.teacher +
// progress.json); the frontend never mutates this state directly.

export class TeacherWorkflow {
  // `app_mode`: bare boolean toggling the active flag (drives the badge).
  setModeActive(active: boolean): void {
    state.domain.teacher.setActive(active);
  }

  // `teacher_state`: full snapshot from Rust.
  applySnapshot(p: TeacherSnapshotPayload): void {
    state.domain.teacher.applySnapshot(p);
  }

  // Future exit button could call a Rust command here; tools-driven exit
  // (LLM calls exit_chinese_teacher_mode) flows back through `app_mode`
  // + `teacher_state` events, so no invoke is needed for that path today.
}