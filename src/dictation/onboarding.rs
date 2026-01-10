use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::logging;
use crate::native_dialogs::{self, PermissionToggle, PermissionsAction, PermissionsWindow};

use super::audio::{check_microphone_permission, request_microphone_permission_sync, MicrophonePermission};
use super::globe_key::check_input_monitoring_permission;
use super::text_injection::check_accessibility_permission;
use super::transcription::{WhisperTranscriber, DictationSetupStatus};

/// Check if this is the first launch (no preferences file exists)
fn is_first_launch() -> bool {
    !get_prefs_path().exists()
}

/// Mark that onboarding has been completed
fn mark_onboarding_complete() {
    let prefs_path = get_prefs_path();
    if let Some(parent) = prefs_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&prefs_path, "onboarding_complete=true\n");
}

fn get_prefs_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("ClaudeSleepPreventer")
        .join("preferences.txt")
}

/// Run the onboarding flow if this is the first launch
pub fn run_onboarding_if_needed() {
    if !is_first_launch() {
        return;
    }

    logging::log("[onboarding] First launch detected, starting setup...");

    let welcome_message = r#"Bienvenue dans Claude Sleep Preventer !

Pour utiliser la dictée vocale (Fn+Shift), activez ces permissions :
Input Monitoring
Microphone
Accessibility

Cliquez sur un interrupteur pour ouvrir le réglage correspondant."#;

    let window = PermissionsWindow::new("Configuration requise", welcome_message);
    window.set_primary_button("Continuer");
    window.set_secondary_button("Plus tard");
    window.set_secondary_visible(true);

    loop {
        update_permission_toggles(&window);
        match window.wait_for_action() {
            PermissionsAction::Primary => {
                window.close();
                break;
            }
            PermissionsAction::Secondary => {
                window.close();
                logging::log("[onboarding] User skipped onboarding");
                return;
            }
            PermissionsAction::Toggle(toggle) => {
                handle_permission_toggle(toggle);
            }
        }
    }

    let model_window =
        native_dialogs::SetupWindow::new("Modèle Whisper", "Vérification du modèle...");
    setup_whisper_model(&model_window);

    let final_message = if WhisperTranscriber::new().setup_status() == DictationSetupStatus::Ready {
        "Configuration terminée !\n\nAppuyez sur Fn+Shift pour dicter du texte."
    } else {
        "Configuration terminée !\n\nPour activer la dictée, allez dans le menu et cliquez sur 'Setup Dictation...' pour télécharger le modèle Whisper."
    };
    model_window.set_title("Prêt !");
    model_window.set_message(final_message);
    model_window.set_primary_button("OK");
    model_window.set_secondary_visible(false);
    model_window.wait_for_action();
    model_window.close();

    mark_onboarding_complete();
    logging::log("[onboarding] Setup complete");
}

fn permission_label(name: &str, granted: bool) -> String {
    let status = if granted { "OK" } else { "OFF" };
    format!("{}: {}", name, status)
}

fn update_permission_toggles(window: &PermissionsWindow) {
    let input_ok = check_input_monitoring_permission();
    let mic_ok = matches!(check_microphone_permission(), MicrophonePermission::Granted);
    let accessibility_ok = check_accessibility_permission();

    window.set_toggle(
        PermissionToggle::InputMonitoring,
        &permission_label("Input Monitoring", input_ok),
        input_ok,
    );
    window.set_toggle(
        PermissionToggle::Microphone,
        &permission_label("Microphone", mic_ok),
        mic_ok,
    );
    window.set_toggle(
        PermissionToggle::Accessibility,
        &permission_label("Accessibility", accessibility_ok),
        accessibility_ok,
    );
}

fn handle_permission_toggle(toggle: PermissionToggle) {
    match toggle {
        PermissionToggle::InputMonitoring => {
            open_input_monitoring_settings();
        }
        PermissionToggle::Microphone => {
            let mut status = check_microphone_permission();
            if status == MicrophonePermission::NotDetermined {
                let granted = request_microphone_permission_sync();
                status = if granted {
                    MicrophonePermission::Granted
                } else {
                    MicrophonePermission::Denied
                };
            }
            if status != MicrophonePermission::Granted {
                open_microphone_settings();
            }
        }
        PermissionToggle::Accessibility => {
            open_accessibility_settings();
        }
    }
}

fn open_input_monitoring_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
        .spawn();
}

fn open_microphone_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
        .spawn();
}

fn open_accessibility_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

fn setup_whisper_model(window: &native_dialogs::SetupWindow) {
    window.show_progress(false);
    let transcriber = WhisperTranscriber::new();

    if transcriber.setup_status() == DictationSetupStatus::Ready {
        logging::log("[onboarding] Whisper model already available");
        return;
    }

    let message = r#"Étape 4/4 : Modèle Whisper

La dictée nécessite un modèle de reconnaissance vocale (~500 Mo).

Voulez-vous le télécharger maintenant ?"#;

    window.set_title("Modèle Whisper");
    window.set_message(message);
    window.set_primary_button("Télécharger");
    window.set_secondary_button("Plus tard");
    window.set_secondary_visible(true);

    if window.wait_for_action() == native_dialogs::SetupAction::Secondary {
        logging::log("[onboarding] User skipped Whisper model download");
        return;
    }

    match super::transcription::download_model_with_window(window) {
        Ok(()) => {
            window.set_title("Téléchargement terminé");
            window.set_message("Le modèle Whisper a été téléchargé.");
            window.set_primary_button("Continuer");
            window.set_secondary_visible(false);
            window.wait_for_action();
        }
        Err(e) => {
            window.set_title("Téléchargement échoué");
            window.set_message(&format!("Échec du téléchargement :\n\n{}", e));
            window.set_primary_button("OK");
            window.set_secondary_visible(false);
            window.wait_for_action();
        }
    }
}
