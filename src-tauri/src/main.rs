#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

use tauri::{Emitter, Manager};
use serde_json::json;

use kassandra::audio::{self, Speaker};
use kassandra::qwen;
use kassandra::wakeword;
use kassandra::AppState;

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
        .invoke_handler(tauri::generate_handler![end_call, toggle_mute, console_log])
        .setup(move |app| {
            let state = app.state::<Arc<Mutex<AppState>>>();
            let app_handle = app.handle().clone();
            let state_clone = state.inner().clone();
            tauri::async_runtime::spawn(async move {
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

async fn run_voice_agent(
    state: Arc<Mutex<AppState>>,
    app: tauri::AppHandle,
    speaker: Speaker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = app.emit("qwen_state", "idle");

    let detector = match wakeword::init_detector() {
        Ok(d) => {
            eprintln!("Wakeword detector loaded successfully");
            Some(d)
        }
        Err(e) => {
            eprintln!("Wakeword detector not available: {e}");
            None
        }
    };

    if detector.is_none() {
        let _ = app.emit("wake_state", json!({"state": "error", "msg": "wakeword model not loaded"}));
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    let mut detector = detector.unwrap();
    state.lock().await.wakeword_active.store(true, Ordering::SeqCst);
    let mic_stream = audio::open_mic()?;
    let mic_stream = Arc::new(Mutex::new(mic_stream));

    // Energy-gated wake-word detection with **leading-silence framing**.
    //
    // Background: livekit-wakeword 0.1's predict() needs a 2s window and is
    // position-sensitive. The kassandra.onnx classifier was trained on TTS
    // positives that have ~800ms of leading silence before the word. Feeding
    // it speech starting at sample 0 scores 0.008 on the user's recorded
    // "Kassandra"; feeding it 800ms silence + the same speech scores 0.66
    // (220× separation from the 0.003 silence baseline). Verified in
    // examples/test_wakeword.rs.
    //
    // Pipeline:
    //   1. Monitor per-chunk RMS to detect speech start / end.
    //   2. Buffer speech (plus 800ms trailing-silence confirmation).
    //   3. On end-of-speech, build predict buffer:
    //        [800ms zero] + [last 1.2s of speech (excluding 800ms tail)]
    //      padded with trailing zeros to 2s. The word sits at t=0.8s in the
    //      window — exactly where the model expects it.
    //   4. Run predict() ONCE per utterance. CPU is now bounded by utterance
    //      rate, not polling. ~5× less CPU than the 500ms rolling-buffer
    //      version.
    //
    // wake_state / wake_rms events drive the frontend circle visualization:
    //   listening  — idle, waiting for speech
    //   hearing    — speech detected, RMS bars react to wake_rms
    //   processing — trailing silence confirmed, predict() running
    //   fired      — score > threshold, Qwen call starting (payload: {score})
    //   rejected   — score <= threshold (payload: {score})
    //   error      — mic / predict failure (payload: {msg})
    // wake_rms emits a normalized 0.0-1.0 energy reading every 100ms while
    // in the hearing state so the circle's bars react to the user's voice.
    //
    // Tuned from the live diagnostic log: silence floor RMS ≈ 44, speaking
    // RMS ≈ 350. Threshold 80 sits at 1.8× the noise floor and well below
    // speech, leaving headroom both ways.
    const RMS_SPEECH_THRESHOLD: f32 = 250.0; // ~3× silence floor; rejects background noise
    const SILENCE_CONFIRM_CHUNKS: u32 = 8; // 800ms below threshold ⇒ end of speech
    const CHUNK_SAMPLES: usize = 1600; // 100ms @ 16kHz
    const WAKE_BUFFER_SAMPLES: usize = 32000; // 2s @ 16kHz (crate's predict minimum)
    const WAKE_THRESHOLD: f32 = 0.45; // silence floor 0.003, false-noise ~0.005-0.30, real word 0.5-0.9
    const LEAD_SILENCE_SAMPLES: usize = 12800; // 800ms — matches TTS training data
    const MAX_SPEECH_SAMPLES: usize = WAKE_BUFFER_SAMPLES - LEAD_SILENCE_SAMPLES; // 1.2s — leaves room for lead in 2s window
    const RMS_NORMALIZE: f32 = 1000.0; // chunk_rms / RMS_NORMALIZE ⇒ 0.0-1.0 for CSS bars

    let mut speech_buf: Vec<i16> = Vec::with_capacity(WAKE_BUFFER_SAMPLES + 1600);
    let mut in_speech = false;
    let mut silence_run: u32 = 0;
    let mut predict_count: u32 = 0;

    let _ = app.emit("wake_state", json!({"state": "listening"}));

    loop {
        if state.lock().await.in_call.load(Ordering::SeqCst) {
            let empty_chunk: Vec<i16> = vec![0; 3200];
            match qwen::run_call(&state, &app, &mic_stream, &speaker, &empty_chunk).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("qwen error: {e}");
                    let _ = app.emit("qwen_error", &e.to_string());
                    let _ = app.emit("wake_state", json!({"state": "error", "msg": e.to_string()}));
                }
            }
            state.lock().await.in_call.store(false, Ordering::SeqCst);
            let _ = app.emit("qwen_state", "idle");
            // Drop any captured audio so post-call speech doesn't re-trigger.
            speech_buf.clear();
            in_speech = false;
            silence_run = 0;
            let _ = app.emit("wake_state", json!({"state": "listening"}));
            continue;
        }

        let chunk = audio::read_chunk(&mic_stream).await?;

        if state.lock().await.muted.load(Ordering::SeqCst) {
            continue;
        }

        let chunk_rms = (chunk.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
            / chunk.len().max(1) as f64)
            .sqrt() as f32;

        if chunk_rms > RMS_SPEECH_THRESHOLD {
            if !in_speech {
                let _ = app.emit("wake_state", json!({"state": "hearing"}));
            }
            in_speech = true;
            silence_run = 0;
            speech_buf.extend_from_slice(&chunk);
            if speech_buf.len() > WAKE_BUFFER_SAMPLES {
                speech_buf.drain(..speech_buf.len() - WAKE_BUFFER_SAMPLES);
            }
            let rms_norm = (chunk_rms / RMS_NORMALIZE).clamp(0.0, 1.0);
            let _ = app.emit("wake_rms", rms_norm);
        } else if in_speech {
            // Below speech threshold while in speech — either a mid-word
            // consonant gap or the trailing silence of the utterance. Buffer
            // it (it's the natural tail of the word) and count toward the
            // end-of-speech confirmation.
            silence_run += 1;
            speech_buf.extend_from_slice(&chunk);
            if speech_buf.len() > WAKE_BUFFER_SAMPLES {
                speech_buf.drain(..speech_buf.len() - WAKE_BUFFER_SAMPLES);
            }
            // Keep bars live but low during the gap so the visual stays in
            // "hearing" mode until processing fires.
            let _ = app.emit("wake_rms", (chunk_rms / RMS_NORMALIZE).clamp(0.0, 1.0));

            if silence_run >= SILENCE_CONFIRM_CHUNKS {
                // End of speech — predict on the captured segment.
                in_speech = false;
                silence_run = 0;
                let _ = app.emit("wake_state", json!({"state": "processing"}));

                let tail = SILENCE_CONFIRM_CHUNKS as usize * CHUNK_SAMPLES;
                let speech_end = speech_buf.len().saturating_sub(tail);
                let speech_samples = speech_end;
                // Keep only the last MAX_SPEECH_SAMPLES of speech so the
                // lead+speech fits inside the 2s predict window.
                let speech_start = speech_end.saturating_sub(MAX_SPEECH_SAMPLES);

                // Build predict buffer: [800ms lead silence] + [speech] + [pad]
                let mut predict_buf = vec![0i16; LEAD_SILENCE_SAMPLES];
                predict_buf.extend_from_slice(&speech_buf[speech_start..speech_end]);
                if predict_buf.len() < WAKE_BUFFER_SAMPLES {
                    predict_buf.extend(std::iter::repeat(0i16).take(
                        WAKE_BUFFER_SAMPLES - predict_buf.len(),
                    ));
                }
                predict_buf.truncate(WAKE_BUFFER_SAMPLES);

                let scores = detector.predict(&predict_buf)?;
                predict_count += 1;
                let score = scores.get("kassandra").copied().unwrap_or(-1.0);
                eprintln!(
                    "wake predict #{}: score={:.4} (speech {} samples + {}ms trailing silence, lead 800ms)",
                    predict_count,
                    score,
                    speech_samples,
                    SILENCE_CONFIRM_CHUNKS * 100
                );

                if score > WAKE_THRESHOLD {
                    eprintln!("wake detected (score {score:.3})");
                    let _ = app.emit("wake_state", json!({"state": "fired", "score": score}));
                    let _ = app.emit("qwen_state", "wake_detected");
                    let _ = app.emit("qwen_state", "connecting");

                    state.lock().await.in_call.store(true, Ordering::SeqCst);

                    match qwen::run_call(
                        &state,
                        &app,
                        &mic_stream,
                        &speaker,
                        &speech_buf.clone(),
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(e) => {
                            let _ = app.emit("qwen_error", &e.to_string());
                            let _ = app.emit("wake_state", json!({"state": "error", "msg": e.to_string()}));
                        }
                    }

                    state.lock().await.in_call.store(false, Ordering::SeqCst);
                    let _ = app.emit("qwen_state", "idle");
                } else {
                    let _ = app.emit("wake_state", json!({"state": "rejected", "score": score}));
                }

                speech_buf.clear();
            }
        }
    }
}
