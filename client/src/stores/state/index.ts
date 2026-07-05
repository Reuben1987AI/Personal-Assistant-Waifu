import { WakeState } from "./domain/wake";
import { CallState } from "./domain/call";
import { TeacherState } from "./domain/teacher";
import { ErrorViewState } from "./view/error";
import { RoutingState } from "./view/routing";

export class DomainState {
  wake = new WakeState();
  call = new CallState();
  teacher = new TeacherState();
}

export class ViewState {
  routing = new RoutingState();
  error = new ErrorViewState();
}
