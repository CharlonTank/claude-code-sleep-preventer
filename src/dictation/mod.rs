mod audio;
mod globe_key;
mod onboarding;
mod overlay;
mod text_injection;
mod transcription;

pub use onboarding::run_onboarding_if_needed;
pub use transcription::run_dictation_setup;

use crate::logging;
use audio::{check_and_request_microphone_permission, AudioRecorder, MicrophonePermission};
use globe_key::{GlobeKeyEvent, GlobeKeyManager};
use overlay::{OverlayMode, RecordingOverlay};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use transcription::WhisperTranscriber;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DictationState {
    Idle,
    Recording,
    Transcribing,
}

pub enum DictationResult {
    Transcribed(String),
    Error(String),
}

pub struct DictationManager {
    state: DictationState,
    globe_key: GlobeKeyManager,
    recorder: Option<AudioRecorder>,
    transcriber: WhisperTranscriber,
    overlay: RecordingOverlay,
    result_rx: Option<Receiver<DictationResult>>,
    enabled: bool,
}

impl DictationManager {
    pub fn new() -> Self {
        Self {
            state: DictationState::Idle,
            globe_key: GlobeKeyManager::new(),
            recorder: None,
            transcriber: WhisperTranscriber::new(),
            overlay: RecordingOverlay::new(),
            result_rx: None,
            enabled: true,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if !self.transcriber.is_available() {
            return Err(
                "Whisper not available. Install with: brew install whisper-cpp".to_string(),
            );
        }

        // Check/request microphone permission
        let mic_permission = check_and_request_microphone_permission();
        logging::log(&format!("[dictation] Microphone permission: {:?}", mic_permission));

        match mic_permission {
            MicrophonePermission::Granted => {}
            MicrophonePermission::Requesting => {
                logging::log("[dictation] Requesting microphone permission...");
            }
            MicrophonePermission::Denied => {
                logging::log("[dictation] Microphone permission denied");
            }
        }

        self.globe_key.start()
    }

    pub fn stop(&mut self) {
        self.globe_key.stop();
        self.overlay.hide();
        self.state = DictationState::Idle;
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.stop();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn is_available(&self) -> bool {
        self.transcriber.is_available()
    }

    pub fn state(&self) -> DictationState {
        self.state
    }

    pub fn model_name(&self) -> Option<String> {
        self.transcriber.model_name()
    }

    pub fn update(&mut self) {
        if !self.enabled {
            return;
        }

        // Check for globe key events
        while let Some(event) = self.globe_key.try_recv() {
            match event {
                GlobeKeyEvent::Ready => {
                    logging::log("[dictation] Globe key listener ready");
                }
                GlobeKeyEvent::DictateStart => {
                    if self.state == DictationState::Idle {
                        self.start_recording();
                    }
                }
                GlobeKeyEvent::DictateStop => {
                    if self.state == DictationState::Recording {
                        self.stop_and_transcribe();
                    }
                }
            }
        }

        // Check for transcription results
        if self.state == DictationState::Transcribing {
            if let Some(rx) = &self.result_rx {
                match rx.try_recv() {
                    Ok(DictationResult::Transcribed(text)) => {
                        logging::log(&format!("[dictation] Transcription: {}", text));
                        self.overlay.hide();
                        if let Err(e) = text_injection::inject_text(&text) {
                            logging::log(&format!("[dictation] Failed to inject text: {}", e));
                        }
                        self.state = DictationState::Idle;
                        self.result_rx = None;
                    }
                    Ok(DictationResult::Error(e)) => {
                        logging::log(&format!("[dictation] Transcription error: {}", e));
                        self.overlay.hide();
                        self.state = DictationState::Idle;
                        self.result_rx = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        // Still processing
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        logging::log("[dictation] Transcription channel disconnected");
                        self.overlay.hide();
                        self.state = DictationState::Idle;
                        self.result_rx = None;
                    }
                }
            }
        }
    }

    fn start_recording(&mut self) {
        // Initialize recorder
        match AudioRecorder::new() {
            Ok(mut recorder) => {
                if let Err(e) = recorder.start_recording() {
                    logging::log(&format!("[dictation] Failed to start recording: {}", e));
                    return;
                }
                self.recorder = Some(recorder);
            }
            Err(e) => {
                logging::log(&format!("[dictation] Failed to create recorder: {}", e));
                return;
            }
        }

        // Show overlay
        self.overlay.show();
        self.state = DictationState::Recording;
        logging::log("[dictation] Recording started");
    }

    fn stop_and_transcribe(&mut self) {
        // Switch overlay to transcribing mode (orange)
        self.overlay.set_mode(OverlayMode::Transcribing);

        // Get samples from recorder
        let samples = match self.recorder.as_mut() {
            Some(recorder) => recorder.stop_recording(),
            None => {
                logging::log("[dictation] No recorder available");
                self.overlay.hide();
                self.state = DictationState::Idle;
                return;
            }
        };

        if samples.is_empty() {
            logging::log("[dictation] No audio recorded");
            self.overlay.hide();
            self.state = DictationState::Idle;
            return;
        }

        // Log audio stats
        let duration_secs = samples.len() as f32 / 48000.0; // Assuming 48kHz
        logging::log(&format!(
            "[dictation] Audio: {} samples, ~{:.1}s duration",
            samples.len(),
            duration_secs
        ));

        // Save to temp file
        let temp_dir = std::env::temp_dir();
        let audio_path = temp_dir.join(format!("dictation_{}.wav", std::process::id()));

        let recorder = self.recorder.take().unwrap();
        if let Err(e) = recorder.save_to_wav(&samples, &audio_path) {
            logging::log(&format!("[dictation] Failed to save audio: {}", e));
            self.overlay.hide();
            self.state = DictationState::Idle;
            return;
        }

        // Start transcription in background thread
        let (tx, rx): (Sender<DictationResult>, Receiver<DictationResult>) = mpsc::channel();
        self.result_rx = Some(rx);
        self.state = DictationState::Transcribing;

        let transcriber = WhisperTranscriber::new();
        thread::spawn(move || {
            let result = match transcriber.transcribe(&audio_path) {
                Ok(text) => DictationResult::Transcribed(text),
                Err(e) => DictationResult::Error(e),
            };

            // Clean up audio file
            let _ = std::fs::remove_file(&audio_path);

            let _ = tx.send(result);
        });

        logging::log("[dictation] Recording stopped, transcribing...");
    }
}

impl Drop for DictationManager {
    fn drop(&mut self) {
        self.stop();
    }
}
