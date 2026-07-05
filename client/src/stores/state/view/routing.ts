import { makeAutoObservable } from "mobx";

// Preemptive routing scaffold. Only `chat` exists today; new screens (avatar,
// memory editor, game mode, browser control) add a variant + a `goTo*()`.
//
// Do NOT add a generic `goTo(route)` method — always add a specific
// `goTo<Screen>()` per route, mirroring naturaleza's RoutingState.

export type Route = { type: "chat" };

export class RoutingState {
  route: Route = { type: "chat" };

  constructor() {
    makeAutoObservable(this);
  }

  goToChat(): void {
    this.route = { type: "chat" };
  }
}
