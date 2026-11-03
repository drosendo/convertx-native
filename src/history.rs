use crate::settings::config_dir;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp: i64,
    pub input: String,
    pub output: String,
    pub converter: String,
    pub from: String,
    pub to: String,
    pub ok: bool,
    pub message: String,
    pub duration_ms: u64,
}

impl HistoryEntry {
    pub fn when(&self) -> String {
        DateTime::from_timestamp(self.timestamp, 0)
            .map(|utc| {
                utc.with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "—".into())
    }
}

fn path() -> PathBuf {
    config_dir().join("history.json")
}

pub fn load() -> Vec<HistoryEntry> {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(entries: &[HistoryEntry]) {
    let file = path();
    if let Some(parent) = file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(entries) {
        let _ = std::fs::write(file, text);
    }
}

pub fn push(entries: &mut Vec<HistoryEntry>, entry: HistoryEntry) {
    entries.insert(0, entry);
    entries.truncate(MAX_ENTRIES);
    save(entries);
}

pub fn clear(entries: &mut Vec<HistoryEntry>) {
    entries.clear();
    save(entries);
}
