#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Mutex;

use tauri::{Emitter, Manager};

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
        .invoke_handler(tauri::generate_handler![end_call, toggle_mute, trigger_wake, console_log])
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

#[tauri::command]
async fn trigger_wake(
    state: tauri::State<'_, Arc<Mutex<AppState>>>,
    speaker: tauri::State<'_, Speaker>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    let state_clone = state.inner().clone();
    let app_clone = app.clone();
    let speaker_clone = speaker.inner().clone();

    if state_clone.lock().await.wakeword_active.load(Ordering::SeqCst) {
        let _ = app_clone.emit("qwen_state", "wake_detected");
        let _ = app_clone.emit("qwen_state", "connecting");
        state_clone.lock().await.in_call.store(true, Ordering::SeqCst);
        return Ok(());
    }

    tauri::async_runtime::spawn(async move {
        let _ = app_clone.emit("qwen_state", "wake_detected");
        let _ = app_clone.emit("qwen_state", "connecting");

        let mic_stream = match audio::open_mic() {
            Ok(m) => Arc::new(Mutex::new(m)),
            Err(e) => {
                eprintln!("mic error: {e}");
                let _ = app_clone.emit("qwen_error", &format!("Mic error: {e}"));
                return;
            }
        };

        state_clone.lock().await.in_call.store(true, Ordering::SeqCst);

        let empty_chunk: Vec<i16> = vec![0; 3200];
        match qwen::run_call(&state_clone, &app_clone, &mic_stream, &speaker_clone, &empty_chunk).await {
            Ok(()) => {}
            Err(e) => {
                eprintln!("qwen error: {e}");
                let _ = app_clone.emit("qwen_error", &e.to_string());
            }
        }

        state_clone.lock().await.in_call.store(false, Ordering::SeqCst);
        let _ = app_clone.emit("qwen_state", "idle");
    });
    Ok(())
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
            eprintln!("Use the UI 'trigger_wake' command to test Qwen connection");
            None
        }
    };

    if detector.is_none() {
        // Park the loop — UI will trigger via command instead
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    let mut detector = detector.unwrap();
    state.lock().await.wakeword_active.store(true, Ordering::SeqCst);
    let mic_stream = audio::open_mic()?;
    let mic_stream = Arc::new(Mutex::new(mic_stream));

    loop {
        if state.lock().await.in_call.load(Ordering::SeqCst) {
            let empty_chunk: Vec<i16> = vec![0; 3200];
            match qwen::run_call(&state, &app, &mic_stream, &speaker, &empty_chunk).await {
                Ok(()) => {}
                Err(e) => {
                    eprintln!("qwen error: {e}");
                    let _ = app.emit("qwen_error", &e.to_string());
                }
            }
            state.lock().await.in_call.store(false, Ordering::SeqCst);
            let _ = app.emit("qwen_state", "idle");
            continue;
        }

        let chunk = audio::read_chunk(&mic_stream).await?;

        if state.lock().await.muted.load(Ordering::SeqCst) {
            continue;
        }

        let scores = detector.predict(&chunk)?;

        if let Some(score) = scores.get("kassandra") {
            if *score > 0.5 {
                let _ = app.emit("qwen_state", "wake_detected");
                let _ = app.emit("qwen_state", "connecting");

                state.lock().await.in_call.store(true, Ordering::SeqCst);

                match qwen::run_call(&state, &app, &mic_stream, &speaker, &chunk).await {
                    Ok(()) => {}
                    Err(e) => {
                        let _ = app.emit("qwen_error", &e.to_string());
                    }
                }

                state.lock().await.in_call.store(false, Ordering::SeqCst);
                let _ = app.emit("qwen_state", "idle");
            }
        }
    }
}
