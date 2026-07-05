import { observer } from "mobx-react-lite";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Routing } from "./routing";

// observer-wrapped root. The Error Boundary catches render/lifecycle throws;
// event-listener throws are routed to state.view.error by the adapter in
// main.tsx and surface through the same error state.
export const App = observer(function App() {
  return (
    <ErrorBoundary>
      <Routing />
    </ErrorBoundary>
  );
});
