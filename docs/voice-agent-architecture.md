# Voice Call Agent: Kassandra

## Overview

A minimal voice call agent that listens for the wake word "Kassandra", then connects to Qwen Omni's real-time speech-to-speech API for a natural voice conversation. Pure HTML/CSS/JS frontend, Rust backend handles mic capture, wake word detection, and WebSocket proxying.

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│ FRONTEND (src/) — Pure HTML/CSS/JS                          │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ UI                                                     │ │
│  │                                                        │ │
│  │  - Status indicator (idle/listening/calling)           │ │
│  │  - Wake word prompt                                    │ │
│  │  - Live transcript (optional)                          │ │
│  │  - 3D waifu canvas (Three.js, for later)               │ │
│  └────────────────────────────────────────────────────────┘ │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Audio Playback                                         │ │
│  │                                                        │ │
│  │  Response audio ← Tauri event ← Rust backend           │ │
│  │  (24kHz 16-bit PCM, base64) → Web Audio API → Speaker  │ │
│  └────────────────────────────────────────────────────────┘ │
└────────────────────────┬─────────────────────────────────────┘
                         │ Tauri IPC
                         │ (events only — frontend never sends audio)
┌────────────────────────▼──────────────────────────────────────┐
│ RUST BACKEND (src-tauri/) — Everything audio                 │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐ │
│  │ Microphone Capture (cpal)                              │ │
│  │                                                        │ │
│  │  - Opens system mic at 16kHz, 16-bit, mono             │ │
│  │  - Streams PCM chunks to wake word detector            │ │
│  │  - During call, also streams to Qwen WebSocket         │ │
│  └─────────────────────┬──────────────────────────────────┘ │
│                        │                                    │
│  ┌─────────────────────▼──────────────────────────────────┐ │
│  │ LiveKit WakeWord (livekit-wakeword crate)              │ │
│  │                                                        │ │
│  │  - Loads kassandra.onnx classifier                     │ │
│  │  - Mel spectrogram + embedding models compiled in      │ │
│  │  - Scores each ~2s audio window                        │ │
│  │  - On detection → triggers Qwen call                   │ │
│  └─────────────────────┬──────────────────────────────────┘ │
│                        │                                    │
│  ┌─────────────────────▼──────────────────────────────────┐ │
│  │ Qwen Omni WebSocket Proxy                              │ │
│  │                                                        │ │
│  │  - Opens WebSocket to Qwen Omni realtime API           │ │
│  │  - Forwards mic audio chunks (base64)                  │ │
│  │  - Receives response audio + transcript events         │ │
│  │  - Emits Tauri events to frontend                      │ │
│  │  - Handles session config (voice, instructions, VAD)   │ │
│  └────────────────────────────────────────────────────────┘ │
└────────────────────────┬─────────────────────────────────────┘
                         │ WebSocket (wss://)
                         │
┌────────────────────────▼──────────────────────────────────────┐
│ QWEN OMNI REALTIME API (Alibaba Cloud)                       │
│                                                              │
│  Endpoint: wss://{WorkspaceId}.ap-southeast-1.maas.          │
│    aliyuncs.com/api-ws/v1/realtime?model=                    │
│    qwen3.5-omni-plus-realtime                                │
│                                                              │
│  Input:  16kHz 16-bit PCM (base64)                           │
│  Output: 24kHz 16-bit PCM (base64) + text transcript         │
│  VAD:    Server-side semantic VAD (auto turn detection)      │
└──────────────────────────────────────────────────────────────┘
```

## Why this architecture

- **Frontend never touches the microphone** — no Web Audio API complexity, no browser permission quirks, no CORS
- **Wake word runs natively in Rust** — `livekit-wakeword` crate has mel/embedding models compiled in, only the classifier `.onnx` is loaded at runtime
- **100x fewer false positives** vs openWakeWord (LiveKit's own benchmarks)
- **Frontend is purely presentational** — UI + audio playback only

## Wake Word Detection

**Engine**: livekit-wakeword (Apache 2.0)
**Crate**: `livekit-wakeword = "0.1"` on crates.io
**Model**: Custom "Kassandra" classifier (.onnx file)

### How it works (Rust side)

```rust
use livekit_wakeword::WakeWordModel;

// Mel spectrogram + embedding models are compiled into the binary.
// Only the classifier ONNX file is loaded at runtime.
let mut model = WakeWordModel::new(&["kassandra.onnx"], 16000)?;

// Feed ~2s PCM audio chunks (i16, at 16kHz)
let scores = model.predict(&audio_chunk)?;
if scores["kassandra"] > 0.5 {
    println!("Wake word detected!");
    // → Start Qwen Omni call
}
```

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
URL: wss://{WorkspaceId}.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model=qwen3.5-omni-plus-realtime
Auth: Authorization: Bearer {DASHSCOPE_API_KEY}
```

### Session configuration (first message after connect)

```json
{
  "event_id": "event_abc123",
  "type": "session.update",
  "session": {
    "modalities": ["text", "audio"],
    "voice": "Cherry",
    "input_audio_format": "pcm",
    "output_audio_format": "pcm",
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

With server-side VAD enabled, no `commit` or `create_response` needed — the server auto-detects speech boundaries.

### Server events (Qwen → Rust → frontend)

| Event | Purpose |
|---|---|
| `session.created` | Connection established |
| `response.audio.delta` | Streaming audio chunk (base64 PCM 24kHz) |
| `response.audio.done` | Audio response complete |
| `response.audio_transcript.delta` | Streaming text |
| `response.audio_transcript.done` | Full transcript |
| `input_audio_buffer.speech_started` | User started speaking |
| `input_audio_buffer.speech_stopped` | User stopped speaking |
| `conversation.item.input_audio_transcription.completed` | User speech transcribed |

## Tauri IPC Interface

### Commands (frontend → Rust)

```javascript
// End an active call (optional — user can just stop talking)
await invoke("end_call");

// Toggle mute during a call
await invoke("toggle_mute");
```

That's it. The frontend doesn't start calls or send audio — wake word detection and call initiation happen entirely in Rust.

### Events (Rust → frontend)

```javascript
// Audio chunk from Qwen (play immediately)
listen("qwen_audio", (event) => {
  const audioBase64 = event.payload;  // 24kHz 16-bit PCM, base64
  playAudioChunk(audioBase64);
});

// Transcript of user speech
listen("qwen_transcript", (event) => {
  const text = event.payload;
  showUserMessage(text);
});

// Transcript of assistant response
listen("qwen_response", (event) => {
  const text = event.payload;
  showAssistantMessage(text);
});

// Call state changes
listen("qwen_state", (event) => {
  const state = event.payload;  // "idle" | "wake_detected" | "connecting" | "connected" | "speaking" | "listening" | "disconnected"
  updateUI(state);
});

// Error
listen("qwen_error", (event) => {
  const error = event.payload;
  showError(error);
});
```

## Project Structure

```
Personal-Assistant-Waifu/
├── src/
│   ├── index.html                  # Main page
│   ├── styles.css                  # All styles
│   └── main.js                     # App entry, UI logic, audio playback
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   ├── audio/
│   │   │   ├── mic.rs              # Microphone capture (cpal)
│   │   │   └── player.rs           # (optional, Rust-side playback)
│   │   ├── wakeword/
│   │   │   ├── detector.rs         # livekit-wakeword wrapper
│   │   │   └── models/
│   │   │       └── kassandra.onnx  # Trained classifier
│   │   └── qwen/
│   │       └── client.rs           # WebSocket proxy to Qwen Omni
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── capabilities/
│       └── default.json
├── docker/
│   ├── Dockerfile.dev              # Dev environment (from architecture.md)
│   └── wakeword-trainer/
│       └── Dockerfile              # Wake word training container
├── wakeword-configs/
│   └── kassandra.yaml              # Wake word training config
├── wakeword-output/                # .gitignored, trained models land here
│   └── kassandra/
│       └── kassandra.onnx
├── .env.example
└── Makefile
```

## Audio Flow

### Idle state (wake word listening)

```
System Mic (cpal) → livekit-wakeword → [no match, keep listening]
```

Everything runs in Rust. Frontend shows "Say 'Kassandra' to start".

### Wake word detected → Call starts

```
1. livekit-wakeword detects "Kassandra" (score ≥ 0.5)
2. Rust emits "qwen_state: wake_detected" → frontend updates UI
3. Rust opens WebSocket to Qwen Omni
4. Rust sends session.update with config
5. Rust emits "qwen_state: connected" → frontend updates UI
6. Rust continues streaming mic audio to Qwen via WebSocket (no interruption)
```

### During call

```
System Mic (cpal) → Rust → Qwen WebSocket (16kHz PCM, base64)
                                    ↓
                          Qwen processes (VAD)
                                    ↓
Qwen generates speech → response.audio.delta → Rust → Tauri event → Frontend
                                                                    ↓
                                                          Web Audio API → Speaker
```

### Barge-in (interrupting Kassandra)

```
User speaks while Kassandra is talking
  → Qwen semantic_vad detects interruption
  → Qwen stops generating audio
  → response.audio.done received
  → Rust stops emitting audio events
  → New turn begins with user's question
```

### Call ends

```
User stops speaking → Qwen VAD timeout → no more responses
  → Rust detects idle → emits "qwen_state: idle"
  → Frontend shows "Say 'Kassandra' to start"
  → livekit-wakeword resumes listening
```

Or user explicitly ends via UI → `invoke("end_call")` → Rust closes WebSocket → resumes wake word listening.

## Dependencies

### Frontend (zero packages)

Pure HTML/CSS/JS. Only browser APIs used: `Web Audio API` for playback.

### Rust backend

| Crate | Purpose |
|---|---|
| `livekit-wakeword` | Wake word detection (mel + embedding compiled in) |
| `cpal` | Cross-platform microphone capture |
| `tokio` | Async runtime |
| `tokio-tungstenite` | WebSocket client |
| `serde` / `serde_json` | JSON serialization |
| `base64` | Audio encoding/decoding |
| `tauri` | Desktop framework |

## Environment Variables

```env
# .env (loaded by Tauri at build time, or set on host)
DASHSCOPE_API_KEY=sk-xxx
QWEN_WORKSPACE_ID=your-workspace-id
QWEN_MODEL=qwen3.5-omni-flash-realtime
QWEN_VOICE=Cherry
QWEN_INSTRUCTIONS=You are Kassandra, a personal AI assistant. Be warm, witty, and concise.
```

## Setup Steps

1. **Get API key**: https://www.alibabacloud.com/help/en/model-studio/get-api-key
   - Singapore region recommended for international access
   - Note your WorkspaceId from the console

2. **Train wake word model** (inside Docker — nothing installed on host):
   ```bash
   docker build -t wakeword-trainer docker/wakeword-trainer/
   make train-wakeword WORD=kassandra
   # Output: wakeword-output/kassandra/kassandra.onnx
   cp wakeword-output/kassandra/kassandra.onnx src-tauri/models/
   ```

3. **Install Tauri dev dependencies** (inside dev container):
   ```bash
   rustup default stable
   cargo install tauri-cli --version "^2.0.0" --locked

   apt-get install -y libwebkit2gtk-4.1-dev build-essential \
     curl wget libssl-dev libgtk-3-dev libayatana-appindicator3-dev \
     librsvg2-dev patchelf libasound2-dev  # libasound2-dev for cpal
   ```

4. **Run**:
   ```bash
   cargo tauri dev
   ```

## Model Selection

| Model | Use case | Cost tier |
|---|---|---|
| `qwen3.5-omni-plus-realtime` | Best quality, web search, tool calling | Higher |
| `qwen3.5-omni-flash-realtime` | Fast, cost-effective, no tool calling | Lower |

Start with `qwen3.5-omni-flash-realtime` for development, switch to `plus` when ready.

## Voice Options

Available voices (55 total). Recommended for a waifu assistant:

| Voice | Style | Languages |
|---|---|---|
| `Cherry` | Warm, female | Multilingual |
| `Tina` | Bright, female | Multilingual |
| `Serena` | Calm, female | Multilingual |

## Known Limitations

- WebSocket sessions max out at 120 minutes (auto-disconnect)
- `livekit-wakeword` Rust crate is new (v0.1) — API may change
- Linux mic capture via cpal requires `libasound2-dev`
- X11 forwarding in dev container: Linux compositors may vary in transparent window support
- No Android support yet (Tauri mobile is separate)

## Phase 2+ (out of scope for now)

- 3D waifu rendering with lip-sync (Three.js + VRM)
- Memory/RAG for context persistence
- Skill system for actions
- Call mode vs voice message mode
- Context editor
- Personality system
