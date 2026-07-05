use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use livekit_wakeword::WakeWordModel;

const WINDOW: usize = 32000;
const WARMUP_ITERS: usize = 5;
const BENCH_ITERS: usize = 50;

fn pad_to_window(src: &[i16]) -> Vec<i16> {
    if src.len() >= WINDOW {
        src[..WINDOW].to_vec()
    } else {
        let mut v = src.to_vec();
        v.extend(std::iter::repeat(0i16).take(WINDOW - v.len()));
        v
    }
}

fn load_pcm(path: &Path) -> Vec<i16> {
    let bytes = std::fs::read(path)
        .unwrap_or_else(|e| {
            eprintln!("read error reading {}: {e}", path.display());
            std::process::exit(1);
        });
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: wakeword-bench <model.onnx> <audio.pcm>");
        eprintln!("  model.onnx  — kassandra classifier ONNX file");
        eprintln!("  audio.pcm   — raw s16le mono 16kHz PCM (any length; padded/truncated to 2s)");
        std::process::exit(2);
    }

    let model_path = PathBuf::from(&args[1]);
    let audio_path = PathBuf::from(&args[2]);

    eprintln!("=== wakeword-native bench (native onnxruntime) ===");
    eprintln!("model: {}", model_path.display());
    eprintln!("audio: {} ({:?})", audio_path.display(), {
        let m = std::fs::metadata(&audio_path).unwrap();
        m.len()
    });
    eprintln!();

    let samples = load_pcm(&audio_path);
    let buf = pad_to_window(&samples);
    let silence = vec![0i16; WINDOW];
    let has_audio = samples.iter().any(|&s| s.abs() > 100);

    eprintln!("loading model...");
    let t0 = Instant::now();
    let mut model = WakeWordModel::new(&[model_path], 16000).unwrap_or_else(|e| {
        eprintln!("model load error: {e}");
        std::process::exit(1);
    });
    let load_time = t0.elapsed();
    eprintln!("model loaded in {load_time:.1?}");
    eprintln!();

    // --- warmup ---
    eprintln!("warmup ({WARMUP_ITERS} iters, silence)...");
    for i in 0..WARMUP_ITERS {
        let _ = model.predict(&silence).unwrap();
        eprint!(
            "\r  warmup {}/{WARMUP_ITERS}",
            i + 1
        );
    }
    eprintln!();
    eprintln!();

    // --- bench: silence ---
    eprintln!("bench: silence ({BENCH_ITERS} iters)...");
    let mut silence_times: Vec<Duration> = Vec::with_capacity(BENCH_ITERS);
    for i in 0..BENCH_ITERS {
        let t0 = Instant::now();
        let _ = model.predict(&silence).unwrap();
        silence_times.push(t0.elapsed());
        eprint!("\r  {}/{BENCH_ITERS}", i + 1);
    }
    eprintln!();
    print_stats("silence predict", &silence_times);

    // --- bench: audio ---
    eprintln!("bench: audio ({BENCH_ITERS} iters)...");
    let mut audio_times: Vec<Duration> = Vec::with_capacity(BENCH_ITERS);
    for i in 0..BENCH_ITERS {
        let t0 = Instant::now();
        let scores = model.predict(&buf).unwrap();
        audio_times.push(t0.elapsed());
        if i == 0 {
            let score = scores.get("kassandra").copied().unwrap_or(-1.0);
            eprintln!("  first-predict score: {score:.4}");
        }
        eprint!("\r  {}/{BENCH_ITERS}", i + 1);
    }
    eprintln!();
    print_stats("audio predict", &audio_times);

    // --- summary ---
    eprintln!("============= SUMMARY =============");
    if has_audio {
        let silence_score = model.predict(&silence).unwrap();
        let audio_score = model.predict(&buf).unwrap();
        eprintln!(
            "silence score: {:.4}",
            silence_score.get("kassandra").copied().unwrap_or(-1.0)
        );
        eprintln!(
            "audio score:   {:.4}",
            audio_score.get("kassandra").copied().unwrap_or(-1.0)
        );
    }
    eprintln!("model load:    {:>8.1} ms", load_time.as_secs_f64() * 1000.0);
    eprintln!(
        "predict mean:  {:>8.1} ms (audio)",
        mean(&audio_times).as_secs_f64() * 1000.0
    );
    eprintln!(
        "predict p95:   {:>8.1} ms (audio)",
        p95(&audio_times).as_secs_f64() * 1000.0
    );
    eprintln!(
        "predict min:   {:>8.1} ms (audio)",
        audio_times.iter().min().unwrap().as_secs_f64() * 1000.0
    );
    eprintln!(
        "predict max:   {:>8.1} ms (audio)",
        audio_times.iter().max().unwrap().as_secs_f64() * 1000.0
    );
}

fn mean(times: &[Duration]) -> Duration {
    let total_ns: u128 = times.iter().map(|d| d.as_nanos()).sum();
    Duration::from_nanos((total_ns / times.len() as u128) as u64)
}

fn p95(times: &[Duration]) -> Duration {
    let mut sorted: Vec<u128> = times.iter().map(|d| d.as_nanos()).collect();
    sorted.sort_unstable();
    let idx = ((times.len() as f64) * 0.95).ceil() as usize - 1;
    Duration::from_nanos(sorted[idx.min(times.len() - 1)] as u64)
}

fn print_stats(label: &str, times: &[Duration]) {
    let min = times.iter().min().unwrap();
    let max = times.iter().max().unwrap();
    let avg = mean(times);
    let p = p95(times);
    println!(
        "  {label}: min={:.1}ms  max={:.1}ms  mean={:.1}ms  p95={:.1}ms",
        min.as_secs_f64() * 1000.0,
        max.as_secs_f64() * 1000.0,
        avg.as_secs_f64() * 1000.0,
        p.as_secs_f64() * 1000.0
    );
}
