const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

function log(msg) {
  console.log(msg);
  invoke("console_log", { message: msg }).catch(() => {});
}

const statusIndicator = document.getElementById("status-indicator");
const statusText = document.getElementById("status-text");
const messages = document.getElementById("messages");
const callBtn = document.getElementById("call-btn");
const muteBtn = document.getElementById("mute-btn");
const endBtn = document.getElementById("end-btn");

const wakeHero = document.getElementById("wake-hero");
const wakeCircle = document.getElementById("wake-circle");
const wakeBars = document.getElementById("wake-bars");
const wakeLabel = document.getElementById("wake-label");
const wakeScore = document.getElementById("wake-score");

let currentKassBubble = null;

const STATE_LABELS = {
  idle: 'Click Call to start',
  wake_detected: "Listening...",
  connecting: "Connecting...",
  connected: "Connected",
  speaking: "Kassandra is speaking",
  listening: "Listening to you",
  disconnected: "Disconnected",
};

function setState(state) {
  statusIndicator.className = "status " + state;
  statusText.textContent = STATE_LABELS[state] || state;

  const inCall = !["idle", "disconnected"].includes(state);
  callBtn.classList.toggle("hidden", inCall);
  muteBtn.classList.toggle("hidden", !inCall);
  endBtn.classList.toggle("hidden", !inCall);
  wakeHero.classList.toggle("hidden", inCall);
}

function appendMessage(text, kind) {
  const row = document.createElement("div");
  row.className = "msg " + kind;
  const bubble = document.createElement("div");
  bubble.className = "bubble " + kind;
  bubble.textContent = text;
  row.appendChild(bubble);
  messages.appendChild(row);
  messages.scrollTop = messages.scrollHeight;
  return bubble;
}

function appendUser(text) {
  appendMessage(text, "user");
  currentKassBubble = null;
}

function appendSystem(text) {
  appendMessage(text, "system");
  currentKassBubble = null;
}

function streamKassDelta(delta) {
  if (!currentKassBubble) {
    currentKassBubble = appendMessage("", "kass");
  }
  currentKassBubble.textContent += delta;
  messages.scrollTop = messages.scrollHeight;
}

function finalizeKass(text) {
  if (currentKassBubble) {
    currentKassBubble.textContent = text;
    messages.scrollTop = messages.scrollHeight;
    currentKassBubble = null;
  } else {
    appendMessage(text, "kass");
  }
}

/* ===== Wake activity circle =====
   24 bars around the circle perimeter. Each bar's height is driven by a
   per-bar CSS variable --h (0..1):
     - `listening` : idle. The Rust wake loop is in energy-gate-off mode and
       emits no wake_rms; the bars are left to the CSS container-level
       breathing animation so the circle never looks frozen.
     - `hearing`   : the RMS gate tripped and the rolling-buffer wake-word
       classifier is scoring. Bars react to wake_rms (denoised chunk energy,
       0..1) so louder speech ⇒ taller bars.
   A small deterministic offset per bar keeps them from moving identically.
   State transitions are driven by wake_state events: listening / hearing /
   processing / fired / rejected / error. */

const N_BARS = 24;
const barEls = [];
const barOffsets = [];
for (let i = 0; i < N_BARS; i++) {
  const el = document.createElement("div");
  el.className = "wake-bar";
  el.style.setProperty("--rot", `${(i * 360) / N_BARS}deg`);
  barEls.push(el);
  barOffsets.push(0.55 + 0.45 * Math.sin(i * 1.3));
  wakeBars.appendChild(el);
}

let scoreTimer = null;

function clearScoreSoon() {
  if (scoreTimer) clearTimeout(scoreTimer);
  scoreTimer = setTimeout(() => {
    wakeScore.textContent = "";
  }, 2500);
}

let lastRms = 0;

function applyBarHeights() {
  // Compute target bar heights based on current wake state.
  const st = wakeHero.dataset.state;
  let drive;
  if (st === "hearing") {
    // Utterance in progress: drive purely by denoised RMS energy. The Rust
    // side already gated on energy, so any wake_rms here is real audio.
    drive = lastRms;
  } else if (st === "listening") {
    // Idle: no wake_rms is emitted in this state, so hold a static low value
    // as a baseline and let the CSS `wake-breathe` keyframe do the actual
    // motion (`--h` only sets scale, breathing animates transform on the
    // container). Keeps the circle alive without false-amplitude flutter.
    drive = 0.18;
  } else {
    // processing / fired / rejected / error — let CSS take over (spin / burst
    // / flat). Don't touch --h here so the CSS keyframes can collapse bars
    // without being overridden.
    return;
  }
  for (let i = 0; i < N_BARS; i++) {
    const h = Math.min(1, drive * barOffsets[i] * 1.4 + 0.08);
    barEls[i].style.setProperty("--h", h.toFixed(3));
  }
}

function setWakeState(state, score, msg) {
  wakeHero.dataset.state = state;

  if (scoreTimer) {
    clearTimeout(scoreTimer);
    scoreTimer = null;
  }

  if (state === "fired") {
    // Hold score under circle for ~2.5s so user can read it, then hide the
    // hero so the chat has the full space for the incoming Qwen exchange.
    // Hero reappears in `listening` state after the call ends.
    wakeScore.textContent = `score ${score.toFixed(2)} ✓`;
    appendSystem(`wake score ${score.toFixed(3)} ✓ detected`);
    scoreTimer = setTimeout(() => {
      wakeHero.classList.add("hidden");
      wakeScore.textContent = "";
    }, 2500);
    return;
  }

  if (state === "rejected") {
    wakeScore.textContent = `score ${score.toFixed(2)} ✗`;
    clearScoreSoon();
    return;
  }

  if (state === "error") {
    wakeScore.textContent = "";
    return;
  }

  // listening / hearing / processing — clear any stale score.
  wakeScore.textContent = "";
  wakeHero.classList.remove("hidden");
}

listen("wake_state", (event) => {
  const p = event.payload;
  if (typeof p === "string") {
    setWakeState(p);
  } else {
    setWakeState(p.state, p.score, p.msg);
  }
});

listen("wake_rms", (event) => {
  // RMS of the denoised mic chunk, 0..1, emitted every 100ms while the wake
  // loop's RMS gate is active (the `hearing` state). Drives the perimeter bar
  // heights; nothing is emitted while idle, so the listening state fades to
  // CSS-driven breathing.
  lastRms = event.payload;
  applyBarHeights();
});

listen("qwen_state", (event) => {
  setState(event.payload);
});

listen("user_transcript", (event) => {
  appendUser(event.payload);
});

listen("qwen_transcript", (event) => {
  streamKassDelta(event.payload);
});

listen("qwen_response", (event) => {
  finalizeKass(event.payload);
});

listen("qwen_error", (event) => {
  statusText.textContent = "Error: " + event.payload;
  statusIndicator.className = "status disconnected";
});

callBtn.addEventListener("click", () => {
  invoke("start_call").catch((e) => log("start_call error: " + e));
});

muteBtn.addEventListener("click", async () => {
  const muted = await invoke("toggle_mute");
  muteBtn.textContent = muted ? "Unmute" : "Mute";
});

endBtn.addEventListener("click", async () => {
  await invoke("end_call");
});

log("frontend: listeners registered");
