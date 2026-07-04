use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use livekit_wakeword::SAMPLE_RATE;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

const CHUNK_DURATION_MS: u64 = 100;
const CHUNK_SAMPLES: usize = (SAMPLE_RATE as u64 * CHUNK_DURATION_MS / 1000) as usize;

pub struct MicStream {
    rx: mpsc::Receiver<Vec<i16>>,
}

pub fn open_mic() -> Result<MicStream, Box<dyn std::error::Error + Send + Sync>> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("No default input device")?;

    eprintln!("mic device: {}", device.name().unwrap_or("unknown".into()));

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(SAMPLE_RATE as u32),
        buffer_size: cpal::BufferSize::Default,
    };

    let (tx, rx) = mpsc::channel(32);

    let mut buffer: Vec<i16> = Vec::with_capacity(CHUNK_SAMPLES);

    let err_fn = move |err| {
        eprintln!("cpal stream error: {err}");
    };

    let stream = device.build_input_stream(
        &config,
        move |data: &[i16], _: &cpal::InputCallbackInfo| {
            buffer.extend_from_slice(data);
            if buffer.len() >= CHUNK_SAMPLES {
                let chunk = buffer.drain(..CHUNK_SAMPLES).collect::<Vec<i16>>();
                let _ = tx.blocking_send(chunk);
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    // Leak the stream so it lives for the process lifetime
    // (cpal::Stream is not Send, can't be stored in async structs)
    std::mem::forget(stream);

    Ok(MicStream { rx })
}

pub async fn read_chunk(
    mic: &Arc<Mutex<MicStream>>,
) -> Result<Vec<i16>, Box<dyn std::error::Error + Send + Sync>> {
    let mut m = mic.lock().await;
    m.rx
        .recv()
        .await
        .ok_or("Microphone stream closed")
        .map_err(|e| e.into())
}

pub fn sample_rate() -> u32 {
    SAMPLE_RATE as u32
}
