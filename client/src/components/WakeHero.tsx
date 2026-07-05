import { useEffect, useRef, useState } from "react";
import { autorun } from "mobx";
import { observer } from "mobx-react-lite";
import { state } from "../stores/stores";

// 24 bars around the circle perimeter. Each bar's height is driven by a
// per-bar CSS variable --h (0..1) set imperatively from wake_rms.
// - listening : idle. No wake_rms emitted; bars hold a static low baseline
//   and the container `wake-breathe` keyframe animates them. Pure CSS.
// - hearing   : RMS gate tripped; bars scale with wake_rms (denoised energy).
// Hero visibility is driven by callState (hidden during a call), NOT wake
// state — manual call mode never emits wake_state. The wake-state score
// display stays for when KASSANDRA_WAKE_ENABLED=true.
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

  // Drive bar --h from RMS. listening → static 0.18 baseline (CSS breathes);
  // hearing → lastRms; processing/fired/rejected/error → leave --h alone so
  // CSS keyframes (spin/burst/flat) take over without inline override.
  useEffect(() => {
    const dispose = autorun(() => {
      const st = state.domain.wake.wakeState;
      const rms = state.domain.wake.lastRms;
      let drive: number;
      if (st === "hearing") {
        drive = rms;
      } else if (st === "listening") {
        drive = 0.18;
      } else {
        return;
      }
      for (let i = 0; i < N_BARS; i++) {
        const el = barRefs.current[i];
        if (!el) continue;
        const h = Math.min(1, drive * BAR_OFFSETS[i] * 1.4 + 0.08);
        el.style.setProperty("--h", h.toFixed(3));
      }
    });
    return dispose;
  }, []);

  // Score text on wake fired/rejected (only fires when wake is enabled).
  // 2.5s display window; cleared otherwise. Dormant in manual call mode.
  useEffect(() => {
    let t: ReturnType<typeof setTimeout> | undefined;
    if (wakeState === "fired" && score !== null) {
      setScoreText(`score ${score.toFixed(2)} \u2713`);
      t = setTimeout(() => setScoreText(""), 2500);
    } else if (wakeState === "rejected" && score !== null) {
      setScoreText(`score ${score.toFixed(2)} \u2717`);
      t = setTimeout(() => setScoreText(""), 2500);
    } else if (wakeState === "error") {
      setScoreText("");
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
