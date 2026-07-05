import { WakeState } from "./domain/wake";
import { CallState } from "./domain/call";
import { ErrorViewState } from "./view/error";
import { RoutingState } from "./view/routing";

export class DomainState {
  wake = new WakeState();
  call = new CallState();
}

export class ViewState {
  routing = new RoutingState();
  error = new ErrorViewState();
}
