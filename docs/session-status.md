# Session Resume: Personal Assistant Waifu (Kassandra)

## Last Updated
2026-07-05 (wake word enabled, native ONNX Runtime integrated, continuous rolling predict)

## This Session (2026-07-05)

### Docker isolation — DONE
- `node_modules/` removed from host. Named Docker volume `waifu-node-modules` shadows `/app/client/node_modules` so `bun install` inside the container never touches the host FS.
- `Makefile` `dev-run` + `dev-shell` both mount `-v waifu-node-modules:/app/client/node_modules`.
- `tauri.conf.json` guard changed from `[ -d node_modules ]` → `[ -x node_modules/.bin/vite ]` since the volume mount creates an empty dir.
- `vite.config.ts` CWD guard: throws if `process.cwd()` doesn't start with `/app/`, blocking `bun run dev`/`build` on host.
- `client/scripts/preinstall.cjs`: blocks `npm`/`pnpm`/`yarn install` on host (checks `/.dockerenv`). Bun skips lifecycle scripts by default, so the vite guard + AGENTS.md rules handle bun.
- `AGENTS.md`: Docker section with hard blocks — no `bun`/`npm`/`cargo`/`tauri` commands on host, no `node_modules/` access from host, always use `make dev-shell`.

### CJK font — DONE
- App now bundles `@fontsource/noto-sans-sc` web font (Chinese-simplified + Latin subsets, 1.1MB woff2). Imported in `client/src/main.tsx`, set as primary `font-family` in `styles.css`. Docker container has zero system CJK fonts — this is the sole source of CJK glyphs.

## What's Working
- App launches, renders UI (status bar, chat, wake-activity circle at top of chat, Mute / End Call buttons)
- Tauri commands fire (`end_call`, `toggle_mute`, `console_log`)
- WebSocket connects to Qwen Omni Realtime (intl default; sg/cn supported)
- Mic capture: cpal 16kHz mono, 100ms (1600-sample) chunks, `std::mem::forget(stream)` keeps stream for process lifetime
- Speaker playback: cpal 24kHz output, `response.audio.delta` → base64 → VecDeque → speaker
- Audio routing: container ALSA `default` → host PipeWire (`/tmp/pipewire-0`)
- Frontend transcript + state updates via Tauri events
- Full audio loop both ways verified (speak → Qwen responds → speakers)
- WebRTC AEC3 echo cancellation (`src-tauri/src/audio/aec.rs`) — hoisted to app startup, shared between wake loop and Qwen call. Full-duplex, barge-in works. AEC NS confirmed not to hurt the wake model (live mic peaks higher than clean sample).

## Wake word — ENABLED (native ONNX Runtime, ~17 ms/predict)

The rolling-buffer continuous-predict design is now fully implemented and enabled by default. The native ONNX Runtime fork of livekit-wakeword (`src-tauri/lib/wakeword-native/`) achieves **~17 ms/predict** — a **94× speedup** over ort-tract (~1,600 ms). This makes the rolling-buffer design viable: each 100ms mic chunk spawns a tokio worker thread for predict(), so the mic read loop never stalls. When the score exceeds the threshold, the wake loop fires and starts a Qwen call.

**Architecture:**
- No energy gate (RMS energy gate removed — model's own non-speech baseline ~0.003 is sufficient)
- No utterance framing, no VAD, no lead-silence crutch
- Predict dispatched on `tokio::spawn`, result collected via `oneshot::channel`
- RMS computed and emitted every 100ms for continuous UI bar animation
- Wake-state events: only `listening` and `fired` (no more `hearing`/`rejected`)

**Build integration:**
- `src-tauri/onnx_compat.c` — C23 glibc compat shim for static ONNX Runtime linking on Debian Bookworm (copied from `bench/`)
- `src-tauri/build.rs` — compiles onnx_compat.c via `cc` crate
- `src-tauri/Cargo.toml` — `cc = "1"` build-dependency added

**WakeConfig** (in `src-tauri/src/main.rs`):
| Field | Env var | Default |
|---|---|---|
| `wake_threshold` | `KASSANDRA_WAKE_THRESHOLD` | `0.45` |
| `post_fire_lockout` | `KASSANDRA_POST_FIRE_LOCKOUT_MS` | `2000ms` |
| `wake_buffer_samples` | `KASSANDRA_WAKE_BUFFER_SAMPLES` | `32000` (2s @ 16kHz) |

Removed: `rms_threshold`, `predict_cadence`, `silence_confirm_chunks` — no longer needed without energy gate / utterance framing.

**End-call flow fix:**
- `end_call` now takes `app: tauri::AppHandle` and emits `qwen_state: "disconnected"` immediately — the frontend updates to show the wake hero before `run_call` cleans up the WebSocket.
- `run_call` cleanup sends a `__CLOSE__` sentinel through the writer channel, aborts the reader task (drops `out_tx_reader`, unblocking the writer), then awaits the writer to close the WebSocket cleanly. Returns in milliseconds instead of blocking the wake loop.

Full history, diagnostics, and training guidance in [`docs/wake-word.md`](wake-word.md).

## UI (done)
- `status bar / scrollable chat / controls` layout, 520×800 decorated resizable window, solid `#1a1a2e` bg, no transparency/blur/border-radius.
- Chat bubbles (user right blue, Kassandra left `#2a2a44`), 15px font, scrollable `#messages`.
- System bubbles (italic, centered) for wake events — ignored by Qwen.
- User transcript: `conversation.item.input_audio_transcription.completed` handled in `client.rs` (both `run_call` and `run_call_manual`) → emits `user_transcript`; `main.js` renders user bubble. Requires `input_audio_transcription.model = paraformer-realtime-v2` in session.update.
- `qwen_transcript` (delta) streams into a growing Kassandra bubble; `qwen_response` (done) finalizes it. No more duplicate text or "You:" mislabel.

## AEC status (done, live-verified)
WebRTC AEC3 in `src-tauri/src/audio/aec.rs`. Hoisted to `run_voice_agent` startup, shared between wake loop pre-filter and Qwen call echo cancellation. While idle, no render is pushed — only NS + HPF active, acting as the wake pre-filter (path A0). Live test this session: AEC-processed mic is fully recognizable by the wake model (live peak 0.53 vs clean sample 0.38) — NS does not hurt. If echo persists during calls: tune `stream_delay_ms`, enable AGC, or fall back to Path 2 (PipeWire `module-echo-cancel`).

## Manual call start — COEXISTS with wake word

The Call button still works alongside automatic wake-word detection. Both paths share the same `in_call` → `run_call` flow in the voice-agent loop. Wake-word detection is enabled by default (`KASSANDRA_WAKE_ENABLED=true`); set to `false` to use manual-only mode.

**What changed (this session):**
- `KASSANDRA_WAKE_ENABLED` default changed from `false` to `true`.
- The wake loop runs continuously, predicting on every 100ms mic chunk on a tokio worker thread.
- `end_call` immediately emits `qwen_state: "disconnected"` so the UI flips back to the wake hero while the WebSocket tears down.
- `run_call` (in `client.rs`) cleanup no longer blocks: sends `__CLOSE__` sentinel, aborts reader task, awaits writer for clean WebSocket close in milliseconds.
- Client-side: `@types/node` added to devDependencies; `types: ["node"]` added to `tsconfig.node.json` (pre-existing issue — vite.config.ts needed `process` global).

## Lessons Learned (still useful — don't retry these)
1. `transparent: true` in tauri.conf.json causes click-through on Linux/WebKitGTK — set `false`.
2. `withGlobalTauri: true` required in tauri.conf.json `app` section to inject `window.__TAURI__`.
3. Tauri v2 custom commands do NOT need explicit permissions — `core:default` is enough.
4. `tokio-tungstenite` needs `native-tls` feature for WSS.
5. Qwen-Omni-Realtime is NOT in US Virginia — intl/sg/cn only. intl: `wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime`. Each region needs its own API key.
6. Audio format: `pcm_16000hz_mono_16bit` in, `pcm_24000hz_mono_16bit` out.
7. `turn_detection.type = semantic_vad` for qwen3.5-omni models.
8. `cpal::Stream` is not `Send` — use `std::mem::forget(stream)` to keep mic + speaker alive for process lifetime.
9. Docker + inotify = broken — `bunx tauri dev` doesn't detect host FS changes in volumes. Run `cargo build` in the container manually before `make dev-run`, or the old binary runs.
10. Cargo registry must be persisted via Docker volumes (`waifu-cargo-registry`, `waifu-cargo-git`).
11. X11 auth requires `xhost +local:docker` + mounting XAUTHORITY. (Now auto-run by `make dev-run`.)
12. JSON doesn't support `//` comments — don't add to tauri.conf.json.
13. WebKitGTK Web Audio API is broken in this Docker setup — play audio in Rust via cpal `speaker.rs`, not in the frontend.
14. `response.audio.delta` carries audio in the `delta` field, not `audio`.
15. Voice `Cherry` is for `qwen3-omni-flash-realtime`; qwen3.5-omni-flash uses `Tina`.
16. PipeWire holds audio hardware exclusively — route container ALSA through the PipeWire socket, not direct `hw:0,0`/`plughw`.
17. `webrtc-audio-processing` `bundled` build needs `meson >= 1.1`, `ninja-build`, `pkg-config`, `libclang-dev`, `python3-pip`. Debian bookworm's apt `meson` 1.0.1 breaks; `pip3 install meson --break-system-packages`. `libclang-dev` required by `bindgen`.

**Wake-word-specific lessons moved to [`docs/wake-word.md`](wake-word.md)** (livekit-wakeword predict-needs-2s, position-sensitivity, ort-tract constraint, silero VAD unusable, rolling-buffer design, AEC NS doesn't hurt, model undertrained, webrtc-vad Send wrapper, silero download URL 404). Read that doc before touching anything wake-related.

## What's NOT Done
1. Remove debug `eprintln!` logging once audio + turn flow is solid (`client.rs`, `main.rs`, `mic.rs`, `speaker.rs`).
2. Fix placeholder icons — `bundle.active = false` for dev, need real icons for production.
3. Wake word: retrain the model (`medium` / 150k steps / 30k samples — see [`docs/wake-word.md`](wake-word.md) Path C) for a wider scoring window + higher peak. With native ONNX Runtime this is purely an accuracy refinement.
4. 3D avatar / lip-sync — Phase 2.
5. Memory / RAG, skills, personality — Phase 2.

## Key Files
- `src-tauri/src/main.rs` — Tauri entry, commands (`start_call`, `end_call` with immediate `disconnected` emit, `toggle_mute`, `console_log`), `run_voice_agent` loop (wake enabled by default — continuous rolling predict, threaded via `tokio::spawn`), `WakeConfig` (3 fields), `env_bool()`.
- `src-tauri/lib/wakeword-native/` — Forked native-ONNX-Runtime wake word crate (94× faster than ort-tract). Identical API to livekit-wakeword, identical inference results.
- `src-tauri/build.rs` — compiles `onnx_compat.c` (C23 glibc compat shim) via `cc` crate for static ONNX Runtime linking on Debian Bookworm.
- `src-tauri/onnx_compat.c` — C23 glibc compat stubs (`__isoc23_strtol` etc.) required for `ort` with `download-binaries` on Bookworm.
- `src-tauri/src/wakeword/detector.rs` — wake word init, uses `wakeword-native` crate (path dep).
- `src-tauri/src/qwen/client.rs` — Qwen WebSocket proxy. `run_call` cleanup sends `__CLOSE__` sentinel, aborts reader task, awaits writer for fast teardown.
- `client/src/stores/state/domain/wake.ts` — Wake state store (still includes `hearing`/`rejected` types — no longer emitted but valid).
- `client/src/stores/workflows/callWorkflow.ts` — `applyWakeState`, `endCall`, `startCall` workflows.
- `client/src/components/WakeHero.tsx` — Wake circle UI. Bars driven by continuous `wake_rms` (no more `hearing`/`listening` branch), score display on `fired` only.

## Environment Variables
```
DASHSCOPE_API_KEY=sk-xxx (region-specific)
QWEN_WORKSPACE_ID=your-workspace-id (only for sg)
QWEN_REGION=intl (intl | sg | cn)
QWEN_MODEL=qwen3.5-omni-flash-realtime
QWEN_VOICE=Tina
QWEN_INSTRUCTIONS=You are Kassandra, a personal AI assistant. Be warm, witty, and concise.
HF_TOKEN=hf-xxx (for wakeword training)

# Wake-loop configurability — documented in docs/wake-word.md (wake loop enabled by default):
KASSANDRA_WAKE_ENABLED=true    # set false for manual-only mode
KASSANDRA_WAKE_THRESHOLD=0.45
KASSANDRA_POST_FIRE_LOCKOUT_MS=2000
KASSANDRA_WAKE_BUFFER_SAMPLES=32000
KASSANDRA_DUMP_PCM=/app/src-tauri/wake_dump.pcm   # optional PCM dump for offline test_wakeword analysis
```

## How to Run
```bash
make dev-run
# App launches with wake-word detection enabled (KASSANDRA_WAKE_ENABLED=true).
# Say "Kassandra" to start a call automatically, or click "Call" for manual start.
# Click "End Call" to stop; the wake hero returns and detection resumes.
# Set KASSANDRA_WAKE_ENABLED=false for manual-only mode.
```

## Docs
- `docs/wake-word.md` — wake-word subsystem: everything tried, learned, diagnostics, next steps. Read before touching anything wake-related.
- `docs/architecture.md` — full architecture + protocol + IPC
- `docs/audio-architecture.md` — audio routing deep-dive (read before touching `src-tauri/src/audio/`)
- `docs/ideas.md` — backlog (avatar, skills, companion-site references)
- `docs/README.md` — doc conventions
