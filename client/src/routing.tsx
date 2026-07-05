import { observer } from "mobx-react-lite";
import { state } from "./stores/stores";
import { ChatScreen } from "./components/ChatScreen";

// Switch over state.view.routing.route. Only `chat` today; new screens drop in
// as new cases alongside a goTo*() in RoutingState.
export const Routing = observer(function Routing() {
  switch (state.view.routing.route.type) {
    case "chat":
      return <ChatScreen />;
  }
});
