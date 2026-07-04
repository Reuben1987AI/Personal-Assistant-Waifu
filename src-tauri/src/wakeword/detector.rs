use livekit_wakeword::{WakeWordModel, WakeWordError};
use std::path::Path;

pub fn init_detector() -> Result<WakeWordModel, WakeWordError> {
    let model_path = Path::new("models/kassandra.onnx");

    if !model_path.exists() {
        return Err(WakeWordError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "kassandra.onnx not found. Train it with: make train-wakeword WORD=kassandra",
        )));
    }

    WakeWordModel::new(&[model_path], livekit_wakeword::SAMPLE_RATE as u32)
}
