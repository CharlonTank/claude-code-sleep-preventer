use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::native_dialogs;

const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin";
const MODEL_FILENAME: &str = "ggml-medium.bin";

#[derive(Debug, Clone, PartialEq)]
pub enum DictationSetupStatus {
    Ready,
    MissingModel,
}

pub struct WhisperTranscriber {
    model_path: Option<PathBuf>,
    whisper_path: PathBuf,
}

impl WhisperTranscriber {
    pub fn new() -> Self {
        let whisper_path = Self::find_whisper_cli();
        let model_path = Self::find_model();

        Self {
            model_path,
            whisper_path,
        }
    }

    /// Find whisper-cli: bundled first, then system
    fn find_whisper_cli() -> PathBuf {
        // Try bundled version first (in app's Resources folder)
        if let Some(exe_path) = env::current_exe().ok() {
            let resources = exe_path
                .parent() // MacOS
                .and_then(|p| p.parent()) // Contents
                .map(|p| p.join("Resources").join("whisper-cli"));

            if let Some(bundled) = resources {
                if bundled.exists() {
                    return bundled;
                }
            }
        }

        // Fall back to system whisper-cli
        PathBuf::from("whisper-cli")
    }

    pub fn setup_status(&self) -> DictationSetupStatus {
        if self.model_path.is_some() {
            DictationSetupStatus::Ready
        } else {
            DictationSetupStatus::MissingModel
        }
    }

    /// Get the app support directory for storing models
    fn app_support_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("ClaudeSleepPreventer")
    }

    fn find_model() -> Option<PathBuf> {
        let model_name = env::var("WHISPER_MODEL").unwrap_or_else(|_| "medium".to_string());

        // Check app support directory first (our downloaded models)
        let app_models_dir = Self::app_support_dir().join("models");
        let app_model = app_models_dir.join(format!("ggml-{}.bin", model_name));
        if app_model.exists() {
            return Some(app_model);
        }

        // Check homebrew location (if user had it installed before)
        let homebrew_dir = PathBuf::from("/opt/homebrew/share/whisper-cpp/models");

        // Try quantized model first (faster), then standard
        let quantized = homebrew_dir.join(format!("ggml-{}-q5_0.bin", model_name));
        let standard = homebrew_dir.join(format!("ggml-{}.bin", model_name));

        if quantized.exists() {
            Some(quantized)
        } else if standard.exists() {
            Some(standard)
        } else {
            // Try fallback to base model
            let base_quantized = homebrew_dir.join("ggml-base-q5_0.bin");
            let base_standard = homebrew_dir.join("ggml-base.bin");

            if base_quantized.exists() {
                Some(base_quantized)
            } else if base_standard.exists() {
                Some(base_standard)
            } else {
                None
            }
        }
    }

    pub fn is_available(&self) -> bool {
        self.model_path.is_some()
    }

    pub fn model_name(&self) -> Option<String> {
        self.model_path.as_ref().and_then(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
        })
    }

    pub fn transcribe(&self, audio_path: &PathBuf) -> Result<String, String> {
        let model_path = self
            .model_path
            .as_ref()
            .ok_or("No Whisper model found. Use Setup Dictation to download.")?;

        // Audio is already 16kHz mono WAV from AudioRecorder
        let output = Command::new(&self.whisper_path)
            .args([
                "-m",
                model_path.to_str().unwrap(),
                "-f",
                audio_path.to_str().unwrap(),
                "-t",
                "8", // 8 threads for Apple Silicon
                "--no-timestamps",
                "-l",
                "auto", // Auto-detect language
            ])
            .output()
            .map_err(|e| format!("whisper-cli failed: {}", e))?;

        if output.status.success() {
            let transcription = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if transcription.is_empty() {
                Err("No speech detected".to_string())
            } else {
                Ok(transcription)
            }
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Transcription failed: {}", stderr))
        }
    }
}

/// Run the dictation setup flow via osascript dialogs
pub fn run_dictation_setup() {
    std::thread::spawn(|| {
        let status = WhisperTranscriber::new().setup_status();

        match status {
            DictationSetupStatus::Ready => {
                show_dialog("Dictation is already set up and ready to use!\n\nPress Fn+Shift to start recording.", "Setup Complete");
            }
            DictationSetupStatus::MissingModel => {
                if !confirm_dialog(
                    "Dictation requires a Whisper model (~500MB download).\n\nThis will download the medium model for speech recognition.\n\nContinue?",
                    "Download Model"
                ) {
                    return;
                }
                download_model();
            }
        }
    });
}

fn show_dialog(message: &str, title: &str) {
    native_dialogs::show_dialog(message, title);
}

fn confirm_dialog(message: &str, title: &str) -> bool {
    native_dialogs::show_confirm_dialog(message, title, "Continue", "Cancel")
}

fn download_model() {
    show_progress("Downloading Whisper model (~500MB)...");

    let models_dir = WhisperTranscriber::app_support_dir().join("models");
    if let Err(e) = fs::create_dir_all(&models_dir) {
        show_dialog(&format!("Failed to create models directory: {}", e), "Setup Failed");
        return;
    }

    let model_path = models_dir.join(MODEL_FILENAME);

    // Use curl with progress
    let result = Command::new("curl")
        .args([
            "-L",
            "--progress-bar",
            "-o",
            model_path.to_str().unwrap(),
            MODEL_URL,
        ])
        .output();

    match result {
        Ok(output) if output.status.success() => {
            show_dialog(
                "Dictation setup complete!\n\nRestart the app and press Fn+Shift to use dictation.",
                "Setup Complete",
            );
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            show_dialog(
                &format!("Failed to download model:\n\n{}", stderr.lines().take(2).collect::<Vec<_>>().join("\n")),
                "Download Failed",
            );
            // Clean up partial download
            let _ = fs::remove_file(&model_path);
        }
        Err(e) => {
            show_dialog(&format!("Failed to download: {}", e), "Download Failed");
        }
    }
}

fn show_progress(message: &str) {
    // Use notification instead of blocking dialog
    native_dialogs::show_notification(message, "Claude Sleep Preventer");
}
