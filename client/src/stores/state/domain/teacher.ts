import { makeAutoObservable } from "mobx";

// Chinese teacher mode state. Driven by the Tauri `app_mode` (active flag) and
// `teacher_state` (full snapshot) events fanned out from the adapter in
// main.tsx. The Rust side is authoritative (AppState.teacher + progress.json);
// the frontend is a passive display.

export type TeacherPhase = "idle" | "learn" | "practice_one" | "practice_all";

export interface TargetSnapshot {
  hanzi: string;
  pinyin: string;
  en: string;
  category: string;
}

export interface TeacherSnapshotPayload {
  active: boolean;
  phase: TeacherPhase;
  target: TargetSnapshot | null;
  learned: string[];
  position: number;
  total: number;
}

const PHASES: ReadonlySet<TeacherPhase> = new Set([
  "idle", "learn", "practice_one", "practice_all",
]);

function assertPhase(s: string): asserts s is TeacherPhase {
  if (!PHASES.has(s as TeacherPhase)) {
    throw new Error(`unknown teacher phase: ${s}`);
  }
}

export class TeacherState {
  active = false;
  phase: TeacherPhase = "idle";
  target: TargetSnapshot | null = null;
  learned: string[] = [];
  position = 0;
  total = 0;

  constructor() {
    makeAutoObservable(this);
  }

  // `app_mode` event: bare boolean toggle for the mode badge.
  setActive(active: boolean): void {
    this.active = active;
  }

  // `teacher_state` event: full snapshot from Rust. Validated at the boundary
  // (fail fast on unknown phase — see AGENTS.md "Fail fast at the adapter
  // boundary") so stores never see a malformed payload.
  applySnapshot(p: TeacherSnapshotPayload): void {
    assertPhase(p.phase);
    this.active = p.active;
    this.phase = p.phase;
    this.target = p.target;
    this.learned = p.learned;
    this.position = p.position;
    this.total = p.total;
  }

  reset(): void {
    this.active = false;
    this.phase = "idle";
    this.target = null;
    this.learned = [];
    this.position = 0;
    this.total = 0;
  }
}