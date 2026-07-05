# Audio Architecture

Read this before touching anything in `src-tauri/src/audio/`, `docker/asound.conf`, `Makefile` audio mounts, or `src/main.js` audio playback.

## Architecture (what works)

```
Mic (DMIC hw:0,6) → cpal capture → 100ms chunks → [AEC3 capture] → Qwen WebSocket
                                                       ↑
                           speaker output → [AEC3 render reference (24kHz→16kHz resampled)]
                                                       ↓
Speaker (PipeWire) ← cpal output ← PCM queue ← base64 decode ← Qwen WebSocket
```

**All audio I/O is in Rust via cpal.** The frontend does NOT play audio. WebKitGTK's Web Audio API is broken in this Docker setup (see below). Audio response chunks are decoded from base64 in `client.rs`, pushed to a `Speaker` queue, and played by a cpal output stream that reads from the queue in its callback.

### Echo cancellation (AEC3)
WebRTC AEC3 runs in-app (`src-tauri/src/audio/aec.rs`) via the `webrtc-audio-processing` crate (bundled C++ build). The speaker output is tapped as a reference ("render") signal, resampled 24kHz→16kHz, and fed into the AEC. The mic capture ("capture") signal is processed against the reference to remove echo. This enables full-duplex audio — the user can barge in and interrupt Kassandra mid-sentence. Also includes noise suppression + high-pass filter. See `aec.rs` module docs for the Path 2 alternative (PipeWire `module-echo-cancel`) if deploying on a controlled host.

### Audio routing

Container ALSA `default` → host PipeWire socket (`/tmp/pipewire-0`). This is the only path that works because the host runs PipeWire which holds the audio hardware exclusively.

- `docker/asound.conf` — ALSA config that routes `default` through PipeWire
- `Makefile` mounts: `/run/user/$(id -u)/pipewire-0:/tmp/pipewire-0:ro`
- Docker image requires `pipewire-alsa` package (in `docker/Dockerfile.dev`)

### Data flow

1. **Mic** (`audio/mic.rs`): cpal captures 16kHz mono i16 from ALSA `default`. Callback accumulates samples, emits 1600-sample (100ms) chunks via `mpsc::channel`. Stream kept alive with `std::mem::forget(stream)` (cpal::Stream is not Send).
2. **AEC** (`audio/aec.rs`): mic chunks → `Aec::process_capture()` → WebRTC AEC3 removes echo using speaker output as reference → cleaned 16kHz mono i16. Speaker chunks are fed via `Aec::push_render()` and resampled 24kHz→16kHz internally. 10ms (160-sample) frames processed per AEC call.
3. **Qwen** (`qwen/client.rs`): cleaned mic chunks → base64 → `input_audio_buffer.append`. Server VAD detects speech, responds with `response.audio.delta` events containing base64 PCM in the **`delta`** field (not `audio`). Speaker output is also fed to AEC render reference.
4. **Speaker** (`audio/speaker.rs`): `response.audio.delta` → base64 decode → i16 samples → `Speaker::push_chunk()` (also feeds AEC render). cpal output stream (24kHz mono) reads from queue in callback, plays to ALSA `default` → PipeWire → speakers.

### Key parameters

| Parameter | Value | Source |
|-----------|-------|--------|
| Input sample rate | 16000 Hz | `livekit_wakeword::SAMPLE_RATE` |
| Output sample rate | 24000 Hz | Qwen output format |
| Chunk duration | 100ms (1600 samples in, ~320 samples out) | `mic.rs` `CHUNK_DURATION_MS` |
| Input format | `pcm_16000hz_mono_16bit` | Qwen session config |
| Output format | `pcm_24000hz_mono_16bit` | Qwen session config |
| VAD type | `semantic_vad` | Required for qwen3.5 models |
| Voice | `Tina` | Default for qwen3.5-omni-flash-realtime |
| Region endpoint | `wss://{workspace}.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime` | Singapore |

## What didn't work (don't retry these)

### 1. Direct ALSA hardware access (`--device /dev/snd` only)

Without PipeWire socket passthrough, container ALSA `default` hits hardware directly. Problem: **PipeWire holds speakers exclusively** (`Device or resource busy`). Mic (DMIC) worked via `plughw:0,6` but speakers (`hw:0,0`) were locked.

**Don't:** route `asound.conf` to `plughw:0,0` or `hw:0,6` directly.
**Do:** route through PipeWire socket.

### 2. WebKitGTK Web Audio API (frontend playback)

Tried: emit audio chunks to frontend via Tauri events, play with `AudioContext` + `createBufferSource`. AudioContext reached `running` state, chunks decoded, `source.start()` called, but **no sound**. WebKitGTK routes Web Audio through GStreamer, and even with `gstreamer1.0-alsa` installed, no audio reached the speakers. The queue also drained suspiciously fast (all sources ended in one tick), suggesting a null/dummy sink.

**Don't:** use `AudioContext` in `main.js` for audio playback.
**Do:** play audio in Rust via `audio/speaker.rs`.

### 3. GStreamer alsasink env vars

Tried: `GST_AUDIOSINK=alsasink`, `GST_DEBUG=alsasink:5`. No GStreamer debug output appeared — WebKitGTK's Web Audio didn't route through GStreamer alsasink at all. These env vars were removed from the Makefile.

**Don't:** set `GST_AUDIOSINK` or `GST_DEBUG` in the Makefile.

### 4. ALSA `asym` config (split playback/capture to different devices)

Tried: `asound.conf` with `type asym` routing playback to `plughw:0,0` and capture to `plughw:0,6`. Playback failed silently — PipeWire held `hw:0,0`. Capture worked but the whole approach was replaced by PipeWire routing.

**Don't:** use `type asym` with direct hardware devices.

### 5. Voice `Cherry`

`Cherry` is for `Qwen3-Omni-Flash-Realtime` (older model). `qwen3.5-omni-flash-realtime` uses `Tina`. Error: `Voice 'Cherry' is not supported.`

### 6. `server_vad` on qwen3.5

Docs recommend `semantic_vad` for qwen3.5-omni models. `server_vad` technically worked but `semantic_vad` is correct.

### 7. Audio field name `audio`

`response.audio.delta` events contain audio in the **`delta`** field, not `audio`. Code originally checked `event["audio"]` → always None. Fixed to check `event["delta"]`.

## Common pitfalls

- **`cpal::Stream` is not Send** — can't store in async structs. Use `std::mem::forget(stream)` to keep alive for process lifetime. Both mic and speaker do this.
- **Two cpal streams on same device** — `run_voice_agent` opens mic for wakeword detection; `trigger_wake` must not open a second mic. Solution: `wakeword_active` flag in `AppState`; if true, `trigger_wake` sets `in_call=true` and `run_voice_agent` handles the Qwen call.
- **Docker + inotify broken** — `bunx tauri dev` doesn't detect host filesystem changes in Docker volumes. After editing Rust code, run `cargo build` in the container manually before `make dev-run`, or the old binary runs.
- **No `asound.conf` in container** — without it, ALSA `default` routes to `hw:0,0` (headset jack, usually empty) → silence. The mounted `docker/asound.conf` is required.

## Verification

Quick test that audio works: run `make dev-run`, click "Trigger Wake", speak. You should see:
- `speaker device: default` at startup
- `qwen: speech started` when you speak
- `qwen transcript: ...` with Kassandra's response
- `qwen: response complete (N audio chunks)`
- Audio plays through speakers

If no sound: check `docker/asound.conf` mounts PipeWire, check `/run/user/$(id -u)/pipewire-0` exists on host, check `pipewire-alsa` in Docker image.
