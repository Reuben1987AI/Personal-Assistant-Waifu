# Session Resume: Personal Assistant Waifu (Kassandra)

## Last Updated
2026-07-04 (node_modules isolated, CJK web font, host install blocks)

## This Session (2026-07-04 late)

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

## Wake word — DEFERRED
The rolling-buffer wake loop (`src-tauri/src/main.rs`) compiles and fires on a real spoken "Kassandra" (score 0.53), but live latency is ~16s with one core pinned — unusable as a real-time trigger. Root cause: livekit-wakeword forces the `ort-tract` backend (~1.6s/predict) while the model's scoring window is only ~0.5s, so continuous rolling predict can't reliably catch the word. **Full history, diagnostics, and the path back to automatic wake are in [`docs/wake-word.md`](wake-word.md).** Until the inference-speed problem is solved (replace tract with real onnxruntime, or retrain for a wider scoring window), we **shipped a manual call start** (UI Call button → Qwen call, no wake word). See "Manual call start — DONE" below.

## UI (done)
- `status bar / scrollable chat / controls` layout, 520×800 decorated resizable window, solid `#1a1a2e` bg, no transparency/blur/border-radius.
- Chat bubbles (user right blue, Kassandra left `#2a2a44`), 15px font, scrollable `#messages`.
- System bubbles (italic, centered) for wake events — ignored by Qwen.
- User transcript: `conversation.item.input_audio_transcription.completed` handled in `client.rs` (both `run_call` and `run_call_manual`) → emits `user_transcript`; `main.js` renders user bubble. Requires `input_audio_transcription.model = paraformer-realtime-v2` in session.update.
- `qwen_transcript` (delta) streams into a growing Kassandra bubble; `qwen_response` (done) finalizes it. No more duplicate text or "You:" mislabel.

## AEC status (done, live-verified)
WebRTC AEC3 in `src-tauri/src/audio/aec.rs`. Hoisted to `run_voice_agent` startup, shared between wake loop pre-filter and Qwen call echo cancellation. While idle, no render is pushed — only NS + HPF active, acting as the wake pre-filter (path A0). Live test this session: AEC-processed mic is fully recognizable by the wake model (live peak 0.53 vs clean sample 0.38) — NS does not hurt. If echo persists during calls: tune `stream_delay_ms`, enable AGC, or fall back to Path 2 (PipeWire `module-echo-cancel`).

## Manual call start — DONE
Replaced automatic wake-word triggering with a UI Call button that starts a Qwen call directly. The wake loop stays in the code (compiles, useful for future wake work — see [`wake-word.md`](wake-word.md)) but is gated off at runtime by `KASSANDRA_WAKE_ENABLED` (default `false`), so it isn't burning a core or emitting wake events.

**Chosen approach — Option 1 (reuse `run_call`):** added a `start_call` Tauri command in `src-tauri/src/main.rs` that flips `in_call=true` and emits `qwen_state: connecting`. The voice-agent loop's existing top-of-loop `in_call` check then runs `qwen::run_call` with the working mic + AEC wiring and an empty wake chunk. This reuses the verified mic-send path. `run_call_manual` (Option 2) was **not** used — it's a half-finished stub with no mic/AEC wiring, a duplicated `session.update`, and an idle loop that just sleeps; wiring it up would duplicate `run_call`'s mic-send loop. It's left in `client.rs` as dormant code.

**What changed:**
- `src-tauri/src/main.rs`: `start_call` command (emits `connecting`, fails if already in call); `env_bool()` helper; wake loop gated behind `KASSANDRA_WAKE_ENABLED` (default false) — detector is skipped, `wakeword_active` stays false, and the loop's wake-detection section is short-circuited (mic is still drained to keep the cpal callback from blocking on the bounded mpsc channel). The `in_call` → `run_call` path is shared between manual and wake modes.
- `src/index.html`: status text `Click Call to start`; new `#call-btn` in `#controls` (visible when idle, hidden once a call starts — replaces Mute/End Call which still hide until in-call).
- `src/main.js`: `callBtn` ref + click → `invoke("start_call")`; `STATE_LABELS.idle` → `Click Call to start`; `setState` toggles `callBtn` inverse to Mute/End Call, and hides `#wake-hero` while in-call (the circle is an idle ambient element — it shouldn't show during a call). The backend only emits `wake_state: listening` in manual mode, so the hero stays breathing with no scores when visible.
- Wake-hero hearing/fired/rejected JS + CSS: left dormant for future wake work.

**To re-enable wake word:** set `KASSANDRA_WAKE_ENABLED=true` (and provide `models/kassandra.onnx`). The wake loop resumes; the Call button still works alongside it (it just sets `in_call`, the loop handles the rest).

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
3. Wake word: replace tract with real onnxruntime OR retrain for a wider scoring window — see [`docs/wake-word.md`](wake-word.md) "Next steps."
4. 3D avatar / lip-sync — Phase 2.
5. Memory / RAG, skills, personality — Phase 2.

## Key Files
- `src-tauri/src/main.rs` — Tauri entry, commands (`start_call`, `end_call`, `toggle_mute`, `console_log`), `run_voice_agent` loop (wake detection gated behind `KASSANDRA_WAKE_ENABLED`, default false — see wake-word.md), `WakeConfig::from_env()`, `env_bool()`, `wake_state`/`wake_rms` emitters, optional `KASSANDRA_DUMP_PCM` writer.
- `src-tauri/src/lib.rs` — `AppState` (muted / in_call / wakeword_active AtomicBools).
- `src-tauri/src/audio/mic.rs` — cpal mic capture, 100ms chunks, `std::mem::forget`.
- `src-tauri/src/audio/speaker.rs` — cpal output, 24kHz, VecDeque, `std::mem::forget`.
- `src-tauri/src/audio/aec.rs` — WebRTC AEC3 echo cancellation (bundled C++). Hoisted to `run_voice_agent` startup. `clear_render()` for post-call cleanup. PipeWire `module-echo-cancel` alt documented in module docs.
- `src-tauri/src/audio/mod.rs` — exports `Aec`, `MicStream`, `Speaker`.
- `src-tauri/src/wakeword/detector.rs` — livekit-wakeword init (loads `models/kassandra.onnx`).
- `src-tauri/examples/test_wakeword.rs` — wake-model diagnostic binary (slide/live/isolated modes).
- `src-tauri/src/qwen/client.rs` — Qwen WebSocket proxy. `run_call` (mic + aec + wake_chunk; used by both wake-fired and manual `start_call`) and `run_call_manual` (dormant — manual call start reuses `run_call` instead; left for future use). Handles `user_transcript` / `qwen_transcript` / `qwen_response`. `run_call` calls `aec.clear_render()` after the call.
- `src/main.js` — Frontend UI, Tauri event listeners (`wake_state`, `wake_rms`, `qwen_state`, `*_transcript`, `qwen_response`, `qwen_error`), wake-circle state machine.
- `src/index.html`, `src/styles.css` — UI (status bar, `#wake-hero` + `#messages` inside `#chat`, Mute/End Call).
- `src-tauri/tauri.conf.json` — `withGlobalTauri: true`, `transparent: false`, `decorations: true`, `resizable: true`, 520×800.
- `src-tauri/capabilities/default.json` — `core:default`.
- `wakeword-configs/kassandra.yaml` — wake-word training config.
- `docker/Dockerfile.dev` — Rust stable, Bun, @tauri-apps/cli, cargo-watch, audio deps, AEC build deps.
- `docker/asound.conf` — ALSA → PipeWire routing.
- `docker/wakeword-trainer/Dockerfile` — livekit-wakeword Python trainer + espeak-ng.
- `Makefile` — dev-build, dev-run (auto-runs `xhost +local:docker`), dev-shell, train-wakeword; waifu-node-modules volume isolation.
- `AGENTS.md` — Frontend architecture rules + Docker isolation hard blocks.
- `client/vite.config.ts` — Vite config + Docker CWD guard (hard-blocks host dev/build).
- `client/scripts/preinstall.cjs` — Docker guard for npm/pnpm/yarn install (host block).
- `client/src/main.tsx` — React mount, Tauri event adapter, CJK web font import.
- `client/src/styles.css` — Global styles, font-family stack with bundled Noto Sans SC.

## Environment Variables
```
DASHSCOPE_API_KEY=sk-xxx (region-specific)
QWEN_WORKSPACE_ID=your-workspace-id (only for sg)
QWEN_REGION=intl (intl | sg | cn)
QWEN_MODEL=qwen3.5-omni-flash-realtime
QWEN_VOICE=Tina
QWEN_INSTRUCTIONS=You are Kassandra, a personal AI assistant. Be warm, witty, and concise.
HF_TOKEN=hf-xxx (for wakeword training)

# Wake-loop configurability — documented in docs/wake-word.md (wake loop deferred, gated off by default):
KASSANDRA_WAKE_ENABLED=false   # set true to re-enable the wake loop (needs models/kassandra.onnx)
KASSANDRA_RMS_THRESHOLD=250
KASSANDRA_PREDICT_CADENCE_MS=200
KASSANDRA_WAKE_THRESHOLD=0.45
KASSANDRA_SILENCE_CONFIRM_CHUNKS=8
KASSANDRA_POST_FIRE_LOCKOUT_MS=2000
KASSANDRA_WAKE_BUFFER_SAMPLES=32000
KASSANDRA_DUMP_PCM=/app/src-tauri/wake_dump.pcm   # optional PCM dump for offline test_wakeword analysis
```

## How to Run
```bash
make dev-run
# App launches in manual call mode (wake loop gated off by KASSANDRA_WAKE_ENABLED=false).
# Click "Call" to start a Qwen call; click "End Call" to stop.
# Set KASSANDRA_WAKE_ENABLED=true to re-enable the wake loop (needs models/kassandra.onnx).
# Watch terminal for qwen events once a call starts: "qwen: speech started", "qwen transcript: ...", "qwen: response complete (N audio chunks)"
```

## Docs
- `docs/wake-word.md` — wake-word subsystem: everything tried, learned, diagnostics, next steps. Read before touching anything wake-related.
- `docs/architecture.md` — full architecture + protocol + IPC
- `docs/audio-architecture.md` — audio routing deep-dive (read before touching `src-tauri/src/audio/`)
- `docs/ideas.md` — backlog (avatar, skills, companion-site references)
- `docs/README.md` — doc conventions
