//! Acoustic Echo Cancellation (AEC) via WebRTC AEC3.
//!
//! This is **Path 1: in-app AEC**. We run WebRTC's AudioProcessing module
//! (AEC3 + noise suppression + high-pass filter) inside the app. The mic
//! capture signal is cleaned of echo using the speaker output as a reference
//! ("render") signal. This enables true full-duplex: the user can barge in
//! and interrupt Kassandra mid-sentence, because their voice is separated from
//! Kassandra's echo.
//!
//! ## Alternative — Path 2: PipeWire `module-echo-cancel`
//!
//! If you deploy on a Linux system where you control the host PipeWire
//! configuration, you can get the **same WebRTC AEC3** with zero app code by
//! letting PipeWire run the echo canceller at the audio-server layer:
//!
//! 1. Create `~/.config/pipewire/pipewire.conf.d/60-echo-cancel.conf`:
//!    ```text
//!    context.modules = [
//!      { name = libpipewire-module-echo-cancel
//!        args = {
//!          capture.props  = { node.name = "Echo Cancellation Capture" }
//!          source.props   = { node.name = "Echo Cancellation Source" }
//!          sink.props     = { node.name = "Echo Cancellation Sink" }
//!          playback.props = { node.name = "Echo Cancellation Playback" }
//!        }
//!      }
//!    ]
//!    ```
//! 2. Restart PipeWire: `systemctl --user restart pipewire`
//! 3. Point cpal (or `docker/asound.conf`) at the virtual
//!    `Echo Cancellation Source` for input and `Echo Cancellation Sink` for
//!    output.
//!
//! PipeWire correlates the sink and capture streams internally — no reference
//! signal plumbing needed. Use Path 2 when shipping to a single controlled
//! Linux host (the AEC runs in the server that already "knows about" all
//! playback). Use Path 1 (this module) when shipping binaries to systems whose
//! PipeWire you don't control.

use std::collections::VecDeque;
use std::sync::Arc;
use webrtc_audio_processing::Processor;
use webrtc_audio_processing::config::{Config, EchoCanceller, HighPassFilter, NoiseSuppression};

const AEC_SAMPLE_RATE: u32 = 16_000;
const FRAME_SAMPLES: usize = AEC_SAMPLE_RATE as usize / 100;

struct LinearResampler {
    input_rate: u32,
    output_rate: u32,
    carry: Vec<f32>,
    phase: f64,
}

impl LinearResampler {
    fn new(input_rate: u32, output_rate: u32) -> Self {
        Self { input_rate, output_rate, carry: Vec::new(), phase: 0.0 }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut combined = std::mem::take(&mut self.carry);
        combined.extend_from_slice(input);

        let step = self.input_rate as f64 / self.output_rate as f64;
        let mut out = Vec::with_capacity((input.len() as f64 / step) as usize + 2);
        let mut pos = self.phase;

        while combined.len() >= 2 && pos < (combined.len() - 1) as f64 {
            let i = pos as usize;
            let frac = (pos - i as f64) as f32;
            let s0 = combined[i];
            let s1 = combined[i + 1];
            out.push(s0 + (s1 - s0) * frac);
            pos += step;
        }

        let consumed = pos.floor() as usize;
        self.carry = combined[consumed..].to_vec();
        self.phase = pos - consumed as f64;
        out
    }
}

pub struct Aec {
    processor: Arc<Processor>,
    render_buffer: VecDeque<f32>,
    resampler: LinearResampler,
}

impl Aec {
    pub fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let processor = Processor::new(AEC_SAMPLE_RATE)?;
        processor.set_config(Config {
            echo_canceller: Some(EchoCanceller::Full { stream_delay_ms: None }),
            high_pass_filter: Some(HighPassFilter::default()),
            noise_suppression: Some(NoiseSuppression::default()),
            ..Default::default()
        });
        eprintln!("AEC: WebRTC AEC3 initialized at {}Hz", AEC_SAMPLE_RATE);
        Ok(Self {
            processor: Arc::new(processor),
            render_buffer: VecDeque::new(),
            resampler: LinearResampler::new(24_000, AEC_SAMPLE_RATE),
        })
    }

    pub fn push_render(&mut self, samples: &[i16]) {
        if samples.is_empty() {
            return;
        }
        let input: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
        let resampled = self.resampler.process(&input);
        self.render_buffer.extend(resampled);
    }

    /// Drop any queued render samples. Call at the end of a Qwen session so
    /// the next wake-loop `process_capture` doesn't feed stale Kassandra audio
    /// into AEC3 as a fake echo reference (which would distort the wake
    /// stream after the call ends).
    pub fn clear_render(&mut self) {
        self.render_buffer.clear();
    }

    pub fn process_capture(&mut self, mic: &[i16]) -> Vec<i16> {
        if mic.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(mic.len());
        let mut render_frame = vec![0.0f32; FRAME_SAMPLES];
        let mut capture_frame = vec![0.0f32; FRAME_SAMPLES];

        for chunk in mic.chunks(FRAME_SAMPLES) {
            for r in render_frame.iter_mut() {
                *r = self.render_buffer.pop_front().unwrap_or(0.0);
            }
            for (i, r) in capture_frame.iter_mut().enumerate() {
                *r = chunk.get(i).map(|&s| s as f32 / 32768.0).unwrap_or(0.0);
            }

            let mut render_arr = [render_frame.as_mut_slice()];
            if let Err(e) = self.processor.process_render_frame(&mut render_arr) {
                eprintln!("AEC render error: {e}");
            }
            let mut capture_arr = [capture_frame.as_mut_slice()];
            if let Err(e) = self.processor.process_capture_frame(&mut capture_arr) {
                eprintln!("AEC capture error: {e}");
            }

            out.extend(capture_frame.iter().map(|&s| {
                (s * 32768.0).clamp(-32768.0, 32767.0) as i16
            }));
        }
        out
    }
}
