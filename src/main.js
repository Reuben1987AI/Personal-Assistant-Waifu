const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

function log(msg) {
  console.log(msg);
  invoke("console_log", { message: msg }).catch(() => {});
}

const statusIndicator = document.getElementById("status-indicator");
const statusText = document.getElementById("status-text");
const messages = document.getElementById("messages");
const muteBtn = document.getElementById("mute-btn");
const endBtn = document.getElementById("end-btn");

const wakeHero = document.getElementById("wake-hero");
const wakeCircle = document.getElementById("wake-circle");
const wakeBars = document.getElementById("wake-bars");
const wakeLabel = document.getElementById("wake-label");
const wakeScore = document.getElementById("wake-score");

let currentKassBubble = null;

const STATE_LABELS = {
  idle: 'Say "Kassandra" to start',
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
  muteBtn.classList.toggle("hidden", !inCall);
  endBtn.classList.toggle("hidden", !inCall);
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
   per-bar CSS variable --h (0..1) updated from the wake_rms event, with a
   small deterministic offset per bar so they don't all move identically.
   State transitions are driven by wake_state events. */

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
  // Only react to RMS while in hearing state — processing/listening ignore.
  if (wakeHero.dataset.state !== "hearing") return;
  lastRms = event.payload;
  for (let i = 0; i < N_BARS; i++) {
    const h = Math.min(1, lastRms * barOffsets[i] * 1.4 + 0.08);
    barEls[i].style.setProperty("--h", h.toFixed(3));
  }
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

muteBtn.addEventListener("click", async () => {
  const muted = await invoke("toggle_mute");
  muteBtn.textContent = muted ? "Unmute" : "Mute";
});

endBtn.addEventListener("click", async () => {
  await invoke("end_call");
});

log("frontend: listeners registered");
