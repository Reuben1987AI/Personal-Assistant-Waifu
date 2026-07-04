const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

function log(msg) {
  console.log(msg);
  invoke("console_log", { message: msg }).catch(() => {});
}

const statusIndicator = document.getElementById("status-indicator");
const statusText = document.getElementById("status-text");
const transcript = document.getElementById("transcript");
const triggerBtn = document.getElementById("trigger-btn");
const muteBtn = document.getElementById("mute-btn");
const endBtn = document.getElementById("end-btn");

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
  triggerBtn.classList.toggle("hidden", inCall);
  muteBtn.classList.toggle("hidden", !inCall);
  endBtn.classList.toggle("hidden", !inCall);
}

function appendTranscript(text, isUser) {
  const span = document.createElement("span");
  span.textContent = (isUser ? "You: " : "Kassandra: ") + text + "\n";
  span.style.color = isUser ? "#80b0f0" : "#c0a0f0";
  transcript.appendChild(span);
  transcript.scrollTop = transcript.scrollHeight;
}

let audioContext = null;

triggerBtn.addEventListener("click", async () => {
  log("trigger button clicked");
  await invoke("trigger_wake");
});

muteBtn.addEventListener("click", async () => {
  const muted = await invoke("toggle_mute");
  muteBtn.textContent = muted ? "Unmute" : "Mute";
});

endBtn.addEventListener("click", async () => {
  await invoke("end_call");
});

log("frontend: registering listeners");

listen("qwen_state", (event) => {
  setState(event.payload);
});

listen("qwen_transcript", (event) => {
  appendTranscript(event.payload, true);
});

listen("qwen_response", (event) => {
  appendTranscript(event.payload, false);
});

listen("qwen_error", (event) => {
  statusText.textContent = "Error: " + event.payload;
  statusIndicator.className = "status disconnected";
});

log("frontend: listeners registered");
