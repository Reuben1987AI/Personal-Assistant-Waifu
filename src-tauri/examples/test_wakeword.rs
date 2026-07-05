// Standalone test: run the trained kassandra.onnx classifier over a real
// audio sample and print per-window scores to diagnose whether the model
// actually recognises the wake word.
//
// Usage (from src-tauri):
//   cargo run --example test_wakeword -- /path/to/audio.pcm [mode]
//
// Modes:
//   slide  (default) — slide a 2s window in 100ms steps across the audio
//                      (padded with silence on both sides). Shows how the
//                      score degrades as the word enters/leaves the window.
//   live           — simulate the energy-gated live path: predict on the
//                    speech segment with 800ms trailing silence removed, then
//                    padded with trailing silence to 2s. Runs once per
//                    detected utterance in the file.
//   isolated       — pad the audio with silence to exactly one 2s window.
//                    Upper bound on what this model can score on this input.
//
// Input: raw s16le mono 16kHz PCM (ffmpeg `-f s16le`).
// Decoded from `kas 1.flac`:
//   ffmpeg -i "kas 1.flac" -ar 16000 -ac 1 -f s16le out.pcm

use livekit_wakeword::WakeWordModel;
use std::path::Path;

const WINDOW: usize = 32000; // 2s @ 16kHz — crate's minimum
const STRIDE: usize = 1600; // 100ms — match the live loop's chunk size
const RMS_SPEECH_THRESHOLD: f32 = 80.0;
const SILENCE_CONFIRM_CHUNKS: usize = 8; // 800ms

fn rms(samples: &[i16]) -> f64 {
    let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum / samples.len().max(1) as f64).sqrt()
}

fn load_pcm(path: &Path) -> Vec<i16> {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| { eprintln!("read error: {e}"); std::process::exit(1); });
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn predict_buf(model: &mut WakeWordModel, buf: &[i16]) -> f32 {
    let scores = model.predict(buf).unwrap_or_else(|e| {
        eprintln!("predict error: {e}");
        std::process::exit(1);
    });
    scores.get("kassandra").copied().unwrap_or(-1.0)
}

fn pad_to_window(src: &[i16]) -> Vec<i16> {
    if src.len() >= WINDOW {
        src[..WINDOW].to_vec()
    } else {
        let mut v = src.to_vec();
        v.extend(std::iter::repeat(0i16).take(WINDOW - v.len()));
        v
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let pcm_path = args.get(1).map(|s| Path::new(s)).unwrap_or_else(|| {
        eprintln!("usage: test_wakeword <file.pcm> [slide|live|isolated]");
        std::process::exit(2);
    });
    let mode = args.get(2).map(|s| s.as_str()).unwrap_or("slide");

    let samples = load_pcm(pcm_path);
    eprintln!(
        "loaded {} samples ({:.3}s) from {}, mode={mode}",
        samples.len(),
        samples.len() as f64 / 16000.0,
        pcm_path.display()
    );

    let mut model = WakeWordModel::new(&[Path::new("models/kassandra.onnx")], 16000)
        .unwrap_or_else(|e| {
            eprintln!("model load error: {e}");
            std::process::exit(1);
        });
    eprintln!("model loaded");

    // Baseline: pure silence
    let silence = vec![0i16; WINDOW];
    let s_score = predict_buf(&mut model, &silence);
    eprintln!("\n[silence baseline]            score={:.4}", s_score);
    eprintln!("==========================================");

    match mode {
        "slide" => {
            let pad_each = WINDOW;
            let mut padded = vec![0i16; pad_each];
            padded.extend_from_slice(&samples);
            padded.extend(std::iter::repeat(0i16).take(pad_each));
            let samples = padded;
            eprintln!(
                "padded with {pad_each} samples silence on each side -> {} total ({:.3}s)",
                samples.len(),
                samples.len() as f64 / 16000.0
            );

            let mut best_score: f32 = 0.0;
            let mut best_offset: usize = 0;

            let mut i = 0;
            while i + WINDOW <= samples.len() {
                let window = &samples[i..i + WINDOW];
                let score = predict_buf(&mut model, window);
                let start_s = i as f64 / 16000.0;
                let end_s = (i + WINDOW) as f64 / 16000.0;
                let bar_len = (score * 50.0) as usize;
                let bar: String = "█".repeat(bar_len);
                eprintln!(
                    "t={:.2}-{:.2}s rms={:6.1} score={:.4} {}",
                    start_s, end_s, rms(window), score, bar
                );
                if score > best_score {
                    best_score = score;
                    best_offset = i;
                }
                i += STRIDE;
            }

            eprintln!("==========================================");
            eprintln!(
                "best score {:.4} at offset {:.2}s",
                best_score,
                best_offset as f64 / 16000.0
            );
        }
        "live" => {
            // Mirror the energy-gated live path: chunk by chunk, buffer speech
            // + 800ms trailing silence, then predict on (speech - 800ms tail)
            // padded to 2s. Emits one predict per detected utterance.
            // Pad input with 2s of trailing silence so the trailing-silence
            // confirmation can complete even on short source files.
            let mut samples = samples.clone();
            samples.extend(std::iter::repeat(0i16).take(WINDOW));
            let mut speech_buf: Vec<i16> = Vec::with_capacity(WINDOW + 1600);
            let mut in_speech = false;
            let mut silence_run: usize = 0;
            let mut utterance: usize = 0;
            let mut max_chunk_rms: f32 = 0.0;

            for (i, chunk) in samples.chunks(STRIDE).enumerate() {
                let chunk_rms = rms(chunk) as f32;
                if chunk_rms > max_chunk_rms {
                    max_chunk_rms = chunk_rms;
                }
                if chunk_rms > RMS_SPEECH_THRESHOLD {
                    if !in_speech {
                        eprintln!("  chunk #{i:3} speech start rms={chunk_rms:6.1}");
                    }
                    in_speech = true;
                    silence_run = 0;
                    speech_buf.extend_from_slice(chunk);
                    if speech_buf.len() > WINDOW {
                        speech_buf.drain(..speech_buf.len() - WINDOW);
                    }
                } else if in_speech {
                    silence_run += 1;
                    speech_buf.extend_from_slice(chunk);
                    if speech_buf.len() > WINDOW {
                        speech_buf.drain(..speech_buf.len() - WINDOW);
                    }
                    if silence_run >= SILENCE_CONFIRM_CHUNKS {
                        in_speech = false;
                        utterance += 1;
                        let tail = SILENCE_CONFIRM_CHUNKS * STRIDE;
                        let speech_end = speech_buf.len().saturating_sub(tail);
                        let speech_samples = speech_end;

                        // Final framings:
                        //  A) lead=800ms + full speech segment (matches main.rs)
                        //  B) lead=800ms + truncated to last 1.2s of speech
                        //  C) baseline: lead=0 (current broken live behaviour)
                        const LEAD_MS: usize = 800;
                        const LEAD_SAMPLES: usize = LEAD_MS * 16; // 12800
                        const MAX_SPEECH: usize = 19200; // 1.2s — leaves room for lead in 2s window

                        let pb_a = {
                            let mut v = vec![0i16; LEAD_SAMPLES];
                            v.extend_from_slice(&speech_buf[..speech_end]);
                            pad_to_window(&v)
                        };
                        let pb_b = {
                            let start = speech_end.saturating_sub(MAX_SPEECH);
                            let mut v = vec![0i16; LEAD_SAMPLES];
                            v.extend_from_slice(&speech_buf[start..speech_end]);
                            pad_to_window(&v)
                        };
                        let pb_c = pad_to_window(&speech_buf[..speech_end]);

                        let score_a = predict_buf(&mut model, &pb_a);
                        let score_b = predict_buf(&mut model, &pb_b);
                        let score_c = predict_buf(&mut model, &pb_c);
                        eprintln!(
                            "  utterance #{utterance} (speech {speech_samples} samples = {:.1}s):",
                            speech_samples as f64 / 16000.0
                        );
                        eprintln!("    A) lead=800ms + full speech:  score={:.4}", score_a);
                        eprintln!("    B) lead=800ms + last 1.2s:   score={:.4}", score_b);
                        eprintln!("    C) lead=0ms  (old live):     score={:.4}", score_c);
                        speech_buf.clear();
                        silence_run = 0;
                    }
                }
            }
            eprintln!("max chunk RMS seen in file: {max_chunk_rms:.1}");
        }
        "isolated" => {
            let buf = pad_to_window(&samples);
            let score = predict_buf(&mut model, &buf);
            eprintln!(
                "isolated: score={:.4} ({} samples, padded with trailing silence to {WINDOW})",
                score,
                samples.len()
            );
        }
        other => {
            eprintln!("unknown mode: {other}");
            std::process::exit(2);
        }
    }
}