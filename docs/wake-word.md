# Wake Word Detection — Kassandra

Single source of truth for the wake-word subsystem. Everything tried, learned, and what's next. The session-status file no longer covers wake-word internals — it points here.

## Status

**Path B proven viable.** The native ONNX Runtime fork of livekit-wakeword (`src-tauri/lib/wakeword-native/`) achieves **~17 ms predict** (mean, 50 iters, 2s audio window) vs ort-tract's ~1,600 ms — a **94× speedup**. This makes the rolling-buffer continuous-predict design viable: at 17 ms/predict against a ~0.5s scoring window, the word slides through ~29 predict opportunities, ensuring reliable detection with sub-100ms wake-to-fire latency.

The fork compiles statically against the prebuilt ONNX Runtime `.a` from `ort-sys`, worked around a Debian Bookworm glibc 2.36 incompatibility (`__isoc23_strtol` symbols missing — see lessons), and produces identical model scores (0.38 on kas1.pcm). Integration into the main app is pending.

## Goal

Detect the spoken word "Kassandra" from the live mic stream and automatically start a Qwen Omni Realtime call. Must be: responsive (≤2s from end of word to call start), low-CPU (not pinning a core), and low-false-fire (no triggers on ambient noise / TV / keyboard).

## What's in the code now (rolling-buffer design)

`run_voice_agent` in `src-tauri/src/main.rs`. Compiles, fires, but too slow to ship.

Pipeline (per 100ms / 1600-sample mic chunk):
1. AEC denoise (NS + HPF; AEC3 idle until a Qwen call pushes render) → denoised chunk.
2. Append to a `Vec<i16>` rolling buffer; drain oldest past `WAKE_BUFFER_SAMPLES` (default 32000 = 2s @ 16kHz).
3. Compute chunk RMS. ≥ `RMS_THRESHOLD` (default 250, i16) ⇒ "active" → emit `wake_state: hearing`.
4. While active AND outside post-fire lockout AND `predict_cadence` elapsed: `detector.predict(&rolling_buffer)`. score > `WAKE_THRESHOLD` (default 0.45) ⇒ fire.
5. On fire: emit `wake_state: fired` {score}, snapshot the rolling buffer as Qwen's first input chunk, clear the buffer, set `in_call=true`, arm `POST_FIRE_LOCKOUT_MS` (default 2000ms).
6. Below-threshold chunks accumulate `silence_run`; after `SILENCE_CONFIRM_CHUNKS` (default 8 = 800ms) go inactive. Emits `rejected` {last_score} if we never fired, else `listening`.

Configurability — every constant is an env var, parsed in `WakeConfig::from_env()`:
- `KASSANDRA_RMS_THRESHOLD` (250.0, f32) — denoised-chunk RMS floor to flip on the gate.
- `KASSANDRA_PREDICT_CADENCE_MS` (200, u64) — max predict frequency while active.
- `KASSANDRA_WAKE_THRESHOLD` (0.45, f32) — score above which we fire.
- `KASSANDRA_SILENCE_CONFIRM_CHUNKS` (8, u32) — consecutive sub-threshold 100ms chunks ⇒ inactive.
- `KASSANDRA_POST_FIRE_LOCKOUT_MS` (2000, u64) — post-fire wake-ignore window.
- `KASSANDRA_WAKE_BUFFER_SAMPLES` (32000, usize) — rolling window length.
- `KASSANDRA_DUMP_PCM` (path, optional) — append every denoised wake-loop chunk to a raw s16le mono 16kHz PCM file for offline `test_wakeword` analysis.

Frontend animation: `wake_rms` (0..1, denoised chunk energy) emitted every 100ms while active drives the perimeter bar heights in `hearing`. In `listening` no `wake_rms` is emitted; `applyBarHeights()` uses a static low baseline and the CSS `wake-breathe` keyframe animates the circle.

## Approaches tried (history)

### 1. RMS energy gate + utterance framing (first attempt)
Energy gate detected speech, buffered it, on end-of-utterance predicted once on `[800ms lead silence] + [last 1.2s speech]` padded to 2s. The lead-silence crutch existed because the model is position-sensitive (see lesson below). Worked on clean audio but the energy gate cycled on transients; no VAD meant keyboard/desk-thump false triggers.

### 2. Silero VAD (FAILED — deleted)
Tried silero VAD ONNX models to gate entry into "hearing" with a learned speech classifier instead of raw energy. **Unusable.** livekit-wakeword pulls `ort` with the `alternative-backend` feature, which forces everything through `ort-tract` (pure-Rust inference). Tract can't translate silero's `If` nodes (control flow for sample-rate branching). Tried 3 model variants (opset 16, opset 15, openvino_16k_ifless) — all failed in tract for different reasons. Silero VAD is not usable in any project that depends on livekit-wakeword. (See lessons 20–21.)

### 3. WebRTC C VAD (worked, then deleted)
`webrtc-vad` crate (VeryAggressive mode) gated entry into `hearing`. Required `ENTRY_VOICE_CHUNKS = 2` consecutive 100ms chunks with ≥1 voice sub-frame to enter; exited on `SILENCE_CONFIRM_CHUNKS = 8` (800ms) voice-less. Predicted once per utterance on the framed buffer (same lead-silence crutch as #1). Worked but added complexity without solving the root cause (undertrained + position-sensitive model). `Vad` struct held a raw `*mut Fvad` (not `Send`), needed `unsafe impl Send for Vad` to cross the tokio boundary. **Deleted** when we moved to the rolling-buffer design — the energy gate + AEC denoise makes VAD unnecessary as an outer filter, and VAD wasn't fixing the real problem.

### 4. Rolling-buffer continuous predict (current — implemented, deferred)
livekit-wakeword's intended production architecture. Energy-gated rolling 2s buffer, predict at a configurable cadence while active, fire on score > threshold. No VAD, no utterance framing, no lead-silence crutch — the word slides through every position in the 2s window so position-sensitivity is moot. **Architecturally correct, but blocked by tract inference speed** (see "Core problem").

## Live diagnostic results (the data that killed the rolling design for now)

### `kas1.pcm` slide test (clean recorded "Kassandra")
Ran `cargo run --example test_wakeword -- kas1.pcm slide` — slides a 2s window in 100ms steps across the sample (padded with silence both sides):
- Silence baseline: `0.003`
- Peak as word slides through window: **`0.38`** at offset 2.0s (word at window-end with ~2s lead silence)
- 120× separation from silence across a ~0.5s band of positions
- **Scoring window is only ~0.5s wide** — the model scores >0.3 only when the word occupies a specific ~0.5s position in the 2s buffer

### Live mic test (the user's actual voice)
With `KASSANDRA_DUMP_PCM` enabled, the rolling buffer fires on a real spoken "Kassandra":
- Scores climb over ~5 predicts (#6→#10): 0.018 → 0.025 → 0.046 → 0.289 → **0.533** → fire
- **AEC noise suppression does NOT hurt the model** — live mic peaks at 0.53, *higher* than the clean `kas1.pcm` peak of 0.38. The audio chain is healthy.
- Latency: **~16s** from speaking to fire (10 predicts × ~1.6s/predict)
- CPU: one core pinned during the climb

### The climbing-score signature
The climbing #6→#10 is the tell. With a real-time rolling buffer, the word should whoosh through the 0.5s scoring window in 0.5s — one predict would catch it, not five. The climb means the buffer is advancing in ~1.6s jerks, not real-time: while `predict()` blocks the wake loop for ~1.6s, no mic chunks are read, so when predict returns the buffer jumps forward by ~1.6s of audio. The word crawls through the scoring position over multiple predicts instead of sliding through smoothly. The blocking accidentally makes the design *more* reliable than a non-blocking version would be — see "Core problem."

## Core problem: predict latency vs scoring window

**The math (ort-tract):**
- tract ONNX predict: **~1,600 ms** per 2s-window call (mel + embedding + classifier)
- model scoring window: **~0.5s** (position band where score > threshold)
- therefore: predict interval (1,600 ms) > scoring window (500 ms)

**The math (native ONNX Runtime — Path B):**
- native predict: **~17 ms** per 2s-window call (verified via Docker benchmark, 50 iters)
- model scoring window: **~0.5s**
- predict / window ratio: **0.034** — comfortably within the ≪ requirement
- ~29 predict opportunities while the word slides through the scoring window

A non-blocking/threaded rolling buffer (the "obvious" fix for the 16s lag) would make this *worse*, not better: the buffer would slide in real-time, the word would pass through the 0.5s scoring window in 0.5s, and a predict would land in that window only ~31% of the time (0.5 / 1.6). The current blocking design accidentally slows the slide so the word dwells in the scoring window across multiple predicts — that's why it works at all, but with 16s latency and a pinned core. **This entire problem disappears with native ONNX Runtime.**

The rolling-buffer continuous-predict design is **architecturally correct only when predict ≪ scoring window**. With tract at 1.6s/predict and a 0.5s window, that condition is violated. **With native ONNX Runtime at 17 ms/predict, the condition is satisfied by a factor of ~29.**

### Native ONNX Runtime benchmark (Docker, 2026-07-05)

Benchmark binary: `bench/` crate, Docker image buildable via `make bench-wakeword`.

Hardware: containerized x86-64 on Debian Bookworm (glibc 2.36), no GPU.
Model: `src-tauri/models/kassandra.onnx`, test audio: `src-tauri/kas1.pcm` (clean recorded "Kassandra").

| Metric | ort-tract | Native ONNX Runtime | Speedup |
|--------|-----------|---------------------|---------|
| Model load | — | 26.5 ms | — |
| predict (mean) | ~1,600 ms | **17.0 ms** | **94×** |
| predict (p95) | — | 18.8 ms | — |
| predict (min) | — | 15.4 ms | — |
| predict (max) | — | 23.8 ms | — |
| Score (kas1.pcm) | 0.38 | 0.38 | identical |

Scores are identical, confirming the native backend produces the same inference results.

## What we learned (hard-won facts — don't retry)

1. **livekit-wakeword 0.1 `predict()` needs ~2s of audio.** Shorter windows silently return 0.0 for every classifier. Predicting on a 100ms chunk always misses.
2. **The kassandra.onnx classifier is position-sensitive.** Trained on TTS positives with ~800ms of leading silence before the word inside the 2s window. Without lead silence, isolated audio scores 0.008; with it, 0.66. The rolling-buffer design sidesteps this in principle (the word slides through every position), but only works if predict is fast enough to catch the scoring position.
3. **livekit-wakeword pulls `ort` with the `alternative-backend` feature**, which disables ONNX Runtime linking entirely and forces everything through `ort-tract` (pure-Rust inference). Tract is ~1,600 ms/predict; native onnxruntime is **17 ms/predict (94× faster, measured)**. The `alternative-backend` feature is enabled via a target-conditional dependency on all platforms except `aarch64-pc-windows-msvc` (`livekit-wakeword` Cargo.toml lines 55–61). There is no user-facing feature flag to disable it — the crate forces `ort-tract` unconditionally.
4. **Forking livekit-wakeword to use native onnxruntime requires only 3 small changes** (done in `src-tauri/lib/wakeword-native/`): remove `alternative-backend` and `ort-tract` from Cargo.toml, remove `ensure_tract_backend()` and `cfg(use_tract)` gating from `src/lib.rs`, neuter `build.rs` (stop setting `cfg(use_tract)`). The `WakeWordModel`, `predict()`, melspectrogram, and embedding modules use generic `ort` APIs and work unchanged with either backend. ~30 lines changed total.
5. **The prebuilt ONNX Runtime static library (from `ort-sys` `download-binaries` `none` feature set) references C23 symbols (`__isoc23_strtol`, `__isoc23_strtoll`, `__isoc23_strtoull`) that are missing from Debian Bookworm's glibc 2.36.** These are glibc compat wrappers emitted by GCC ≥12 when source files are compiled with `-std=c2x` or `_GNU_SOURCE`. Neither `lld` nor GNU `ld.bfd` can resolve them on Bookworm. The workaround is a 15-line C compat shim (`bench/onnx_compat.c`) that provides trivial forwarding implementations compiled via `cc` crate. This must be added to the build whenever `ort` with `download-binaries` is used on Debian Bookworm.
6. **The `ort-sys` `download-binaries` `none` feature set tarball only includes `libonnxruntime.a` (static), not `.so`.** The `load-dynamic` approach cannot work with the `none` binaries — they lack the shared library. Static linking is the only option with the prebuilt CPU binaries. Additionally, `load-dynamic` + `download-binaries` is a non-functional combination: `load-dynamic` enables `disable-linking` which causes the `ort-sys` build script to return early before downloading anything.
7. **Silero VAD is not usable** in any project that depends on livekit-wakeword (tract can't translate its `If` nodes). Tried 3 model variants — all failed. livekit's Python `livekit-plugins-silero` uses real onnxruntime and isn't portable to Rust; no equivalent Rust VAD crate exists in livekit/rust-sdks. (Note: with native onnxruntime via the fork, Silero VAD would likely work, but the rolling-buffer energy-gate design makes VAD unnecessary.)
8. **The intended production design for livekit-wakeword is rolling-buffer continuous predict.** The mel+embedding pipeline is itself the "is this speech" filter — non-speech scores ~0.005. Framing crutches (lead silence, end-of-utterance detection, VAD gating) were workarounds for an undertrained, position-sensitive model. The rolling design eliminates them all — *assuming a fast classifier*.
9. **AEC noise suppression (NS Moderate) does not hurt the model.** Live mic peaks at 0.53 vs clean sample 0.38 — NS-processed mic is fully recognizable. Don't disable NS chasing accuracy; it's not the problem.
10. **The model is undertrained.** Peaks at 0.38 on clean TTS-like audio, 0.53 on live mic. Configured `wakeword-configs/kassandra.yaml` calls for `medium` / 150k steps / 30k samples; the current model was trained with less. Wider training → wider scoring window + higher peak. With native ONNX Runtime this is an accuracy refinement, not a blocking issue.
11. **`webrtc-vad`'s `Vad` struct holds a raw `*mut Fvad` (not `Send`)** — needed `unsafe impl Send for Vad` to cross the tokio boundary. Module deleted; kept here as a record.
12. **curl on GitHub `/raw/refs/heads/master/...` URLs returns 404 for some paths** — silero VAD models moved from `files/` to `src/silero_vad/data/`. Always verify `file <downloaded>` is `data` not `HTML document` before trusting a model download.

## Key files

- `src-tauri/src/main.rs` — `run_voice_agent` wake loop (rolling-buffer + energy-gate + continuous predict + post-fire lockout), `WakeConfig::from_env()`, `wake_state`/`wake_rms` emitters, optional `KASSANDRA_DUMP_PCM` writer.
- `src-tauri/src/wakeword/detector.rs` — wake word init (loads `models/kassandra.onnx`). Uses `wakeword-native` crate (path dep) instead of `livekit-wakeword`.
- `src-tauri/lib/wakeword-native/` — **Forked native-ONNX-Runtime wake word crate.** Diff from upstream `livekit-wakeword` 0.1.3: removed `ort-tract` + `alternative-backend`, uses `ort` with `download-binaries`. Identical API surface.
- `bench/` — Standalone wake word benchmark crate. `make bench-wakeword` builds and runs in Docker. Measures predict time, model load time, and score accuracy. Contains `onnx_compat.c` (C23 glibc compat shim required for static linking on Debian Bookworm).
- `src-tauri/examples/test_wakeword.rs` — wake-model diagnostic binary. Modes: `slide` (rolling-window sweep), `live` (energy-gated utterance framing simulation), `isolated` (single padded window). Run on a dump: `cargo run --example test_wakeword -- wake_dump.pcm slide`.
- `src-tauri/kas1.pcm` — raw s16le mono 16kHz "Kassandra" sample (gitignored). Decoded via `ffmpeg -i "kas 1.flac" -ar 16000 -ac 1 -f s16le`. Regression fixture.
- `src-tauri/models/kassandra.onnx` — the trained wake-word classifier (gitignored, 164KB).
- `wakeword-configs/kassandra.yaml` — wake-word training config.
- `docker/Dockerfile.bench` — Docker image for the native ONNX Runtime wake word benchmark. Downloads ONNX Runtime via `ort-sys` `download-binaries`, builds with C23 compat shim.
- `docker/Dockerfile.dev` — dev environment Docker image (includes Tauri deps).
- `docker/wakeword-trainer/Dockerfile` — livekit-wakeword Python trainer + espeak-ng.
- `src-tauri/src/qwen/client.rs` — `run_call` (wake-fired, takes mic + aec + wake_chunk) and `run_call_manual` (no mic/aec — currently unused, candidate hook for manual call start).
- `src/main.js`, `src/styles.css` — wake-circle UI (`wake_state`/`wake_rms` listeners, `applyBarHeights()`, `#wake-hero`).

## Next steps (ranked)

### Path A — manual call start (NOW, unblocks using the app)
Ship a UI button that starts a Qwen call without the wake word. The wake loop stays in the code but is sidetracked/disabled at runtime. `run_call_manual` already exists in `client.rs` (no mic/aec wiring — would need mic added) OR add a `start_call` Tauri command that sets `in_call=true` and lets the wake loop's top-of-loop `in_call` check run `run_call` with an empty wake chunk (reuses existing mic-audio-send logic). Goal: usable app today while wake word is solved properly. This is the next session's task.

### Path B — replace tract with real onnxruntime ✅ PROVEN (the real latency fix)

**Status: Fork built, benchmarked, 94× speedup confirmed.** The forked `wakeword-native` crate at `src-tauri/lib/wakeword-native/` is a drop-in replacement for `livekit-wakeword` with identical API and identical inference results. The main `Cargo.toml` already points to the path dependency.

**Remaining integration work:**
- Add `cc` build-dependency and `onnx_compat.c` to the main `src-tauri` crate (the C23 glibc compat shim is required for static linking on Debian Bookworm).
- Set `KASSANDRA_PREDICT_CADENCE_MS` to ~50 (was 200) — at 17 ms/predict, the rolling buffer can run a predict on every 100ms mic chunk without blocking.
- Thread `predict()` to avoid blocking the mic read loop (spawn on tokio, collect score on join). At 17 ms it's less critical than at 1,600 ms, but the loop shouldn't stall.
- Eventually switch `KASSANDRA_WAKE_ENABLED` default to `true`.

### Path C — retrain the model (accuracy, not latency)
`medium` / 150k steps / 30k samples per `wakeword-configs/kassandra.yaml`. Wider scoring window + higher peak. With native ONNX Runtime this is purely an accuracy refinement — the latency constraint is solved. Needs `HF_TOKEN` + compute. Lower priority than Path B integration.

### Path D — utterance-gated predict on the rolling buffer (OBSOLETE)
This was a pragmatic fallback for the 1,600 ms predict world. With native ONNX Runtime at 17 ms/predict, continuous rolling-buffer predict is the correct architecture. Path D is documented for historical reference only.

## Tuning guidance (for whoever revisits wake word)

- If false fires on transients: bump `KASSANDRA_RMS_THRESHOLD` or `KASSANDRA_WAKE_THRESHOLD` (env vars, no rebuild).
- If missed wake words: lower `KASSANDRA_WAKE_THRESHOLD` to ~0.30 (clean audio peaks at 0.38, live at 0.53), or lower `KASSANDRA_RMS_THRESHOLD` to ~150.
- If double-fires: increase `KASSANDRA_POST_FIRE_LOCKOUT_MS`.
- If CPU spikes during sustained noise: increase `KASSANDRA_PREDICT_CADENCE_MS` to ~300 (trades responsiveness for CPU; doesn't help the core latency problem).
- To capture audio for offline analysis: set `KASSANDRA_DUMP_PCM=/app/src-tauri/wake_dump.pcm`, speak, stop the app, run `cargo run --example test_wakeword -- wake_dump.pcm slide`.
