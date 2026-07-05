# Voice Call Agent: Kassandra

## Overview

A minimal voice call agent that listens for the wake word "Kassandra", then connects to Qwen Omni's real-time speech-to-speech API for a natural voice conversation. Pure HTML/CSS/JS frontend; Rust backend handles mic capture, wake word detection, WebSocket proxying, and audio playback.

## Design Principles

- **Bring your own keys** — no data leaves your machine except to APIs you explicitly configure
- **Dev isolation** — all `cargo build`s and dependency installs happen inside the dev container, never on the host
- **Tauri = UI + audio + proxy** — the desktop app handles UI, mic capture (cpal), audio playback (cpal), wake word detection, and the Qwen WebSocket proxy. Qwen Omni handles STT + LLM + TTS in a single realtime API, so there are no separate STT/TTS/LLM runtime containers right now (those are Phase 2+, see README.md)

## Dev Environment: Docker + X11

All development happens inside a Docker container. The GUI is forwarded to the host via X11, using the host GPU for native rendering.

```
┌─────────────────────────────────────────────────────────┐
│ HOST (your real machine)                                │
│                                                         │
│  ┌─────────────┐    ┌──────────────┐    ┌────────────┐ │
│  │ IDE         │    │ X Server     │    │ PipeWire   │ │
│  │ (Neovim,    │    │ (X11)        │    │ (audio)    │ │
│  │  VS Code)   │    │              │    │            │ │
│  └──────┬──────┘    └──────┬───────┘    └─────┬──────┘ │
│         │ edit source      │ receives X11     │        │
│         │ via bind mount   │ protocol         │        │
└─────────┼──────────────────┼──────────────────┼────────┘
          │                  │                  │
          ▼                  ▼                  ▼
┌─────────────────────────────────────────────────────────┐
│ DEV CONTAINER (waifu-dev)                               │
│                                                         │
│  ┌───────────────────────────────────────────────────┐ │
│  │ /app (bind-mounted source code)                   │ │
│  │                                                   │ │
│  │  Rust toolchain   │  Bun + @tauri-apps/cli       │ │
│  │  libwebkit2gtk4.1 │  cargo deps (persisted vol)  │ │
│  │  libasound2-dev   │  pipewire-alsa / gstreamer   │ │
│  │                                                   │ │
│  │  $ bunx tauri dev → window via DISPLAY=:0        │ │
│  │  Window renders on HOST X server with HOST GPU   │ │
│  │  Audio → HOST PipeWire via /tmp/pipewire-0       │ │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### Host passthroughs (see Makefile)

- X11 socket `/tmp/.X11-unix` + `DISPLAY` + `XAUTHORITY` → GUI on host X server with host GPU
- PipeWire socket `/run/user/$(id -u)/pipewire-0` + `docker/asound.conf` + `pipewire-alsa` → container ALSA `default` routes through host PipeWire (which holds the audio hardware exclusively). Details in [audio-architecture.md](audio-architecture.md)
- `/dev/dri` + render group → GPU acceleration
- `/dev/snd` + audio group → ALSA device access
- Named volumes `waifu-cargo-registry`, `waifu-cargo-git` → persist cargo cache across container restarts

### Security boundaries

| Threat | Mitigation |
|---|---|
| Malicious npm/cargo postinstall or build.rs scripts | Run inside container, can't touch host filesystem beyond bind mount |
| Compromised dependency at runtime | Container has no access to host files beyond the bind mount |
| X11 keylogging | Scoped via `xhost +local:docker` (no clipboard, limited input forwarding) |

Start with `make dev-build` then `make dev-run` (handles all mounts above).

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│ FRONTEND (src/) — Pure HTML/CSS/JS, UI only                  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ UI                                                     │ │
│  │                                                        │ │
│  │  - Status indicator (idle/listening/calling/speaking)  │ │
│  │  - Live transcript (You / Kassandra lines)             │ │
│  │  - Trigger Wake / Mute / End Call buttons              │ │
│  │  - 3D waifu canvas (Three.js, for later)               │ │
│  └────────────────────────────────────────────────────────┘ │
└────────────────────────┬─────────────────────────────────────┘
                         │ Tauri IPC (events only)
                         │ frontend never sends OR plays audio
┌────────────────────────▼──────────────────────────────────────┐
│ RUST BACKEND (src-tauri/) — all audio + protocol              │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Microphone Capture (cpal) — audio/mic.rs               │ │
│  │                                                        │ │
│  │  - Opens system mic at 16kHz, 16-bit, mono             │ │
│  │  - Emits 100ms (1600-sample) PCM chunks via mpsc       │ │
│  │  - Idle: chunks feed wake word detector                │ │
│  │  - In call: chunks base64-encoded → Qwen WebSocket     │ │
│  └─────────────────────┬──────────────────────────────────┘ │
│                        │                                    │
│  ┌─────────────────────▼──────────────────────────────────┐ │
│  │ LiveKit WakeWord — wakeword/detector.rs                │ │
│  │                                                        │ │
│  │  - Loads models/kassandra.onnx classifier              │ │
│  │  - Mel spectrogram + embedding models compiled in      │ │
│  │  - Scores each chunk, triggers Qwen call when > 0.5    │ │
│  └─────────────────────┬──────────────────────────────────┘ │
│                        │                                    │
│  ┌─────────────────────▼──────────────────────────────────┐ │
│  │ Qwen Omni WebSocket Proxy — qwen/client.rs             │ │
│  │                                                        │ │
│  │  - Opens WSS to Qwen Omni realtime API                 │ │
│  │  - Sends session.update (voice, formats, VAD)          │ │
│  │  - Forwards mic audio (input_audio_buffer.append)      │ │
│  │  - Receives response.audio.delta → pushes to Speaker    │ │
│  │  - Emits Tauri events: qwen_state/transcript/response   │ │
│  └─────────────────────┬──────────────────────────────────┘ │
│                        │                                    │
│  ┌─────────────────────▼──────────────────────────────────┐ │
│  │ Speaker (cpal output) — audio/speaker.rs               │ │
│  │                                                        │ │
│  │  - 24kHz mono i16 output stream                        │ │
│  │  - Reads from a VecDeque fed by response.audio.delta   │ │
│  │  - Plays to ALSA default → PipeWire → host speakers    │ │
│  └────────────────────────────────────────────────────────┘ │
└────────────────────────┬─────────────────────────────────────┘
                         │ WebSocket (wss://)
                         │
┌────────────────────────▼──────────────────────────────────────┐
│ QWEN OMNI REALTIME API (Alibaba Cloud DashScope)             │
│                                                              │
│  Default endpoint (intl):                                    │
│    wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime      │
│      ?model=qwen3.5-omni-flash-realtime                      │
│  Region variants: sg ({workspace}.ap-southeast-1.maas...)    │
│                   cn (dashscope.aliyuncs.com)                │
│                                                              │
│  Input:  16kHz 16-bit PCM (base64)                           │
│  Output: 24kHz 16-bit PCM (base64) + text transcript         │
│  VAD:    Server-side semantic VAD (auto turn detection)      │
└──────────────────────────────────────────────────────────────┘
```

## Why this architecture

- **Frontend never touches the microphone or speakers** — no Web Audio API complexity, no browser permission quirks, no CORS, no WebKitGTK Web Audio breakage
- **All audio I/O in Rust via cpal** — mic capture and speaker playback both run in the backend; the frontend only renders UI + transcripts
- **Wake word runs natively in Rust** — `livekit-wakeword` crate has mel/embedding models compiled in, only the classifier `.onnx` is loaded at runtime
- **100x fewer false positives** vs openWakeWord (LiveKit's own benchmarks)

For audio routing details (ALSA → PipeWire, cpal device config, failed approaches), see [audio-architecture.md](audio-architecture.md).

## Wake Word Detection

**Engine**: livekit-wakeword (Apache 2.0)
**Crate**: `livekit-wakeword = "0.1"` (see `src-tauri/Cargo.toml`)
**Model**: Custom "Kassandra" classifier (`src-tauri/models/kassandra.onnx`)

### How it works (Rust side)

```rust
use livekit_wakeword::WakeWordModel;

// Mel spectrogram + embedding models are compiled into the binary.
// Only the classifier ONNX file is loaded at runtime.
let mut model = WakeWordModel::new(&["models/kassandra.onnx"], 16000)?;

// Feed 100ms PCM chunks (i16, at 16kHz)
let scores = model.predict(&audio_chunk)?;
if scores["kassandra"] > 0.5 {
    println!("Wake word detected!");
    // → Start Qwen Omni call
}
```

The path `models/kassandra.onnx` is relative to the working dir, which is `src-tauri/` when running `bunx tauri dev` (see Makefile). So the onnx file lives at `src-tauri/models/kassandra.onnx`.

### Training the "Kassandra" model (Docker container)

Training runs in an isolated container — PyTorch, TTS models, and `pip install` scripts never touch the host.

**Dockerfile** (`docker/wakeword-trainer/Dockerfile`):
```dockerfile
FROM python:3.11-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential && rm -rf /var/lib/apt/lists/*

RUN pip install --no-cache-dir "livekit-wakeword[train,eval,export]"

RUN livekit-wakeword setup

WORKDIR /workspace
```

**Commands**:
```bash
# Build the training image
docker build -t wakeword-trainer docker/wakeword-trainer/

# Create config
cat > wakeword-configs/kassandra.yaml << 'EOF'
model_name: kassandra
target_phrases:
  - "kassandra"
n_samples: 10000
model:
  model_type: conv_attention
  model_size: small
  steps: 50000
EOF

# Train (mount config dir, output goes to mounted volume)
docker run --rm \
  -v $PWD/wakeword-configs:/workspace/configs \
  -v $PWD/wakeword-output:/workspace/output \
  wakeword-trainer \
  livekit-wakeword run configs/kassandra.yaml

# Output: wakeword-output/kassandra/kassandra.onnx
# Copy to: src-tauri/models/kassandra.onnx
```

**Or with Make**:
```bash
make train-wakeword WORD=kassandra
```

### Why no browser wake word?

LiveKit's wakeword has **no JavaScript/browser SDK** yet. But that's fine — running it in Rust is better:
- Native performance, no WASM overhead
- Mic capture via `cpal` (cross-platform, no browser APIs)
- Simpler frontend (no ONNX Runtime Web, no AudioWorklet)
- Audio never leaves the backend until the call starts

## Qwen Omni WebSocket Protocol

### Connection

```
URL (intl, default): wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model=qwen3.5-omni-flash-realtime
URL (sg):           wss://{WorkspaceId}.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model={model}
URL (cn):           wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model={model}
Auth:               Authorization: Bearer {DASHSCOPE_API_KEY}
```

Region is selected via `QWEN_REGION` (`intl` | `sg` | `cn`). See `qwen/client.rs` for exact URL resolution. Each region requires its own API key.

### Session configuration (first message after connect)

```json
{
  "event_id": "session_001",
  "type": "session.update",
  "session": {
    "modalities": ["text", "audio"],
    "voice": "Tina",
    "input_audio_format": "pcm_16000hz_mono_16bit",
    "output_audio_format": "pcm_24000hz_mono_16bit",
    "instructions": "You are Kassandra, a personal AI assistant. Be warm, witty, and concise.",
    "turn_detection": {
      "type": "semantic_vad",
      "threshold": 0.5,
      "silence_duration_ms": 800
    }
  }
}
```

### Client events (Rust → Qwen)

| Event | Purpose |
|---|---|
| `session.update` | Configure session (sent once) |
| `input_audio_buffer.append` | Send audio chunk: `{"type": "input_audio_buffer.append", "audio": "<base64>"}` |

With server-side semantic VAD enabled, no `commit` or `create_response` is needed — the server auto-detects speech boundaries and triggers responses. (`run_call_manual` does send an explicit `response.create` to force a greeting; `run_call` relies on VAD.)

### Server events (Qwen → Rust)

| Event | Purpose |
|---|---|
| `session.created` | Connection established |
| `response.audio.delta` | Streaming audio chunk — base64 PCM 24kHz in the **`delta`** field (not `audio`) |
| `response.audio.done` | Audio response complete |
| `response.audio_transcript.delta` | Assistant response streaming text |
| `response.audio_transcript.done` | Assistant response full transcript |
| `input_audio_buffer.speech_started` | User started speaking |
| `input_audio_buffer.speech_stopped` | User stopped speaking |
| `conversation.item.input_audio_transcription.completed` | User speech transcribed (not yet forwarded to frontend) |

Handled in `qwen/client.rs` (`run_call`): `response.audio.delta` (→ `Speaker::push_chunk`), `response.audio_transcript.delta` (→ `qwen_transcript` event), `response.audio_transcript.done` (→ `qwen_response` event), `input_audio_buffer.speech_started` (→ `qwen_state: listening`), `response.audio.done` (→ `qwen_state: speaking`), `error` (→ `qwen_error` event).

## Tauri IPC Interface

### Commands (frontend → Rust)

```javascript
// Start a call manually (UI button, or fallback when wakeword model is missing)
await invoke("trigger_wake");

// End an active call
await invoke("end_call");

// Toggle mute during a call (returns new muted state)
const muted = await invoke("toggle_mute");

// Frontend → backend log bridge
await invoke("console_log", { message: "..." });
```

When the wakeword model is loaded, `run_voice_agent` owns the mic and `trigger_wake` just flips `in_call` (the agent loop picks it up). When the model is missing, `trigger_wake` opens the mic and calls `run_call` directly.

### Events (Rust → frontend)

Audio is **not** sent to the frontend — response audio is played in Rust via `audio/speaker.rs` (cpal). The frontend only receives state + transcript events:

```javascript
// Call state changes
listen("qwen_state", (event) => {
  // "idle" | "wake_detected" | "connecting" | "connected" | "speaking" | "listening" | "disconnected"
  updateUI(event.payload);
});

// Assistant response transcript delta (streaming)
listen("qwen_transcript", (event) => {
  appendTranscript(event.payload, /* isUser */ true);
});

// Assistant response final transcript
listen("qwen_response", (event) => {
  appendTranscript(event.payload, /* isUser */ false);
});

// Error
listen("qwen_error", (event) => {
  showError(event.payload);
});
```

Note: `qwen_transcript` and `qwen_response` both carry the **assistant's** response transcript (delta vs done). `main.js` currently renders `qwen_transcript` under the "You:" label — a known labeling mismatch. User speech is not transcribed/emitted yet (`conversation.item.input_audio_transcription.completed` is unhandled).

## Project Structure

```
Personal-Assistant-Waifu/
├── src/
│   ├── index.html                  # Main page
│   ├── styles.css                  # All styles
│   └── main.js                     # UI logic + Tauri event listeners (no audio)
├── src-tauri/
│   ├── src/
│   │   ├── main.rs                 # Tauri entry, commands, run_voice_agent loop
│   │   ├── lib.rs                  # AppState (muted / in_call / wakeword_active)
│   │   ├── audio/
│   │   │   ├── mic.rs              # cpal mic capture, 100ms PCM chunks
│   │   │   └── speaker.rs          # cpal output, 24kHz, base64→i16 queue
│   │   ├── wakeword/
│   │   │   └── detector.rs         # livekit-wakeword wrapper
│   │   └── qwen/
│   │       └── client.rs           # WebSocket proxy (run_call, run_call_manual)
│   ├── models/
│   │   └── kassandra.onnx          # Trained wake word classifier
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── capabilities/
│       └── default.json
├── docker/
│   ├── Dockerfile.dev              # Dev environment (Rust + Bun + Tauri CLI + audio deps)
│   ├── asound.conf                 # ALSA→PipeWire routing (mounted into container)
│   └── wakeword-trainer/
│       └── Dockerfile              # Wake word training container
├── wakeword-configs/
│   └── kassandra.yaml              # Wake word training config
├── wakeword-output/                # .gitignored, trained models land here
│   └── kassandra/
│       └── kassandra.onnx
├── .env.example
├── Makefile
└── README.md
```

## Audio Flow

### Idle state (wake word listening)

```
System Mic (cpal) → 100ms chunks → livekit-wakeword → [no match, keep listening]
```

Everything runs in Rust. Frontend shows "Say 'Kassandra' to start".

### Wake word detected → Call starts

```
1. livekit-wakeword detects "Kassandra" (score ≥ 0.5)
2. Rust emits "qwen_state: wake_detected" → frontend updates UI
3. Rust opens WebSocket to Qwen Omni
4. Rust sends session.update with config
5. Rust emits "qwen_state: connected" → frontend updates UI
6. Rust continues streaming mic audio to Qwen via WebSocket (same mic stream, no interruption)
```

### During call

```
System Mic (cpal) → Rust → Qwen WebSocket (16kHz PCM, base64)
                                    ↓
                          Qwen processes (semantic VAD)
                                    ↓
Qwen generates speech → response.audio.delta → Rust
                                    ↓
                          base64 decode → i16 → Speaker queue
                                    ↓
                          cpal output stream (24kHz) → ALSA → PipeWire → speakers
```

### Barge-in (interrupting Kassandra)

```
User speaks while Kassandra is talking
  → Qwen semantic_vad detects interruption
  → Qwen stops generating audio
  → response.audio.done received
  → New turn begins with user's question
```

Note: barge-in / echo suppression is **not** implemented yet — the mic keeps capturing during playback, so speaker audio can feed back into the mic and trigger spurious new responses. This is the current open issue; see `.session-status.md`.

### Call ends

```
User clicks End Call → invoke("end_call") → in_call=false → run_call loop breaks
  → Rust closes WebSocket
  → Rust emits "qwen_state: idle"
  → livekit-wakeword resumes listening
```

## Dependencies

### Frontend (zero packages)

Pure HTML/CSS/JS. No browser audio APIs used — the frontend only handles UI and Tauri event listeners. All audio I/O is in Rust via cpal.

### Rust backend

| Crate | Purpose |
|---|---|
| `livekit-wakeword` | Wake word detection (mel + embedding compiled in) |
| `cpal` | Cross-platform mic capture + speaker playback |
| `tokio` | Async runtime |
| `tokio-tungstenite` | WebSocket client (`native-tls` feature for WSS) |
| `serde` / `serde_json` | JSON serialization |
| `base64` | Audio encoding/decoding |
| `tauri` | Desktop framework |
| `dotenvy` | `.env` loading |
| `ort` | ONNX runtime (transitive, for wakeword) |

## Environment Variables

```env
# .env (loaded by dotenvy at startup)
DASHSCOPE_API_KEY=sk-xxx            # region-specific (intl/sg/cn each need their own)
QWEN_WORKSPACE_ID=your-workspace-id # only needed for sg region
QWEN_REGION=intl                    # intl | sg | cn
QWEN_MODEL=qwen3.5-omni-flash-realtime
QWEN_VOICE=Tina
QWEN_INSTRUCTIONS=You are Kassandra, a personal AI assistant. Be warm, witty, and concise.
HF_TOKEN=hf-xxx                     # for wakeword training
```

## Setup Steps

1. **Get API key**: https://www.alibabacloud.com/help/en/model-studio/get-api-key
   - intl/sg region recommended for international access
   - Note your WorkspaceId from the console (required for sg region)

2. **Train wake word model** (inside Docker — nothing installed on host):
   ```bash
   docker build -t wakeword-trainer docker/wakeword-trainer/
   make train-wakeword WORD=kassandra
   # Output: wakeword-output/kassandra/kassandra.onnx
   cp wakeword-output/kassandra/kassandra.onnx src-tauri/models/
   ```

3. **Build dev image & run**:
   ```bash
   make dev-build
   make dev-run
   # Click "Trigger Wake" in the Kassandra window, or say "Kassandra"
   ```

## Model Selection

| Model | Use case | Cost tier |
|---|---|---|
| `qwen3.5-omni-plus-realtime` | Best quality, web search, tool calling | Higher |
| `qwen3.5-omni-flash-realtime` | Fast, cost-effective, no tool calling | Lower (default) |

Start with `flash` for development, switch to `plus` when ready.

## Voice Options

Available voices (55 total). Recommended for a waifu assistant:

| Voice | Style | Languages |
|---|---|---|
| `Tina` | Bright, female | Multilingual |
| `Serena` | Calm, female | Multilingual |
| `Cherry` | Warm, female | Multilingual — only `qwen3-omni-flash-realtime` (older model) |

`qwen3.5-omni-flash-realtime` uses `Tina` (the current default). `Cherry` is rejected with `Voice 'Cherry' is not supported` on qwen3.5 models.

## Known Limitations

- WebSocket sessions max out at 120 minutes (auto-disconnect)
- `livekit-wakeword` Rust crate is new (v0.1) — API may change
- Linux mic/speaker via cpal requires `libasound2-dev` + PipeWire socket passthrough (see [audio-architecture.md](audio-architecture.md))
- X11 forwarding in dev container: Linux compositors may vary in transparent window support
- No Android support yet (Tauri mobile is separate)
- Barge-in / echo suppression not implemented — speaker audio can feed back into the mic (see `.session-status.md`)

## Phase 2+ (out of scope for now)

- 3D waifu rendering with lip-sync (Three.js + VRM)
- Memory/RAG for context persistence
- Skill system for actions
- Call mode vs voice message mode
- Context editor
- Personality system
- Separate STT/TTS/LLM runtime containers (if moving off Qwen Omni)
