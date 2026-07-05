#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, oneshot};

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
    /// Score above which we fire. livekit-wakeword's silence floor is ~0.003;
    /// false-noise sits at ~0.005-0.30; a clear wake word is 0.5-0.9. Default
    /// 0.45 sits in the gap. Lower to 0.35 if misses; raise if false fires.
    wake_threshold: f32,
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
    /// a fair shot at its scoring position every predict. Default 32000.
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
            wake_threshold: env_or("KASSANDRA_WAKE_THRESHOLD", 0.45),
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
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    speaker: tauri::State<'_, Speaker>,
) -> Result<(), String> {
    let s = state.lock().await;
    s.in_call.store(false, Ordering::SeqCst);
    drop(s);
    // Emit immediately so the frontend updates before run_call's WebSocket
    // cleanup finishes (reader/writer task teardown can take seconds).
    let _ = app.emit("qwen_state", "disconnected");
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

    // Wake-word detection is enabled by default (KASSANDRA_WAKE_ENABLED=true).
    // Continuous rolling-buffer predict — see docs/wake-word.md for the full
    // architecture. The native ONNX Runtime fork achieves ~17 ms/predict, so
    // we predict every 100ms mic chunk on a tokio worker thread without
    // blocking the mic read loop. Calls can also be started manually via the
    // `start_call` Tauri command (the Call button), which flips `in_call` and
    // lets the top-of-loop check run `qwen::run_call` with an empty wake chunk.
    let wake_enabled = env_bool("KASSANDRA_WAKE_ENABLED", true);

    let detector: Option<Arc<std::sync::Mutex<livekit_wakeword::WakeWordModel>>> = if wake_enabled {
        match wakeword::init_detector() {
            Ok(d) => {
                eprintln!("Wakeword detector loaded successfully");
                Some(Arc::new(std::sync::Mutex::new(d)))
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

    // Wake-word detection — livekit-wakeword's intended architecture: a
    // **rolling 2s buffer with continuous predict**. No energy gate, no VAD,
    // no utterance framing, no lead-silence crutch. With the native ONNX
    // Runtime fork at ~17 ms/predict, we predict once per 100ms mic chunk
    // (dispatched on a tokio worker thread) so the mic read loop never stalls.
    //
    // The mel+embedding classifier pipeline is itself the "is this speech"
    // filter — non-speech audio scores ~0.005. No external gate needed.
    //
    // Pipeline (per 100ms chunk):
    //   1. AEC denoise (NS + HPF; AEC3 idle until Qwen call pushes render).
    //   2. Append to the rolling buffer; drain oldest past WAKE_BUFFER_SAMPLES.
    //   3. Emit wake_rms (denoised energy, 0..1) every chunk for reactive bars.
    //   4. If no predict in flight and outside the post-fire lockout: clone the
    //      rolling buffer, spawn predict() on a tokio worker, collect score via
    //      oneshot. On fire: emit fired + start the Qwen call.
    //
    // wake_state / wake_rms events drive the frontend circle:
    //   listening  — idle, bars react to wake_rms continuously
    //   fired      — score > threshold + Qwen call starting (payload: {score})
    //   error      — mic / predict failure (payload: {msg})

    let mut rolling: Vec<i16> = Vec::with_capacity(cfg.wake_buffer_samples + WakeConfig::CHUNK_SAMPLES);
    let mut lockout_until: Instant = Instant::now();
    let mut pending_wake_chunk: Option<Vec<i16>> = None;
    let mut predict_result: Option<oneshot::Receiver<f32>> = None;
    let mut predict_count: u32 = 0;

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
            // Fire delivered us here; run the Qwen call, then reset for a
            // fresh wake cycle.
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
            // Drop captured audio + in-flight predict, re-arm a fresh cycle.
            predict_result = None;
            rolling.clear();
            lockout_until = Instant::now();
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

        // Emit wake_rms every chunk so the frontend bars react continuously.
        let _ = app.emit("wake_rms", rms_norm);

        // Check if the previous predict completed.
        let fired_score: Option<f32> = if let Some(mut rx) = predict_result.take() {
            match rx.try_recv() {
                Ok(score) => {
                    eprintln!(
                        "wake predict #{}: score={:.4} (rolling {}/{} samples)",
                        predict_count,
                        score,
                        rolling.len(),
                        cfg.wake_buffer_samples
                    );
                    (score > cfg.wake_threshold).then_some(score)
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    predict_result = Some(rx); // still running, put back
                    None
                }
                Err(oneshot::error::TryRecvError::Closed) => None,
            }
        } else {
            None
        };

        if let Some(score) = fired_score {
            eprintln!("wake detected (score {score:.3})");
            let _ = app.emit("wake_state", json!({"state": "fired", "score": score}));
            let _ = app.emit("qwen_state", "wake_detected");
            let _ = app.emit("qwen_state", "connecting");

            // Hand the audio that fired to Qwen as its first input
            // chunk, then clear the window so post-call residual can't
            // re-fire the same word.
            pending_wake_chunk = Some(rolling.clone());
            rolling.clear();
            lockout_until = Instant::now() + cfg.post_fire_lockout;
            state.lock().await.in_call.store(true, Ordering::SeqCst);
            continue;
        }

        // Start a new predict if no inflight, outside lockout, and buffer full.
        let now = Instant::now();
        if predict_result.is_none() && now >= lockout_until && rolling.len() >= cfg.wake_buffer_samples {
            if let Some(ref det) = detector {
                let (tx, rx) = oneshot::channel();
                predict_result = Some(rx);
                let buffer = rolling.clone();
                let det = det.clone();
                predict_count += 1;
                tokio::spawn(async move {
                    let score = det.lock().unwrap()
                        .predict(&buffer)
                        .ok()
                        .and_then(|scores| scores.get("kassandra").copied())
                        .unwrap_or(-1.0);
                    let _ = tx.send(score);
                });
            }
        }
    }
}