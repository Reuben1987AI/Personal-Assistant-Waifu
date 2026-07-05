import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";
import { StatusBar } from "./StatusBar";
import { WakeHero } from "./WakeHero";
import { Messages } from "./Messages";
import { Controls } from "./Controls";

// The single `chat` screen. Composes the presentational components; reads no
// state for display itself — each child reads its own slice via observer().
export const ChatScreen = observer(function ChatScreen() {
  if (state.view.error.error) {
    return (
      <div id="error-screen">
        <h1>Kassandra error</h1>
        <pre>{state.view.error.message}</pre>
        <button onClick={() => state.view.error.clear()}>Dismiss</button>
      </div>
    );
  }
  return (
    <>
      <StatusBar />
      <main id="chat">
        <WakeHero />
        <Messages />
      </main>
      <Controls />
    </>
  );
});
