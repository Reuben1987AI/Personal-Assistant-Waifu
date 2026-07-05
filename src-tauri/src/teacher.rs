use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

// Embedded default curriculum. On first run we write it to app_data_dir/
// curriculum.json so the user can edit it in place; subsequent runs load
// the user's copy. `include_str!` keeps dev mode (where Tauri's resource
// resolver may not find resources) working without extra wiring.
const DEFAULT_CURRICULUM: &str = include_str!("../resources/curriculum.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurriculumEntry {
    pub hanzi: String,
    pub pinyin: String,
    pub en: String,
    pub category: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Learn,
    PracticeOne,
    PracticeAll,
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Idle
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Progress {
    pub learned: Vec<String>,
    pub position: usize,
    pub phase: Phase,
    pub target_hanzi: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetSnapshot {
    pub hanzi: String,
    pub pinyin: String,
    pub en: String,
    pub category: String,
}

impl From<&CurriculumEntry> for TargetSnapshot {
    fn from(e: &CurriculumEntry) -> Self {
        Self {
            hanzi: e.hanzi.clone(),
            pinyin: e.pinyin.clone(),
            en: e.en.clone(),
            category: e.category.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TeacherSnapshot {
    pub active: bool,
    pub phase: &'static str,
    pub target: Option<TargetSnapshot>,
    pub learned: Vec<String>,
    pub position: usize,
    pub total: usize,
}

pub struct TeacherState {
    pub active: bool,
    pub curriculum: Vec<CurriculumEntry>,
    pub progress: Progress,
    pub data_dir: Option<PathBuf>,
}

impl Default for TeacherState {
    fn default() -> Self {
        Self {
            active: false,
            curriculum: Vec::new(),
            progress: Progress::default(),
            data_dir: None,
        }
    }
}

impl TeacherState {
    /// Load curriculum + progress from app_data_dir. Seeds curriculum.json
    /// from the embedded default if missing; creates an empty progress.json
    /// if missing. Called once from main.rs setup() after Tauri owns the app
    /// handle so we have access to app_data_dir.
    pub fn init(&mut self, data_dir: PathBuf) -> Result<(), String> {
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app_data_dir: {e}"))?;
        self.data_dir = Some(data_dir.clone());

        let cur_path = data_dir.join("curriculum.json");
        if !cur_path.exists() {
            std::fs::write(&cur_path, DEFAULT_CURRICULUM)
                .map_err(|e| format!("seed curriculum.json: {e}"))?;
        }
        let cur_text =
            std::fs::read_to_string(&cur_path).map_err(|e| format!("read curriculum.json: {e}"))?;
        self.curriculum =
            serde_json::from_str(&cur_text).map_err(|e| format!("parse curriculum.json: {e}"))?;

        let prog_path = data_dir.join("progress.json");
        if prog_path.exists() {
            let prog_text = std::fs::read_to_string(&prog_path)
                .map_err(|e| format!("read progress.json: {e}"))?;
            self.progress =
                serde_json::from_str(&prog_text).map_err(|e| format!("parse progress.json: {e}"))?;
        } else {
            self.progress = Progress::default();
            self.save();
        }
        Ok(())
    }

    fn save(&self) {
        if let Some(dir) = &self.data_dir {
            let path = dir.join("progress.json");
            if let Ok(text) = serde_json::to_string_pretty(&self.progress) {
                let _ = std::fs::write(path, text);
            }
        }
    }

    pub fn target_entry(&self) -> Option<&CurriculumEntry> {
        let target = self.progress.target_hanzi.as_ref()?;
        self.curriculum.iter().find(|e| &e.hanzi == target)
    }

    /// Pick the next unlearned word at/after the current cursor. Stops at the
    /// first unlearned word; sets target. Returns None if curriculum exhausted.
    pub fn advance(&mut self) -> Option<&CurriculumEntry> {
        let start = self.progress.position.min(self.curriculum.len());
        for i in start..self.curriculum.len() {
            let h = &self.curriculum[i].hanzi;
            if !self.progress.learned.iter().any(|x| x == h) {
                self.progress.position = i;
                self.progress.target_hanzi = Some(h.clone());
                return Some(&self.curriculum[i]);
            }
        }
        None
    }

    /// Enter teacher mode: flip active, ensure a target is loaded (resumes at
    /// the saved position/target), enter LEARN phase.
    pub fn enter_mode(&mut self) {
        self.active = true;
        if self.progress.target_hanzi.is_none() {
            self.advance();
        }
        self.progress.phase = Phase::Learn;
        self.save();
    }

    /// Exit teacher mode: flip inactive, clear phase. Position/target are
    /// preserved so the next entry resumes where the user left off.
    pub fn exit_mode(&mut self) {
        self.active = false;
        self.progress.phase = Phase::Idle;
        self.save();
    }

    pub fn set_phase(&mut self, phase: Phase, target: Option<String>) {
        self.progress.phase = phase;
        if let Some(t) = target {
            self.progress.target_hanzi = Some(t);
        }
        self.save();
    }

    /// Mark a word as learned (idempotent). If it's the current target,
    /// advance the cursor and pick the next unlearned word, then return to
    /// LEARN phase — "after practicing a specific word we go to learn
    /// another word".
    pub fn mark_word_learned(&mut self, hanzi: &str) {
        if !hanzi.is_empty() && !self.progress.learned.iter().any(|x| x == hanzi) {
            self.progress.learned.push(hanzi.to_string());
        }
        if self.progress.target_hanzi.as_deref() == Some(hanzi) {
            self.progress.position = self.progress.position.saturating_add(1);
        }
        self.advance();
        self.progress.phase = Phase::Learn;
        self.save();
    }

    /// Bare-minimum tutor instructions, baked from the current teacher state.
    /// The LLM self-drives the LEARN → PRACTICE_ONE → LEARN → PRACTICE_ALL
    /// loop and calls set_phase / mark_word_learned to keep the app in sync
    /// (option D). Conciseness is hard-capped at two short sentences per turn;
    /// explanations are forbidden unless explicitly asked.
    pub fn build_instructions(&self) -> String {
        let known: Vec<&str> = self.progress.learned.iter().map(|s| s.as_str()).collect();
        let known_str = if known.is_empty() {
            "(none yet)".to_string()
        } else {
            known.join(", ")
        };

        match self.target_entry() {
            Some(t) => format!(
                "You are Kassandra in CHINESE TEACHER mode. Bare-minimum tutor. Teach one word at a time.\n\
                 Rules: Never explain grammar, etymology, or culture unless the user explicitly asks. \
                 One short clause per turn. Say the target word, give its meaning in one clause, ask \
                 the user to repeat. If the user says the English translation instead of Chinese, \
                 refuse gently and ask them to say it in Chinese. Accept the target Hanzi or its \
                 pinyin (tones optional) as a correct repeat. Never exceed two short sentences per turn. \
                 No preamble, no recaps.\n\
                 Current phase: {phase}. Target word: {hanzi} (pinyin: {pinyin}, en: {en}).\n\
                 Known words: {known}.\n\
                 When the user repeats the target correctly, call mark_word_learned(\"{hanzi}\"), then \
                 call set_phase(\"learn\", <next word hanzi>), then introduce the next word in the same \
                 bare-minimum style. After every few new words, call set_phase(\"practice_all\") and \
                 weave all known words into one short dialogue for the user to respond to; after one \
                 short dialogue exchange return to learn with the next unlearned word.",
                phase = phase_name(self.progress.phase),
                hanzi = t.hanzi,
                pinyin = t.pinyin,
                en = t.en,
                known = known_str,
            ),
            None => format!(
                "You are Kassandra in CHINESE TEACHER mode. The curriculum is exhausted — every word \
                 has been learned. Congratulate the user in one short sentence and ask what they'd \
                 like to practice. Known words: {known}.",
                known = known_str,
            ),
        }
    }

    /// When teacher mode is off (normal assistant chat) we still register
    /// these tools so the LLM can call enter_chinese_teacher_mode() from a
    /// normal conversation. The bare-minimum tutor prompt is only injected
    /// once the mode is on (handled in qwen/client.rs by selection between
    /// the default instructions and `build_instructions()`).
    pub fn build_tools() -> serde_json::Value {
        json!([
            {
                "type": "function",
                "name": "enter_chinese_teacher_mode",
                "description": "Enter CHINESE TEACHER mode. Call when the user asks to learn, practice, or study Chinese.",
                "parameters": {"type": "object", "properties": {}, "required": []}
            },
            {
                "type": "function",
                "name": "exit_chinese_teacher_mode",
                "description": "Exit CHINESE TEACHER mode and return to normal assistant chat. Call when the user asks to stop, leave, or quit Chinese teaching.",
                "parameters": {"type": "object", "properties": {}, "required": []}
            },
            {
                "type": "function",
                "name": "set_phase",
                "description": "Report a transition to a new teaching phase. Call whenever the teaching loop changes phase.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "phase": {"type": "string", "enum": ["learn", "practice_one", "practice_all"]},
                        "target_word": {"type": "string", "description": "Hanzi of the word being taught (omit for practice_all)."}
                    },
                    "required": ["phase"]
                }
            },
            {
                "type": "function",
                "name": "mark_word_learned",
                "description": "Record that the user has mastered a word. Call when the user correctly repeats / practices the target word.",
                "parameters": {
                    "type": "object",
                    "properties": {"hanzi": {"type": "string"}},
                    "required": ["hanzi"]
                }
            }
        ])
    }

    /// Dispatch a function call from the realtime API → mutate teacher state
    /// + persist progress.json. Returns the string to send back as
    /// `function_call_output` so the LLM can continue the turn.
    pub fn handle_tool_call(&mut self, name: &str, args: &serde_json::Value) -> String {
        match name {
            "enter_chinese_teacher_mode" => {
                self.enter_mode();
                let snap = self.snapshot();
                format!(
                    "OK. Teacher mode activated. Phase: {}. Target: {}. Known: {}.",
                    snap.phase,
                    snap.target
                        .map(|t| t.hanzi)
                        .unwrap_or_else(|| "none".to_string()),
                    snap.learned.join(", "),
                )
            }
            "exit_chinese_teacher_mode" => {
                self.exit_mode();
                "OK. Teacher mode disabled.".to_string()
            }
            "set_phase" => {
                let phase = args["phase"].as_str().unwrap_or("learn");
                let target = args["target_word"].as_str().map(|s| s.to_string());
                let p = match phase {
                    "learn" => Phase::Learn,
                    "practice_one" => Phase::PracticeOne,
                    "practice_all" => Phase::PracticeAll,
                    _ => Phase::Learn,
                };
                self.set_phase(p, target);
                format!("OK. Phase now {}.", phase_name(self.progress.phase))
            }
            "mark_word_learned" => {
                let h = args["hanzi"].as_str().unwrap_or("");
                if h.is_empty() {
                    return "ERROR: missing hanzi".to_string();
                }
                self.mark_word_learned(h);
                format!("OK. {} marked learned.", h)
            }
            other => format!("ERROR: unknown tool {other}"),
        }
    }

    /// True if the tool call flipped the active flag — used by qwen/client.rs
    /// to decide whether to send a fresh `session.update` with the teacher
    /// instructions + `language: "zh"` (or strip them on exit).
    pub fn mode_edge(&self, prev_active: bool) -> bool {
        self.active != prev_active
    }

    pub fn snapshot(&self) -> TeacherSnapshot {
        TeacherSnapshot {
            active: self.active,
            phase: phase_name(self.progress.phase),
            target: self.target_entry().map(Into::into),
            learned: self.progress.learned.clone(),
            position: self.progress.position,
            total: self.curriculum.len(),
        }
    }
}

pub fn phase_name(p: Phase) -> &'static str {
    match p {
        Phase::Idle => "idle",
        Phase::Learn => "learn",
        Phase::PracticeOne => "practice_one",
        Phase::PracticeAll => "practice_all",
    }
}