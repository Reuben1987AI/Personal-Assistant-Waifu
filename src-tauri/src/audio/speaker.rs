use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

const OUTPUT_SAMPLE_RATE: u32 = 24000;

pub struct Speaker {
    queue: Arc<Mutex<VecDeque<i16>>>,
}

impl Clone for Speaker {
    fn clone(&self) -> Self {
        Self {
            queue: self.queue.clone(),
        }
    }
}

impl Speaker {
    pub fn dummy() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn push_chunk(&self, samples: &[i16]) {
        let mut q = self.queue.lock().await;
        q.extend(samples.iter().copied());
    }

    pub async fn clear(&self) {
        let mut q = self.queue.lock().await;
        q.clear();
    }
}

pub fn open_speaker() -> Result<Speaker, Box<dyn std::error::Error + Send + Sync>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No default output device")?;

    eprintln!("speaker device: {}", device.name().unwrap_or("unknown".into()));

    let config = cpal::StreamConfig {
        channels: 1,
        sample_rate: cpal::SampleRate(OUTPUT_SAMPLE_RATE),
        buffer_size: cpal::BufferSize::Default,
    };

    let queue: Arc<Mutex<VecDeque<i16>>> = Arc::new(Mutex::new(VecDeque::new()));
    let queue_clone = queue.clone();

    let err_fn = move |err| {
        eprintln!("cpal speaker stream error: {err}");
    };

    let stream = device.build_output_stream(
        &config,
        move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
            let mut q = queue_clone.blocking_lock();
            for sample in data.iter_mut() {
                *sample = q.pop_front().unwrap_or(0);
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    std::mem::forget(stream);

    Ok(Speaker { queue })
}
