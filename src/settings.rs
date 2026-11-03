use crate::convert::ConvertOptions;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Set on Android, where the app cannot see a normal home directory.
static BASE_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_base_dir(path: PathBuf) {
    let _ = BASE_DIR.set(path);
}

fn base_dir() -> Option<&'static PathBuf> {
    BASE_DIR.get()
}

pub fn home_dir() -> PathBuf {
    if let Some(base) = base_dir() {
        return base.clone();
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// `~/Library/Application Support/ConvertX` on macOS, `%APPDATA%\ConvertX` on
/// Windows and `~/.config/convertx` elsewhere.
pub fn config_dir() -> PathBuf {
    if let Some(base) = base_dir() {
        return base.join("config");
    }
    if cfg!(target_os = "macos") {
        home_dir().join("Library/Application Support/ConvertX")
    } else if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join("AppData/Roaming"))
            .join("ConvertX")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"))
            .join("convertx")
    }
}

pub fn default_output_dir() -> PathBuf {
    if let Some(base) = base_dir() {
        return base.join("ConvertX");
    }
    let downloads = home_dir().join("Downloads");
    if downloads.is_dir() {
        downloads.join("ConvertX")
    } else {
        home_dir().join("ConvertX")
    }
}

fn default_parallel_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(2)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub output_dir: String,
    pub parallel_jobs: usize,
    pub ffmpeg_input_args: String,
    pub ffmpeg_output_args: String,
    pub svg_scale: f32,
    pub jpeg_quality: u8,
    /// Kill a converter that runs longer than this. Zero means no limit.
    pub convert_timeout_secs: u64,
    /// Ask GitHub for a newer release on startup.
    pub check_updates: bool,
    pub open_output_after: bool,
    pub record_history: bool,
    pub last_target: String,
    /// Binary name -> absolute path chosen by the user.
    pub custom_paths: std::collections::BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            output_dir: default_output_dir().to_string_lossy().into_owned(),
            parallel_jobs: default_parallel_jobs(),
            ffmpeg_input_args: String::new(),
            ffmpeg_output_args: String::new(),
            svg_scale: 1.0,
            jpeg_quality: 90,
            convert_timeout_secs: 1800,
            check_updates: true,
            open_output_after: false,
            record_history: true,
            last_target: String::new(),
            custom_paths: Default::default(),
        }
    }
}

impl Settings {
    fn path() -> PathBuf {
        config_dir().join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn output_path(&self) -> PathBuf {
        let dir = PathBuf::from(&self.output_dir);
        if dir.as_os_str().is_empty() {
            default_output_dir()
        } else {
            dir
        }
    }

    pub fn convert_options(&self) -> ConvertOptions {
        ConvertOptions {
            svg_scale: self.svg_scale,
            jpeg_quality: self.jpeg_quality,
            ffmpeg_input_args: self.ffmpeg_input_args.clone(),
            ffmpeg_output_args: self.ffmpeg_output_args.clone(),
            timeout_secs: self.convert_timeout_secs,
        }
    }
}
