use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::audio::{MicStream, Speaker};
use crate::AppState;

const DEFAULT_MODEL: &str = "qwen3.5-omni-flash-realtime";
const DEFAULT_VOICE: &str = "Tina";
const DEFAULT_INSTRUCTIONS: &str =
    "You are Kassandra, a personal AI assistant. Be warm, witty, and concise.";

pub async fn run_call(
    state: &Arc<Mutex<AppState>>,
    app: &AppHandle,
    mic: &Arc<Mutex<MicStream>>,
    speaker: &Speaker,
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

    let _ = app.emit("qwen_state", "connected");

    let aec = match crate::audio::Aec::new() {
        Ok(a) => {
            eprintln!("AEC enabled (WebRTC AEC3)");
            Some(Arc::new(std::sync::Mutex::new(a)))
        }
        Err(e) => {
            eprintln!("AEC unavailable, continuing without echo cancellation: {e}");
            None
        }
    };

    let app_clone = app.clone();
    let speaker_clone = speaker.clone();
    let aec_clone = aec.clone();
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

            if write
                .send(Message::Text(audio_msg.to_string().into()))
                .await
                .is_err()
            {
                eprintln!("ws send failed");
                break;
            }
        } else {
            eprintln!("no mic chunk, breaking");
            break;
        }
    }

    let _ = write.close().await;
    let _ = reader.await;

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
