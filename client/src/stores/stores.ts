import { DomainState, ViewState } from "./state";
import { CallWorkflow } from "./workflows/callWorkflow";
import { TeacherWorkflow } from "./workflows/teacherWorkflow";

// Singletons — import `state` / `workflows` directly. No Provider, no context.
export const state = new (class AppState {
  domain = new DomainState();
  view = new ViewState();
})();

export const workflows = new (class Workflows {
  call = new CallWorkflow();
  teacher = new TeacherWorkflow();
})();
