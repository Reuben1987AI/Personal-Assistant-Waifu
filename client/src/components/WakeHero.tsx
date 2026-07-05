import { useEffect, useRef, useState } from "react";
import { autorun } from "mobx";
import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";

// 24 bars around the circle perimeter. Each bar's height is driven by a
// per-bar CSS variable --h (0..1) set imperatively from wake_rms.
// wake_rms is emitted every 100ms unconditionally; bars react continuously
// to ambient audio energy. Hero visibility is driven by callState (hidden
// during a call).
const N_BARS = 24;
const BAR_OFFSETS = Array.from({ length: N_BARS }, (_, i) => 0.55 + 0.45 * Math.sin(i * 1.3));

export const WakeHero = observer(function WakeHero() {
  const wake = state.domain.wake;
  const call = state.domain.call;
  const wakeState = wake.wakeState;
  const score = wake.score;
  // Hero hidden during a call (connecting/connected/listening/speaking/
  // wake_detected); shown idle/disconnected. Mirrors the committed
  // setState(): `wakeHero.classList.toggle("hidden", inCall)`.
  const inCall = !["idle", "disconnected"].includes(call.callState);

  const [scoreText, setScoreText] = useState("");
  const barRefs = useRef<(HTMLDivElement | null)[]>([]);

  // Drive bar --h from RMS continuously. wake_rms is emitted every 100ms
  // unconditionally, so bars react to ambient audio energy at all times.
  // fired/error → leave --h alone so CSS keyframes take over.
  useEffect(() => {
    const dispose = autorun(() => {
      const st = state.domain.wake.wakeState;
      if (st !== "listening") return;
      const rms = state.domain.wake.lastRms;
      for (let i = 0; i < N_BARS; i++) {
        const el = barRefs.current[i];
        if (!el) continue;
        const h = Math.min(1, rms * BAR_OFFSETS[i] * 1.4 + 0.08);
        el.style.setProperty("--h", h.toFixed(3));
      }
    });
    return dispose;
  }, []);

  // Score text on wake fired. 2.5s display window; cleared otherwise.
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    if (wakeState === "fired" && score !== null) {
      setScoreText(`score ${score.toFixed(2)} \u2713`);
      t = setTimeout(() => setScoreText(""), 2500);
    } else {
      setScoreText("");
    }
    return () => { if (t) clearTimeout(t); };
  }, [wakeState, score]);

  return (
    <div id="wake-hero" data-state={wakeState} className={inCall ? "hidden" : ""}>
      <div id="wake-circle">
        <div id="wake-bars">
          {Array.from({ length: N_BARS }, (_, i) => (
            <div
              key={i}
              className="wake-bar"
              ref={(el) => { barRefs.current[i] = el; }}
              style={{ "--rot": `${(i * 360) / N_BARS}deg` } as React.CSSProperties}
            />
          ))}
        </div>
      </div>
      <div id="wake-label" />
      <div id="wake-score">{scoreText}</div>
    </div>
  );
});
