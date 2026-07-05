// Probe: does qwen3.5-omni-plus-realtime accept `tools` and a `language`
// hint on `input_audio_transcription` in a `session.update`?
//
// We don't trust the JS-rendered DashScope docs. We open a ws, send a
// session.update carrying a function tool + a zh transcription hint, and
// print every server event for a few seconds. If the server rejects either
// field it emits an `error` event. The outcome decides whether the
// chinese-teacher-mode design can use tool calls (option B/D) or must fall
// back to transcript pattern-matching (option A).
//
// Usage (from src-tauri/):
//   cargo run --example probe_realtime_tools
//   cargo run --example probe_realtime_tools -- qwen3.5-omni-plus-realtime
//
// Reads DASHSCOPE_API_KEY / QWEN_REGION / QWEN_WORKSPACE_ID from ../.env.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const DEFAULT_PROBE_MODEL: &str = "qwen3.5-omni-plus-realtime";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env from project root (one level up from src-tauri/).
    let _ = dotenvy::from_path("../.env");
    let _ = dotenvy::dotenv();

    let api_key = std::env::var("DASHSCOPE_API_KEY")
        .unwrap_or_else(|_| "sk-xxx".to_string());
    let region = std::env::var("QWEN_REGION").unwrap_or_else(|_| "intl".to_string());

    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_PROBE_MODEL.to_string());

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

    eprintln!("probe: connecting to {url} (model={model}, region={region})");

    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {api_key}").parse().unwrap());

    let (ws_stream, _resp) = connect_async(request).await?;
    eprintln!("probe: ws connected (HTTP {})", _resp.status());
    let (mut write, mut read) = ws_stream.split();

    // --- Phase 1: session.update WITH `tools` + zh transcription hint ---
    // OpenAI-realtime-style function tool. If the server doesn't know `tools`
    // we expect an `error` event back. We also add `language: "zh"` to
    // `input_audio_transcription` to test whether paraformer can be pinned
    // to Chinese (the strongest anti-cheat for the teacher mode).
    let session_update_tools = json!({
        "event_id": "probe_001",
        "type": "session.update",
        "session": {
            "modalities": ["text", "audio"],
            "voice": "Tina",
            "input_audio_format": "pcm_16000hz_mono_16bit",
            "output_audio_format": "pcm_24000hz_mono_16bit",
            "input_audio_transcription": {
                "model": "paraformer-realtime-v2",
                "language": "zh"
            },
            "instructions": "You are Kassandra. Respond briefly in one short sentence.",
            "turn_detection": {
                "type": "semantic_vad",
                "threshold": 0.5,
                "silence_duration_ms": 800
            },
            "tools": [
                {
                    "type": "function",
                    "name": "enter_chinese_teacher_mode",
                    "description": "Enter chinese teacher mode. Call when the user asks to learn / practice Chinese.",
                    "parameters": {
                        "type": "object",
                        "properties": {},
                        "required": []
                    }
                },
                {
                    "type": "function",
                    "name": "set_phase",
                    "description": "Report transition to a new teaching phase.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "phase": {
                                "type": "string",
                                "enum": ["learn", "practice_one", "practice_all"]
                            },
                            "target_word": { "type": "string" }
                        },
                        "required": ["phase"]
                    }
                },
                {
                    "type": "function",
                    "name": "mark_word_learned",
                    "description": "Record that the user has mastered a word.",
                    "parameters": {
                        "type": "object",
                        "properties": { "hanzi": { "type": "string" } },
                        "required": ["hanzi"]
                    }
                },
                {
                    "type": "function",
                    "name": "exit_chinese_teacher_mode",
                    "description": "Exit chinese teacher mode.",
                    "parameters": { "type": "object", "properties": {}, "required": [] }
                }
            ]
        }
    });

    eprintln!("probe: sending session.update WITH tools + language=zh");
    eprintln!("probe: payload = {}", session_update_tools);
    write
        .send(Message::Text(session_update_tools.to_string().into()))
        .await?;

    drain_events(&mut read, Duration::from_secs(6), "PHASE1").await;

    // --- Phase 2: session.update WITHOUT tools, WITHOUT language hint ---
    // Control: confirm the session.update itself works on this model with
    // the baseline shape the app uses today, so we can distinguish "model
    // rejects tools specifically" from "model rejects this whole shape".
    let session_update_baseline = json!({
        "event_id": "probe_002",
        "type": "session.update",
        "session": {
            "modalities": ["text", "audio"],
            "voice": "Tina",
            "input_audio_format": "pcm_16000hz_mono_16bit",
            "output_audio_format": "pcm_24000hz_mono_16bit",
            "input_audio_transcription": {
                "model": "paraformer-realtime-v2"
            },
            "instructions": "You are Kassandra. Respond briefly.",
            "turn_detection": {
                "type": "semantic_vad",
                "threshold": 0.5,
                "silence_duration_ms": 800
            }
        }
    });

    eprintln!("probe: sending baseline session.update (no tools, no language)");
    write
        .send(Message::Text(session_update_baseline.to_string().into()))
        .await?;

    drain_events(&mut read, Duration::from_secs(6), "PHASE2").await;

    let _ = write.close().await;
    Ok(())
}

/// Read every text message for up to `dur`, print each, and summarize which
/// event `type`s appeared. Records whether any `error` event fired, which is
/// the signal the probe is looking for.
async fn drain_events<S>(read: &mut S, dur: Duration, label: &str)
where
    S: futures_util::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    use std::collections::BTreeSet;
    let mut seen_types: BTreeSet<String> = BTreeSet::new();
    let mut saw_error: Option<String> = None;
    let mut n = 0usize;

    let deadline = tokio::time::Instant::now() + dur;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, read.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                n += 1;
                let parsed: serde_json::Value = serde_json::from_str(&text)
                    .unwrap_or(serde_json::Value::Null);
                let t = parsed["type"].as_str().unwrap_or("<no type>").to_string();
                if t == "error" {
                    let detail = parsed["error"]
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("")
                        .to_string();
                    saw_error = Some(detail);
                }
                eprintln!("[{label} #{n}] type={t}: {text}");
                seen_types.insert(t);
            }
            Ok(Some(Ok(Message::Close(c)))) => {
                eprintln!("[{label}] ws closed by server: {c:?}");
                break;
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(e))) => {
                eprintln!("[{label}] ws error: {e}");
                break;
            }
            Ok(None) => {
                eprintln!("[{label}] stream ended");
                break;
            }
            Err(_) => break, // timeout
        }
    }

    eprintln!("----- {label} summary -----");
    eprintln!("  events received: {n}");
    eprintln!("  event types:    {:?}", seen_types);
    match &saw_error {
        Some(detail) => eprintln!("  ERROR fired:     {detail}"),
        None => eprintln!("  no error event fired"),
    }
    eprintln!("---------------------");
    // Touch BASE64 so unused-import lint stays happy even if code paths
    // above never encode anything.
    let _ = BASE64.encode(b"");
}