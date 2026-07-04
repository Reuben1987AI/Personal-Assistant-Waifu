# Architecture: Personal Assistant Waifu

## Overview

A desktop 3D waifu assistant with full dev environment isolation. The UI runs as a native Tauri app with transparent window support, while all risky dependencies (LLM SDKs, STT/TTS libraries, npm packages) run in isolated Docker containers.

## Design Principles

- **Bring your own keys** — no data leaves your machine except to APIs you explicitly configure
- **Dev isolation** — all `npm install`, `cargo build`, and dependency downloads happen inside containers, never on the host
- **Runtime isolation** — STT/TTS processing, skill execution, and LLM calls run in separate containers
- **Tauri = pretty face** — the desktop app handles 3D rendering, window management, and IPC only

---

## Dev Environment: Docker + X11

All development happens inside a Docker container. The GUI is forwarded to the host via X11, using the host GPU for native 3D rendering.

```
┌─────────────────────────────────────────────────────────┐
│ HOST (your real machine)                                │
│                                                         │
│  ┌─────────────┐    ┌──────────────┐    ┌────────────┐ │
│  │ IDE         │    │ X Server     │    │ Docker     │ │
│  │ (Neovim,    │    │ (Wayland/    │    │ daemon     │ │
│  │  VS Code)   │    │  X11)        │    │            │ │
│  └──────┬──────┘    └──────┬───────┘    └─────┬──────┘ │
│         │                  │                  │        │
│         │ edit source      │ receives X11     │ manages│
│         │ via bind mount   │ protocol         │        │
│         │                  │ commands         │        │
└─────────┼──────────────────┼──────────────────┼────────┘
          │                  │                  │
          ▼                  ▼                  ▼
┌─────────────────────────────────────────────────────────┐
│ DEV CONTAINER (untrusted zone)                          │
│                                                         │
│  ┌───────────────────────────────────────────────────┐ │
│  │ /app (bind-mounted source code)                   │ │
│  │                                                   │ │
│  │  Rust toolchain  │  Bun runtime  │  Tauri CLI    │ │
│  │  libwebkit2gtk   │  npm deps     │  cargo deps   │ │
│  │                                                   │ │
│  │  $ tauri dev → creates window via DISPLAY=:0     │ │
│  │  Window renders on HOST X server with HOST GPU   │ │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘
```

### X11 Forwarding Details

- Container connects to host X server via `/tmp/.X11-unix` socket mount
- `DISPLAY` environment variable points to host display
- GPU acceleration works because the host X server handles rendering
- X11 security mitigations: no clipboard sharing, no access to host input devices

### Security Boundaries

| Threat | Mitigation |
|---|---|
| Malicious npm postinstall scripts | Run inside container, can't touch host filesystem |
| Malicious cargo build.rs scripts | Run inside container |
| Compromised dependency at runtime | Container has no access to host files beyond bind mount |
| X11 keylogging | Mitigated via `x11docker` restrictions (no clipboard, no input forwarding) |
| Source code tampering | Bind mount can be set to read-only (`:ro`) if editing from host IDE |

---

## Runtime Architecture

```
┌─────────────────────────────────────────────────────────┐
│ Tauri Desktop App (runs on host)                        │
│                                                         │
│  ┌───────────────────────────────────────────────────┐ │
│  │ WebView (WebKitGTK)                               │ │
│  │                                                   │ │
│  │  Three.js / Babylon.js                            │ │
│  │  └── VRM 3D model + lip-sync + animations        │ │
│  │                                                   │ │
│  │  UI: chat, settings, controls                     │ │
│  └─────────────────────┬─────────────────────────────┘ │
│                        │ IPC (Tauri commands)          │
│  ┌─────────────────────▼─────────────────────────────┐ │
│  │ Rust Backend (src-tauri/)                         │ │
│  │                                                   │ │
│  │  - Window management (transparent, click-through) │ │
│  │  - IPC command handlers                           │ │
│  │  - HTTP client → Docker backend services          │ │
│  │  - Local audio capture (microphone)               │ │
│  └─────────────────────┬─────────────────────────────┘ │
└────────────────────────┼────────────────────────────────┘
                         │ HTTP / WebSocket
                         │ localhost ports
┌────────────────────────▼────────────────────────────────┐
│ Docker Compose (runtime services)                       │
│                                                         │
│  ┌─────────────────┐  ┌─────────────────┐              │
│  │ STT Service     │  │ TTS Service     │              │
│  │ (Whisper/etc)   │  │ (Piper/etc)     │              │
│  │ :8001           │  │ :8002           │              │
│  └─────────────────┘  └─────────────────┘              │
│                                                         │
│  ┌─────────────────┐  ┌─────────────────┐              │
│  │ Skill Runner    │  │ Memory/RAG      │              │
│  │ (sandboxed)     │  │ Service         │              │
│  │ :8003           │  │ :8004           │              │
│  └─────────────────┘  └─────────────────┘              │
└─────────────────────────────────────────────────────────┘
```

### Service Responsibilities

| Service | Purpose | Isolation |
|---|---|---|
| **Tauri App** | 3D rendering, window management, UI, audio capture | Runs on host (trusted binary) |
| **STT Service** | Speech-to-text processing | Docker container |
| **TTS Service** | Text-to-speech generation | Docker container |
| **Skill Runner** | Execute user skills/scripts | Docker container (sandboxed, no network unless granted) |
| **Memory/RAG** | Vector store, context retrieval | Docker container |

### Communication Flow

```
User speaks → Tauri captures audio → POST to STT service
STT returns text → Tauri sends to Rust backend
Rust backend → calls LLM API (bring your own key)
LLM response → Rust backend → POST to TTS service
TTS returns audio → Tauri plays audio + updates 3D model lipsync
```

---

## Project Structure

```
Personal-Assistant-Waifu/
├── .devcontainer/
│   └── dev.Dockerfile          # Dev environment image
├── docker/
│   ├── Dockerfile.dev          # Dev environment (alternative to devcontainer)
│   ├── docker-compose.dev.yml  # Dev services
│   ├── docker-compose.yml      # Runtime services (STT, TTS, skills, memory)
│   ├── wakeword-trainer/
│   │   └── Dockerfile          # Wake word training (PyTorch, isolated)
│   └── services/
│       ├── stt/
│       │   └── Dockerfile
│       ├── tts/
│       │   └── Dockerfile
│       ├── skills/
│       │   └── Dockerfile
│       └── memory/
│           └── Dockerfile
├── src/                        # Tauri frontend (web UI)
│   ├── assets/
│   │   └── waifu.vrm           # 3D model
│   ├── components/
│   │   ├── WaifuViewer.tsx     # Three.js 3D renderer
│   │   ├── ChatPanel.tsx
│   │   └── SettingsPanel.tsx
│   ├── styles/
│   └── main.tsx
├── src-tauri/                  # Tauri Rust backend
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   ├── commands/
│   │   │   ├── audio.rs        # Microphone capture
│   │   │   ├── stt.rs          # Call STT service
│   │   │   ├── tts.rs          # Call TTS service
│   │   │   └── llm.rs          # Call LLM API
│   │   └── window.rs           # Transparent window setup
│   ├── Cargo.toml
│   ├── tauri.conf.json
│   └── capabilities/
│       └── default.json
├── .env.example
├── Makefile                    # Dev workflow commands
└── README.md
```

---

## Dev Workflow

### Prerequisites (host machine)

- Docker
- `x11docker` (or manual X11 socket forwarding)
- Your preferred IDE (runs on host, edits bind-mounted source)

### Start Dev Environment

```bash
# Build dev image
make dev-build

# Start dev container with X11 + GPU
make dev-run

# Inside container:
cd /app
bun install
cargo tauri dev
```

### X11 Forwarding (manual, without x11docker)

```bash
docker run -it --rm \
  -v /tmp/.X11-unix:/tmp/.X11-unix:ro \
  -e DISPLAY=$DISPLAY \
  -v $PWD:/app \
  -w /app \
  --device /dev/dri:/dev/dri \
  --group-add $(getent group render | cut -d: -f3) \
  waifu-dev
```

### With x11docker (recommended)

```bash
x11docker --gpu \
  --clipboard=no \
  --share=$PWD:/app:rw \
  --workdir=/app \
  -- waifu-dev
```

### Runtime Services

```bash
# Start all backend services
docker compose up -d

# Start just STT + TTS
docker compose up -d stt tts
```

### Wake Word Training

```bash
# Build training image (PyTorch, TTS, livekit-wakeword)
docker build -t wakeword-trainer docker/wakeword-trainer/

# Train a custom wake word
make train-wakeword WORD=kassandra

# Output: wakeword-output/kassandra/kassandra.onnx
```

Training is fully isolated — PyTorch, synthetic TTS generation, and all pip dependencies run inside the container. Only the final `.onnx` file is copied to the project.

---

## Window Configuration

The Tauri app uses a transparent, borderless, always-on-top window:

```json
{
  "windows": [{
    "title": "Waifu",
    "transparent": true,
    "decorations": false,
    "alwaysOnTop": true,
    "width": 600,
    "height": 800,
    "resizable": false
  }]
}
```

Click-through is toggled via Rust:
```rust
window.set_ignore_cursor_events(true)  // clicks pass through to desktop
window.set_ignore_cursor_events(false) // clicks interact with waifu
```

---

## Key Decisions Pending

- [ ] VRM model source (pre-made vs generated)
- [ ] STT engine (Whisper, Vosk, cloud API)
- [ ] TTS engine (Piper, Coqui, cloud API)
- [ ] LLM provider (bring your own key)
- [ ] Frontend framework (React, Svelte, Vue)
- [ ] Skill system architecture (repo structure, plugin format)
