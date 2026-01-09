use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::logging;
use crate::native_dialogs;

use super::audio::{check_microphone_permission, request_microphone_permission_sync, MicrophonePermission};
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

Pour utiliser la dictée vocale (Fn+Shift), l'app a besoin de :

• Input Monitoring - pour détecter les touches
• Microphone - pour enregistrer votre voix
• Accessibility - pour écrire le texte
• Modèle Whisper - pour la reconnaissance vocale (~500 Mo)

Cliquez "Configurer" pour les activer."#;

    let window = native_dialogs::SetupWindow::new("Configuration requise", welcome_message);
    window.set_primary_button("Configurer");
    window.set_secondary_button("Plus tard");
    window.set_secondary_visible(true);

    if window.wait_for_action() == native_dialogs::SetupAction::Secondary {
        window.close();
        logging::log("[onboarding] User skipped onboarding");
        return;
    }

    setup_input_monitoring(&window);
    setup_microphone(&window);
    setup_accessibility(&window);
    setup_whisper_model(&window);

    let final_message = if WhisperTranscriber::new().setup_status() == DictationSetupStatus::Ready {
        "Configuration terminée !\n\nAppuyez sur Fn+Shift pour dicter du texte."
    } else {
        "Configuration terminée !\n\nPour activer la dictée, allez dans le menu et cliquez sur 'Setup Dictation...' pour télécharger le modèle Whisper."
    };
    window.set_title("Prêt !");
    window.set_message(final_message);
    window.set_primary_button("OK");
    window.set_secondary_visible(false);
    window.wait_for_action();
    window.close();

    mark_onboarding_complete();
    logging::log("[onboarding] Setup complete");
}

fn setup_input_monitoring(window: &native_dialogs::SetupWindow) {
    window.show_progress(false);
    // We can't programmatically check Input Monitoring permission,
    // so we just guide the user to enable it
    let message = r#"Étape 1/4 : Input Monitoring

Cette permission permet à l'app de détecter quand vous appuyez sur Fn+Shift.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Cliquez le cadenas pour déverrouiller
2. Cochez "Claude Sleep Preventer"
3. Revenez ici"#;

    window.set_title("Input Monitoring");
    window.set_message(message);
    window.set_primary_button("Ouvrir");
    window.set_secondary_button("Passer");
    window.set_secondary_visible(true);

    if window.wait_for_action() == native_dialogs::SetupAction::Primary {
        // Open System Preferences to Input Monitoring
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();

        // Wait for user to come back
        window.set_message(
            "Une fois que vous avez activé Input Monitoring, cliquez Continuer pour continuer.",
        );
        window.set_primary_button("Continuer");
        window.set_secondary_visible(false);
        window.wait_for_action();
    }
}

fn setup_microphone(window: &native_dialogs::SetupWindow) {
    window.show_progress(false);
    let mic_status = check_microphone_permission();

    match mic_status {
        MicrophonePermission::Granted => {
            logging::log("[onboarding] Microphone already granted");
        }
        MicrophonePermission::NotDetermined => {
            let message = "Étape 2/4 : Microphone\n\nUne demande d'accès au microphone va apparaître.\n\nCliquez \"Autoriser\" pour activer la dictée vocale.";
            window.set_title("Microphone");
            window.set_message(message);
            window.set_primary_button("Autoriser");
            window.set_secondary_button("Passer");
            window.set_secondary_visible(true);

            if window.wait_for_action() != native_dialogs::SetupAction::Primary {
                return;
            }

            let granted = request_microphone_permission_sync();
            if granted {
                logging::log("[onboarding] Microphone permission granted");
            } else {
                logging::log("[onboarding] Microphone permission denied");
                open_microphone_settings(window);
            }
        }
        MicrophonePermission::Denied => {
            open_microphone_settings(window);
        }
    }
}

fn open_microphone_settings(window: &native_dialogs::SetupWindow) {
    let message = r#"Étape 2/4 : Microphone

Cette permission permet à l'app d'enregistrer votre voix pour la dictée.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Trouvez "Claude Sleep Preventer"
2. Activez l'accès au microphone
3. Revenez ici"#;

    window.set_title("Microphone");
    window.set_message(message);
    window.set_primary_button("Ouvrir");
    window.set_secondary_button("Passer");
    window.set_secondary_visible(true);

    if window.wait_for_action() == native_dialogs::SetupAction::Primary {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn();

        window.set_message("Cliquez Continuer une fois que vous avez activé le microphone.");
        window.set_primary_button("Continuer");
        window.set_secondary_visible(false);
        window.wait_for_action();
    }
}

fn setup_accessibility(window: &native_dialogs::SetupWindow) {
    window.show_progress(false);
    if check_accessibility_permission() {
        logging::log("[onboarding] Accessibility already granted");
        return;
    }

    let message = r#"Étape 3/4 : Accessibility

Cette permission permet à l'app d'écrire le texte dicté dans n'importe quelle application.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Cliquez le cadenas pour déverrouiller
2. Cochez "Claude Sleep Preventer"
3. Revenez ici"#;

    window.set_title("Accessibility");
    window.set_message(message);
    window.set_primary_button("Ouvrir");
    window.set_secondary_button("Passer");
    window.set_secondary_visible(true);

    if window.wait_for_action() == native_dialogs::SetupAction::Primary {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();

        window.set_message("Cliquez Continuer une fois que vous avez activé Accessibility.");
        window.set_primary_button("Continuer");
        window.set_secondary_visible(false);
        window.wait_for_action();
    }
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
