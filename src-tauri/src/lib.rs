pub mod audio;
pub mod qwen;
pub mod teacher;
pub mod wakeword;

use std::sync::atomic::AtomicBool;

use crate::teacher::TeacherState;

pub struct AppState {
    pub muted: AtomicBool,
    pub in_call: AtomicBool,
    pub wakeword_active: AtomicBool,
    pub teacher: TeacherState,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            muted: AtomicBool::new(false),
            in_call: AtomicBool::new(false),
            wakeword_active: AtomicBool::new(false),
            teacher: TeacherState::default(),
        }
    }
}