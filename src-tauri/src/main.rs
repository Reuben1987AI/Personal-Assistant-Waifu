#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use tauri::{Emitter, Manager};
use serde_json::json;

use kassandra::audio::{self, Speaker};
use kassandra::qwen;
use kassandra::wakeword;
use kassandra::AppState;

/// Wake-loop tuning. Every value is overridable via an env var of the same
/// name (e.g. `KASSANDRA_WAKE_THRESHOLD=0.35`) so thresholds can be tweaked
/// without a rebuild — same style as the Qwen model/voice/region config in
/// `client.rs`. See `docs/.session-status.md` for tuning guidance.
struct WakeConfig {
    /// RMS floor (i16, 0..32767) below which a 100ms chunk is treated as
    /// silence. The AEC noise-suppressor runs first, so this measures the
    /// *denoised* energy. Default 250 — quiet enough to catch a clear spoken
    /// word, high enough to ignore breath / HVAC / tape hiss.
    rms_threshold: f32,
    /// While active, run `kassandra.onnx` on the rolling buffer at most once
    /// per this interval. Lower ⇒ more responsive but more CPU. Default 200ms.
    predict_cadence: Duration,
    /// Score above which we fire. livekit-wakeword's silence floor is ~0.003;
    /// false-noise sits at ~0.005-0.30; a clear wake word is 0.5-0.9. Default
    /// 0.45 sits in the gap. Lower to 0.35 if misses; raise if false fires.
    wake_threshold: f32,
    /// Active → inactive requires this many consecutive sub-threshold 100ms
    /// chunks. 8 = 800ms of quiet confirms an utterance ended (so we can emit
    /// a `rejected` score for UX feedback). Default 8.
    silence_confirm_chunks: u32,
    /// After a successful fire, ignore further wake events for this long so
    /// the same word still sitting in the rolling buffer can't double-fire.
    /// The actual Qwen call typically blocks the wake loop far longer than
    /// this; the lockout covers the brief fire → call-start window and any
    /// very-short/manual call. Default 2000ms.
    post_fire_lockout: Duration,
    /// Rolling buffer length in samples (16kHz mono). livekit-wakeword 0.1's
    /// `predict()` needs ~2s of audio — shorter windows silently return 0.0.
    /// 32000 samples = 2.0s. The word naturally slides through every position
    /// in this window, so the undertrained/position-sensitive classifier gets
    /// a fair shot at its scoring position every cadence tick. Default 32000.
    wake_buffer_samples: usize,
}

impl WakeConfig {
    const CHUNK_SAMPLES: usize = 1600; // 100ms @ 16kHz
    const RMS_NORMALIZE: f32 = 1000.0; // chunk_rms / RMS_NORMALIZE ⇒ 0..1 for CSS

    fn from_env() -> Self {
        fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
            std::env::var(key)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default)
        }
        Self {
            rms_threshold: env_or("KASSANDRA_RMS_THRESHOLD", 250.0),
            predict_cadence: Duration::from_millis(env_or(
                "KASSANDRA_PREDICT_CADENCE_MS",
                200u64,
            )),
            wake_threshold: env_or("KASSANDRA_WAKE_THRESHOLD", 0.45),
            silence_confirm_chunks: env_or("KASSANDRA_SILENCE_CONFIRM_CHUNKS", 8u32),
            post_fire_lockout: Duration::from_millis(env_or(
                "KASSANDRA_POST_FIRE_LOCKOUT_MS",
                2000u64,
            )),
            wake_buffer_samples: env_or("KASSANDRA_WAKE_BUFFER_SAMPLES", 32000usize),
        }
    }
}

/// Parse a boolean env var. Recognizes `1`/`true`/`yes`/`on` (case-insensitive);
/// anything else (including unset) falls back to `default`.
fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(default)
}

fn main() {
    dotenvy::dotenv().ok();

    let app_state = Arc::new(Mutex::new(AppState::new()));

    let speaker = match audio::open_speaker() {
        Ok(s) => {
            eprintln!("Speaker opened successfully");
            s
        }
        Err(e) => {
            eprintln!("Speaker not available: {e}");
            Speaker::dummy()
        }
    };

    tauri::Builder::default()
        .manage(app_state.clone())
        .manage(speaker.clone())
        .invoke_handler(tauri::generate_handler![end_call, toggle_mute, start_call, console_log])
        .setup(move |app| {
            let state = app.state::<Arc<Mutex<AppState>>>();
            // app_data_dir is captured resolve-time (sync); the teacher state
            // load itself runs in the spawned async task (it locks AppState).
            let data_dir = app.path().app_data_dir().ok();
            let app_handle = app.handle().clone();
            let state_clone = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                // Load curriculum.json + progress.json from the per-user app
                // data dir. Seeds curriculum.json from the embedded default
                // on first run. Errors only log — teacher mode just won't
                // work, the rest of the app should still start.
                if let Some(dir) = data_dir {
                    if let Err(e) = state_clone.lock().await.teacher.init(dir) {
                        eprintln!("teacher state init failed: {e}");
                    }
                } else {
                    eprintln!("app_data_dir unavailable — teacher mode disabled");
                }
                if let Err(e) = run_voice_agent(state_clone, app_handle, speaker).await {
                    eprintln!("Voice agent error: {e}");
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[tauri::command]
async fn console_log(message: String) -> Result<(), String> {
    eprintln!("[frontend] {message}");
    Ok(())
}

#[tauri::command]
async fn end_call(
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    speaker: tauri::State<'_, Speaker>,
) -> Result<(), String> {
    let s = state.lock().await;
    s.in_call.store(false, Ordering::SeqCst);
    drop(s);
    speaker.clear().await;
    Ok(())
}

#[tauri::command]
async fn toggle_mute(state: tauri::State<'_, Arc<Mutex<AppState>>>) -> Result<bool, String> {
    let s = state.lock().await;
    let new_muted = !s.muted.load(Ordering::SeqCst);
    s.muted.store(new_muted, Ordering::SeqCst);
    Ok(new_muted)
}

/// Manual call start (wake word deferred — see docs/wake-word.md). Flips
/// `in_call` so the voice-agent loop's top-of-loop check runs `qwen::run_call`
/// with the working mic + AEC wiring and an empty wake chunk. Emits
/// `qwen_state: connecting` for immediate UI feedback before the WebSocket
/// opens (run_call emits `connected` once it's up). Fails if a call is already
/// in progress.
#[tauri::command]
async fn start_call(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
) -> Result<(), String> {
    let s = state.lock().await;
    if s.in_call.load(Ordering::SeqCst) {
        return Err("call already in progress".into());
    }
    s.in_call.store(true, Ordering::SeqCst);
    drop(s);
    let _ = app.emit("qwen_state", "connecting");
    Ok(())
}

async fn run_voice_agent(
    state: Arc<Mutex<AppState>>,
    app: tauri::AppHandle,
    speaker: Speaker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = app.emit("qwen_state", "idle");

    // Hoisted AEC instance: shared between the wake loop (denoise before the
    // RMS gate) and the Qwen call (echo cancellation with speaker render).
    // While idle, no render is pushed — process_capture_frame is fed zero
    // render, leaving only the noise-suppressor + high-pass filter active,
    // which is exactly the wake pre-filter we want.
    let aec: Option<Arc<std::sync::Mutex<audio::Aec>>> = match audio::Aec::new() {
        Ok(a) => {
            eprintln!("AEC enabled (WebRTC AEC3) — wake pre-filter + call echo cancellation");
            Some(Arc::new(std::sync::Mutex::new(a)))
        }
        Err(e) => {
            eprintln!(
                "AEC unavailable — wake loop runs on raw mic, call has no echo cancel: {e}"
            );
            None
        }
    };

    // Wake-word detection is gated behind KASSANDRA_WAKE_ENABLED (default
    // false). The rolling-buffer wake loop stays in the code (compiles, useful
    // for future wake work — see docs/wake-word.md) but is skipped at runtime
    // in manual mode so it isn't burning a core or emitting wake events. Calls
    // are started manually via the `start_call` Tauri command, which flips
    // `in_call` and lets the loop's top-of-loop check run `qwen::run_call` with
    // the working mic + AEC wiring and an empty wake chunk.
    let wake_enabled = env_bool("KASSANDRA_WAKE_ENABLED", false);

    let mut detector = if wake_enabled {
        match wakeword::init_detector() {
            Ok(d) => {
                eprintln!("Wakeword detector loaded successfully");
                Some(d)
            }
            Err(e) => {
                eprintln!("Wakeword detector not available: {e}");
                let _ = app.emit("wake_state", json!({"state": "error", "msg": "wakeword model not loaded"}));
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    } else {
        eprintln!(
            "Wake word disabled (KASSANDRA_WAKE_ENABLED=false) — manual call mode; use the Call button"
        );
        None
    };

    let cfg = WakeConfig::from_env();

    if wake_enabled {
        state.lock().await.wakeword_active.store(true, Ordering::SeqCst);
    }
    let mic_stream = audio::open_mic()?;
    let mic_stream = Arc::new(Mutex::new(mic_stream));

    // Wake-word detection — livekit-wakeword's intended architecture: an
    // **energy-gated rolling 2s buffer with continuous predict at a
    // configurable cadence**. No VAD, no utterance framing, no lead-silence
    // crutch.
    //
    // Background: livekit-wakeword 0.1's `predict()` needs a 2s window and is
    // position-sensitive — the kassandra.onnx classifier was trained on TTS
    // positives that have ~800ms of leading silence before the word. Earlier
    // attempts worked around this by detecting utterance boundaries and pre-
    // pending lead silence ("framing crutch"). The rolling buffer makes all of
    // that irrelevant: as the user speaks, every new 100ms chunk pushes the
    // window forward, so the word naturally slides through every position
    // inside the 2s window. At some cadence tick it lands on the classifier's
    // scoring position and fires.
    //
    // The mel+embedding classifier pipeline is itself the "is this speech"
    // filter — non-speech audio scores low (~0.005). The energy gate is just
    // a cheap outer "is there any sound at all?" filter that keeps us from
    // burning CPU on the ONNX when the room is silent. The AEC noise
    // suppressor (path A0) runs *before* the RMS computation, so the gate
    // measures denoised energy — steady background no longer cycles predicts.
    //
    // Pipeline (per 100ms chunk):
    //   1. AEC denoise (NS + HPF; AEC3 is idle until a Qwen call pushes
    //      render) → denoised chunk.
    //   2. Append to the rolling buffer; drain oldest past WAKE_BUFFER_SAMPLES.
    //   3. RMS gate: chunk_rms >= rms_threshold ⇒ "active" (emit `hearing`).
    //   4. While active && not in post-fire lockout && predict cadence elapsed:
    //        detector.predict(&rolling_buffer); if score > wake_threshold ⇒ fire.
    //      Else (sub-threshold) accumulate silence_run; after
    //      silence_confirm_chunks go inactive and emit `rejected` (or
    //      `listening` if we fired during this active period).
    //
    // wake_state / wake_rms events drive the frontend circle:
    //   listening  — idle, bars breathe via CSS (no wake_rms emitted)
    //   hearing    — RMS above gate, bars react to wake_rms
    //   fired      — score > threshold + Qwen call starting (payload: {score})
    //   rejected   — went inactive without firing (payload: {score})
    //   error      — mic / predict failure (payload: {msg})

    let mut rolling: Vec<i16> = Vec::with_capacity(cfg.wake_buffer_samples + WakeConfig::CHUNK_SAMPLES);
    let mut active = false;
    let mut silence_run: u32 = 0;
    let mut predict_count: u32 = 0;
    let mut last_predict: Instant = Instant::now();
    let mut last_score: f32 = 0.0;
    let mut lockout_until: Instant = Instant::now(); // far past → unlocked at start
    let mut fired_this_utterance = false;
    let mut pending_wake_chunk: Option<Vec<i16>> = None;

    // Optional diagnostic: append every denoised wake-loop chunk (the exact
    // bytes the rolling buffer / classifier sees) to a raw s16le mono 16kHz
    // PCM file. Run `test_wakeword slide <file>` on the dump to see whether
    // the live AEC-processed mic is recognizable to the model. Off by default;
    // set KASSANDRA_DUMP_PCM=/app/src-tauri/wake_dump.pcm to enable.
    let mut dump: Option<std::fs::File> = std::env::var("KASSANDRA_DUMP_PCM").ok().map(|p| {
        match std::fs::File::create(&p) {
            Ok(f) => {
                eprintln!("wake PCM dump → {p} (raw s16le mono 16kHz)");
                Some(f)
            }
            Err(e) => {
                eprintln!("wake PCM dump disabled (open {p} failed: {e})");
                None
            }
        }
    }).flatten();

    let _ = app.emit("wake_state", json!({"state": "listening"}));

    loop {
        if state.lock().await.in_call.load(Ordering::SeqCst) {
            // Fire delivered us here; run the Qwen call, then reset the wake
            // state machine so the next session starts clean.
            let wake_chunk = pending_wake_chunk
                .take()
                .unwrap_or_else(|| vec![0; 2 * WakeConfig::CHUNK_SAMPLES]);
            match qwen::run_call(&state, &app, &mic_stream, &speaker, &aec, &wake_chunk).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("qwen error: {e}");
                    let _ = app.emit("qwen_error", &e.to_string());
                    let _ = app.emit("wake_state", json!({"state": "error", "msg": e.to_string()}));
                }
            }
            state.lock().await.in_call.store(false, Ordering::SeqCst);
            let _ = app.emit("qwen_state", "idle");
            // Drop any captured audio and re-arm a fresh wake cycle.
            rolling.clear();
            active = false;
            silence_run = 0;
            fired_this_utterance = false;
            last_predict = Instant::now();
            let _ = app.emit("wake_state", json!({"state": "listening"}));
            continue;
        }

        let chunk = audio::read_chunk(&mic_stream).await?;

        // Manual mode (wake disabled): drain the mic to keep the bounded
        // mpsc channel (cap 32) from backing up and stalling the cpal capture
        // callback, then skip wake detection. The `in_call` check above
        // handles call start/stop; nothing else to do while idle.
        if !wake_enabled {
            continue;
        }

        if state.lock().await.muted.load(Ordering::SeqCst) {
            continue;
        }

        // Apply the shared AEC stack (NS + HPF; AEC3 is idle until a Qwen call
        // pushes render) BEFORE the RMS gate. NS strips steady background so
        // the energy gate measures denoised loudness. Echo cancellation
        // against stale render would only matter during a call, and the wake
        // loop doesn't run then.
        let chunk: Vec<i16> = match &aec {
            Some(a) => a.lock().unwrap().process_capture(&chunk),
            None => chunk,
        };

        if let Some(f) = &mut dump {
            use std::io::Write;
            let bytes: Vec<u8> = chunk.iter().flat_map(|s| s.to_le_bytes()).collect();
            let _ = f.write_all(&bytes);
        }

        let chunk_rms = (chunk.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
            / chunk.len().max(1) as f64)
            .sqrt() as f32;
        let rms_norm = (chunk_rms / WakeConfig::RMS_NORMALIZE).clamp(0.0, 1.0);

        // Push into the rolling window (the classifier always sees the latest
        // 2s of denoised audio, slides the word through every position).
        rolling.extend_from_slice(&chunk);
        let surplus = rolling.len().saturating_sub(cfg.wake_buffer_samples);
        if surplus > 0 {
            rolling.drain(..surplus);
        }

        let now = Instant::now();

        if chunk_rms >= cfg.rms_threshold {
            if !active {
                active = true;
                fired_this_utterance = false;
                let _ = app.emit("wake_state", json!({"state": "hearing"}));
            }
            silence_run = 0;
        } else if active {
            silence_run += 1;
            if silence_run >= cfg.silence_confirm_chunks {
                active = false;
                if fired_this_utterance {
                    let _ = app.emit("wake_state", json!({"state": "listening"}));
                } else {
                    let _ =
                        app.emit("wake_state", json!({"state": "rejected", "score": last_score}));
                }
            }
        }

        // Bars react while we believe there's audio (active + the
        // silence-confirm tail). Once fully idle the frontend fades to the
        // CSS-driven listening animation; we stop emitting so it can.
        if active {
            let _ = app.emit("wake_rms", rms_norm);
        }

        // Rolling predict — only while active and outside the post-fire lockout.
        if active && now >= lockout_until && now.duration_since(last_predict) >= cfg.predict_cadence {
            last_predict = now;
            if rolling.len() >= cfg.wake_buffer_samples {
                let scores = detector
                    .as_mut()
                    .expect("wake detector present when wake_enabled")
                    .predict(&rolling)?;
                predict_count += 1;
                last_score = scores.get("kassandra").copied().unwrap_or(-1.0);
                eprintln!(
                    "wake predict #{predict_count}: score={last_score:.4} (rolling {}/{} samples)",
                    rolling.len(),
                    cfg.wake_buffer_samples
                );

                if last_score > cfg.wake_threshold {
                    eprintln!("wake detected (score {last_score:.3})");
                    let _ = app.emit("wake_state", json!({"state": "fired", "score": last_score}));
                    let _ = app.emit("qwen_state", "wake_detected");
                    let _ = app.emit("qwen_state", "connecting");

                    // Hand the audio that fired to Qwen as its first input
                    // chunk, then clear the window so post-call residual can't
                    // re-fire the same word.
                    pending_wake_chunk = Some(rolling.clone());
                    rolling.clear();
                    active = false;
                    silence_run = 0;
                    fired_this_utterance = true;
                    lockout_until = now + cfg.post_fire_lockout;
                    state.lock().await.in_call.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}