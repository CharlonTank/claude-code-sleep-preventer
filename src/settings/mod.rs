//! Application settings with JSON persistence

pub mod window;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Sleep prevention settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepPreventionSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for SleepPreventionSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Speech-to-text settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechToTextSettings {
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default)]
    pub vocabulary_words: Vec<String>,
}

impl Default for SpeechToTextSettings {
    fn default() -> Self {
        Self {
            language: "auto".to_string(),
            vocabulary_words: vec!["Claude".to_string(), "Anthropic".to_string()],
        }
    }
}

/// Application settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub sleep_prevention: SleepPreventionSettings,
    #[serde(default)]
    pub speech_to_text: SpeechToTextSettings,
}

fn default_true() -> bool {
    true
}

fn default_language() -> String {
    "auto".to_string()
}

impl AppSettings {
    /// Get the settings file path
    pub fn settings_path() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("ClaudeSleepPreventer")
            .join("settings.json")
    }

    /// Load settings from disk, returning defaults if file doesn't exist or is invalid
    pub fn load() -> Self {
        let path = Self::settings_path();
        if !path.exists() {
            return Self::default();
        }

        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::settings_path();

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create settings directory: {}", e))?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;

        fs::write(&path, content).map_err(|e| format!("Failed to write settings: {}", e))
    }

    /// Get the list of supported languages for speech-to-text
    pub fn supported_languages() -> Vec<(&'static str, &'static str)> {
        vec![
            ("auto", "Auto-detect"),
            ("en", "English"),
            ("fr", "French"),
            ("de", "German"),
            ("es", "Spanish"),
            ("it", "Italian"),
            ("pt", "Portuguese"),
            ("zh", "Chinese"),
            ("ja", "Japanese"),
            ("ko", "Korean"),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = AppSettings::default();
        assert!(settings.sleep_prevention.enabled);
        assert_eq!(settings.speech_to_text.language, "auto");
        assert!(settings.speech_to_text.vocabulary_words.contains(&"Claude".to_string()));
    }

    #[test]
    fn test_deserialize_partial() {
        let json = r#"{"sleep_prevention": {"enabled": false}}"#;
        let settings: AppSettings = serde_json::from_str(json).unwrap();
        assert!(!settings.sleep_prevention.enabled);
        // speech_to_text should have defaults
        assert_eq!(settings.speech_to_text.language, "auto");
    }
}
