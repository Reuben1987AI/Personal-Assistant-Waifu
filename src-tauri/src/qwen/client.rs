use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::audio::{Aec, MicStream, Speaker};
use crate::teacher::TeacherState;
use crate::AppState;

// Plus is required for tool calling (flash has no tools support — see
// docs/architecture.md). Teacher mode uses tools to enter/exit the mode and
// to keep the app in sync with LLM-driven phase transitions, so every call
// runs on plus. Higher cost than flash, accepted as the price of option B.
const DEFAULT_MODEL: &str = "qwen3.5-omni-plus-realtime";
const DEFAULT_VOICE: &str = "Tina";
const DEFAULT_INSTRUCTIONS: &str =
    "You are Kassandra, a personal AI assistant. Be warm, witty, and concise.";

pub async fn run_call(
    state: &Arc<Mutex<AppState>>,
    app: &AppHandle,
    mic: &Arc<Mutex<MicStream>>,
    speaker: &Speaker,
    aec: &Option<Arc<std::sync::Mutex<Aec>>>,
    wake_chunk: &[i16],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_key =
        std::env::var("DASHSCOPE_API_KEY").unwrap_or_else(|_| "sk-xxx".to_string());
    let model = std::env::var("QWEN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let voice = std::env::var("QWEN_VOICE").unwrap_or_else(|_| DEFAULT_VOICE.to_string());
    let instructions =
        std::env::var("QWEN_INSTRUCTIONS").unwrap_or_else(|_| DEFAULT_INSTRUCTIONS.to_string());
    let region = std::env::var("QWEN_REGION").unwrap_or_else(|_| "intl".to_string());

    let url = match region.as_str() {
        "cn" | "china" | "beijing" => {
            format!("wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
        "sg" | "singapore" => {
            let workspace_id = std::env::var("QWEN_WORKSPACE_ID")
                .unwrap_or_else(|_| "your-workspace-id".to_string());
            format!("wss://{workspace_id}.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
        _ => {
            format!("wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
    };

    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {api_key}").parse().unwrap(),
    );

    let (ws_stream, _) = connect_async(request).await?;
    let (mut write, mut read) = ws_stream.split();

    // Teacher-mode aware session.update. Tools are always registered (so the
    // LLM can call enter_chinese_teacher_mode() from a normal chat). When
    // teacher mode is on we additionally swap in the bare-minimum tutor
    // prompt and pin paraformer to Chinese (the strongest anti-cheat —
    // English "hello" can't autocorrect into 你好 if STT is CN-only).
    let (active_instructions, language_hint) = {
        let s = state.lock().await;
        if s.teacher.active {
            (s.teacher.build_instructions(), Some("zh"))
        } else {
            (instructions.clone(), None)
        }
    };
    // Clone for the spawned reader task (used when a tool call flips the mode
    // back off and we need to fall back to the default chat instructions).
    let instructions_for_closure = instructions.clone();
    let transcription = match language_hint {
        Some(lang) => json!({ "model": "paraformer-realtime-v2", "language": lang }),
        None => json!({ "model": "paraformer-realtime-v2" }),
    };
    let session_update = json!({
        "event_id": "session_001",
        "type": "session.update",
        "session": {
            "modalities": ["text", "audio"],
            "voice": voice,
            "input_audio_format": "pcm_16000hz_mono_16bit",
            "output_audio_format": "pcm_24000hz_mono_16bit",
            "input_audio_transcription": transcription,
            "instructions": active_instructions,
            "turn_detection": {
                "type": "semantic_vad",
                "threshold": 0.5,
                "silence_duration_ms": 800
            },
            "tools": TeacherState::build_tools()
        }
    });

    write
        .send(Message::Text(session_update.to_string().into()))
        .await?;

    let _ = app.emit("qwen_state", "connected");

    // Emit the current teacher mode/state at session start so the frontend
    // badge is in sync even on a fresh call (e.g. user resumed after restart
    // with progress.json on disk → mode was active, now active again).
    {
        let s = state.lock().await;
        let _ = app.emit("app_mode", s.teacher.active);
        let _ = app.emit("teacher_state", s.teacher.snapshot());
    }

    // Single writer task owns `write` (SplitSink doesn't implement Clone).
    // Both the mic feeder loop and the ws reader post JSON strings onto this
    // channel — the writer drains and sends them sequentially. This also lets
    // the reader send tool-call outputs (conversation.item.create + response
    // create + post-flip session.update) without needing a `write` handle.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if write.send(Message::Text(text.into())).await.is_err() {
                eprintln!("ws write failed (channel)");
                break;
            }
        }
        let _ = write.close().await;
    });

    let app_clone = app.clone();
    let speaker_clone = speaker.clone();
    let aec_clone = aec.clone();
    let state_clone = state.clone();
    let out_tx_reader = out_tx.clone();
    let reader = tokio::spawn(async move {
        let mut audio_chunks = 0u32;
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) {
                        let event_type = event["type"].as_str().unwrap_or("");
                        match event_type {
                            "response.audio.delta" => {
                                if let Some(delta) = event["delta"].as_str() {
                                    audio_chunks += 1;
                                    if let Ok(bytes) = BASE64.decode(delta) {
                                        let samples: Vec<i16> = bytes
                                            .chunks_exact(2)
                                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                            .collect();
                                        speaker_clone.push_chunk(&samples).await;
                                        if let Some(aec) = &aec_clone {
                                            aec.lock().unwrap().push_render(&samples);
                                        }
                                    }
                                }
                            }
                            "response.audio_transcript.delta" => {
                                if let Some(delta) = event["delta"].as_str() {
                                    let _ = app_clone.emit("qwen_transcript", delta);
                                }
                            }
                            "response.audio_transcript.done" => {
                                if let Some(transcript) = event["transcript"].as_str() {
                                    eprintln!("qwen transcript: {transcript}");
                                    let _ = app_clone.emit("qwen_response", transcript);
                                }
                            }
                            "conversation.item.input_audio_transcription.completed" => {
                                if let Some(transcript) = event["transcript"].as_str() {
                                    eprintln!("user transcript: {transcript}");
                                    let _ = app_clone.emit("user_transcript", transcript);
                                }
                            }
                            "input_audio_buffer.speech_started" => {
                                eprintln!("qwen: speech started");
                                let _ = app_clone.emit("qwen_state", "listening");
                            }
                            "response.audio.done" => {
                                eprintln!("qwen: response complete ({audio_chunks} audio chunks)");
                                let _ = app_clone.emit("qwen_state", "speaking");
                            }
                            // Tool-call dispatch. The realtime API emits one
                            // `response.function_call_arguments.done` per
                            // function call in a response, carrying `name` and
                            // `arguments` (a JSON string). We dispatch under
                            // the AppState mutex, persist, emit `app_mode` +
                            // `teacher_state` so the frontend badge updates,
                            // then submit a `function_call_output` so the
                            // conversation records the result, followed by a
                            // `response.create` instructing the LLM to
                            // continue based on the new teacher state. If the
                            // tool flipped the mode, we first send a fresh
                            // `session.update` so the LLM's follow-up runs
                            // under the tutor prompt (or back under the
                            // default chat prompt on exit).
                            "response.function_call_arguments.done" => {
                                let name = event["name"].as_str().unwrap_or("").to_string();
                                let args_str = event["arguments"].as_str().unwrap_or("{}").to_string();
                                let call_id = event["call_id"].as_str().unwrap_or("").to_string();
                                let parsed_args: serde_json::Value =
                                    serde_json::from_str(&args_str).unwrap_or(serde_json::Value::Null);
                                eprintln!(
                                    "tool call: {name} args={args_str} call_id={call_id}"
                                );

                                let need_session_update;
                                let output_text;
                                {
                                    let mut s = state_clone.lock().await;
                                    let prev_active = s.teacher.active;
                                    output_text =
                                        s.teacher.handle_tool_call(&name, &parsed_args);
                                    need_session_update = s.teacher.mode_edge(prev_active);
                                    let _ = app_clone.emit("app_mode", s.teacher.active);
                                    let _ = app_clone.emit("teacher_state", s.teacher.snapshot());
                                }

                                if need_session_update {
                                    let s = state_clone.lock().await;
                                    let (instr, lang) = if s.teacher.active {
                                        (s.teacher.build_instructions(), Some("zh"))
                                    } else {
                                        (instructions_for_closure.clone(), None)
                                    };
                                    let trans = match lang {
                                        Some(l) => json!({ "model": "paraformer-realtime-v2", "language": l }),
                                        None => json!({ "model": "paraformer-realtime-v2" }),
                                    };
                                    let upd = json!({
                                        "event_id": format!("session_update_{call_id}"),
                                        "type": "session.update",
                                        "session": {
                                            "instructions": instr,
                                            "input_audio_transcription": trans,
                                        }
                                    });
                                    if out_tx_reader.send(upd.to_string()).is_err() {
                                        eprintln!("tool: session.update send failed (channel closed)");
                                    }
                                }

                                let item = json!({
                                    "type": "conversation.item.create",
                                    "item": {
                                        "type": "function_call_output",
                                        "call_id": call_id,
                                        "output": output_text,
                                    }
                                });
                                let _ = out_tx_reader.send(item.to_string());

                                // Resume the turn. The follow-up instruction
                                // is baked from the current teacher state so
                                // the LLM picks up where it left off based on
                                // the now-persisted phase/target/known set.
                                let follow_up = {
                                    let s = state_clone.lock().await;
                                    if s.teacher.active {
                                        s.teacher.build_instructions()
                                    } else {
                                        "OK. Continue as Kassandra.".to_string()
                                    }
                                };
                                let resume = json!({
                                    "type": "response.create",
                                    "response": {
                                        "modalities": ["text", "audio"],
                                        "instructions": follow_up
                                    }
                                });
                                let _ = out_tx_reader.send(resume.to_string());
                            }
                            "error" => {
                                eprintln!("qwen server error: {text}");
                                let _ = app_clone.emit("qwen_error", text.as_str());
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    eprintln!("qwen ws error: {e}");
                    break;
                }
                _ => {}
            }
        }
    });

    let mut first_chunk = true;
    let mut chunk_count = 0u64;
    loop {
        if !state.lock().await.in_call.load(Ordering::SeqCst) {
            eprintln!("call ended by user");
            break;
        }

        let chunk = if first_chunk {
            first_chunk = false;
            Some(wake_chunk.to_vec())
        } else {
            match crate::audio::read_chunk(mic).await {
                Ok(c) => Some(c),
                Err(e) => {
                    eprintln!("mic read error: {e}");
                    None
                }
            }
        };

        if let Some(chunk) = chunk {
            chunk_count += 1;
            if chunk_count % 50 == 1 {
                let rms = (chunk.iter().map(|&s| (s as f64).powi(2)).sum::<f64>()
                    / chunk.len().max(1) as f64)
                    .sqrt();
                eprintln!(
                    "sent audio chunk #{chunk_count} ({} samples, RMS {:.1})",
                    chunk.len(),
                    rms
                );
            }

            if state.lock().await.muted.load(Ordering::SeqCst) {
                continue;
            }

            let to_send: Vec<i16> = match &aec {
                Some(a) => a.lock().unwrap().process_capture(&chunk),
                None => chunk,
            };

            let encoded = BASE64.encode(
                to_send
                    .iter()
                    .flat_map(|s| s.to_le_bytes())
                    .collect::<Vec<u8>>(),
            );

            let audio_msg = json!({
                "type": "input_audio_buffer.append",
                "audio": encoded
            });

            if out_tx.send(audio_msg.to_string()).is_err() {
                eprintln!("ws send failed (channel closed)");
                break;
            }
        } else {
            eprintln!("no mic chunk, breaking");
            break;
        }
    }

    // Drop the mic-feeder's sender so the writer task's channel drains and
    // ends; `out_tx_reader` (in the reader task) will also drop on task exit.
    drop(out_tx);
    let _ = reader.await;
    let _ = writer.await;

    // Drain stale render so the wake loop's post-call process_capture doesn't
    // run AEC3 against a phantom echo reference.
    if let Some(a) = aec {
        a.lock().unwrap().clear_render();
    }

    Ok(())
}

pub async fn run_call_manual(
    state: &Arc<Mutex<AppState>>,
    app: &AppHandle,
    speaker: &Speaker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_key =
        std::env::var("DASHSCOPE_API_KEY").unwrap_or_else(|_| "sk-xxx".to_string());
    let model = std::env::var("QWEN_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.to_string());
    let voice = std::env::var("QWEN_VOICE").unwrap_or_else(|_| DEFAULT_VOICE.to_string());
    let instructions =
        std::env::var("QWEN_INSTRUCTIONS").unwrap_or_else(|_| DEFAULT_INSTRUCTIONS.to_string());
    let region = std::env::var("QWEN_REGION").unwrap_or_else(|_| "intl".to_string());

    let url = match region.as_str() {
        "cn" | "china" | "beijing" => {
            format!("wss://dashscope.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
        "sg" | "singapore" => {
            let workspace_id = std::env::var("QWEN_WORKSPACE_ID")
                .unwrap_or_else(|_| "your-workspace-id".to_string());
            format!("wss://{workspace_id}.ap-southeast-1.maas.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
        _ => {
            format!("wss://dashscope-intl.aliyuncs.com/api-ws/v1/realtime?model={model}")
        }
    };

    let mut request = url.into_client_request()?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {api_key}").parse().unwrap(),
    );

    let (ws_stream, _) = connect_async(request).await?;
    let (mut write, mut read) = ws_stream.split();

    let session_update = json!({
        "event_id": "session_001",
        "type": "session.update",
        "session": {
            "modalities": ["text", "audio"],
            "voice": voice,
"input_audio_format": "pcm_16000hz_mono_16bit",
            "output_audio_format": "pcm_24000hz_mono_16bit",
            "input_audio_transcription": {
                "model": "paraformer-realtime-v2"
            },
            "instructions": instructions,
            "turn_detection": {
                "type": "semantic_vad",
                "threshold": 0.5,
                "silence_duration_ms": 800
            }
        }
    });

    write
        .send(Message::Text(session_update.to_string().into()))
        .await?;

    let response_create = json!({
        "type": "response.create",
        "response": {
            "modalities": ["text", "audio"],
            "instructions": "Greet the user briefly. Say hello and ask how you can help."
        }
    });

    write
        .send(Message::Text(response_create.to_string().into()))
        .await?;

    write
        .send(Message::Text(response_create.to_string().into()))
        .await?;

    let _ = app.emit("qwen_state", "connected");

    let app_clone = app.clone();
    let state_clone = state.clone();
    let speaker_clone = speaker.clone();
    let reader = tokio::spawn(async move {
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(event) = serde_json::from_str::<serde_json::Value>(&text) {
                        let event_type = event["type"].as_str().unwrap_or("");
                        match event_type {
                            "response.audio.delta" => {
                                if let Some(delta) = event["delta"].as_str() {
                                    if let Ok(bytes) = BASE64.decode(delta) {
                                        let samples: Vec<i16> = bytes
                                            .chunks_exact(2)
                                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                            .collect();
                                        speaker_clone.push_chunk(&samples).await;
                                    }
                                }
                            }
                            "response.audio_transcript.delta" => {
                                if let Some(delta) = event["delta"].as_str() {
                                    let _ = app_clone.emit("qwen_transcript", delta);
                                }
                            }
                            "response.audio_transcript.done" => {
                                if let Some(transcript) = event["transcript"].as_str() {
                                    let _ = app_clone.emit("qwen_response", transcript);
                                }
                            }
                            "conversation.item.input_audio_transcription.completed" => {
                                if let Some(transcript) = event["transcript"].as_str() {
                                    let _ = app_clone.emit("user_transcript", transcript);
                                }
                            }
                            "input_audio_buffer.speech_started" => {
                                let _ = app_clone.emit("qwen_state", "listening");
                            }
                            "response.audio.done" => {
                                let _ = app_clone.emit("qwen_state", "speaking");
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }
    });

    loop {
        if !state_clone.lock().await.in_call.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let _ = write.close().await;
    let _ = reader.await;

    Ok(())
}
