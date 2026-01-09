use std::fs;
use std::path::PathBuf;
use std::process::Command;

use crate::logging;
use crate::native_dialogs;

use super::audio::check_and_request_microphone_permission;
use super::audio::MicrophonePermission;
use super::text_injection::check_accessibility_permission;

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

    // Show welcome dialog
    let welcome_message = r#"Bienvenue dans Claude Sleep Preventer !

Pour utiliser la dictée vocale (Fn+Shift), l'app a besoin de 3 permissions :

• Input Monitoring - pour détecter les touches
• Microphone - pour enregistrer votre voix
• Accessibility - pour écrire le texte

Cliquez "Configurer" pour les activer."#;

    if !show_confirm_dialog(welcome_message, "Configuration requise", "Configurer", "Plus tard") {
        logging::log("[onboarding] User skipped onboarding");
        return;
    }

    // Step 1: Input Monitoring
    setup_input_monitoring();

    // Step 2: Microphone
    setup_microphone();

    // Step 3: Accessibility
    setup_accessibility();

    // Done
    show_dialog(
        "Configuration terminée !\n\nAppuyez sur Fn+Shift pour dicter du texte.\n\nSi une permission manque, allez dans Préférences Système > Sécurité et confidentialité > Confidentialité.",
        "Prêt !",
    );

    mark_onboarding_complete();
    logging::log("[onboarding] Setup complete");
}

fn setup_input_monitoring() {
    // We can't programmatically check Input Monitoring permission,
    // so we just guide the user to enable it
    let message = r#"Étape 1/3 : Input Monitoring

Cette permission permet à l'app de détecter quand vous appuyez sur Fn+Shift.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Cliquez le cadenas pour déverrouiller
2. Cochez "Claude Sleep Preventer"
3. Revenez ici"#;

    if show_confirm_dialog(message, "Input Monitoring", "Ouvrir", "Passer") {
        // Open System Preferences to Input Monitoring
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();

        // Wait for user to come back
        show_dialog(
            "Une fois que vous avez activé Input Monitoring, cliquez OK pour continuer.",
            "Input Monitoring",
        );
    }
}

fn setup_microphone() {
    let mic_status = check_and_request_microphone_permission();

    match mic_status {
        MicrophonePermission::Granted => {
            logging::log("[onboarding] Microphone already granted");
        }
        MicrophonePermission::Requesting | MicrophonePermission::Denied => {
            // Request adds the app to the list, then we open System Preferences
            // The system dialog is unreliable, so we guide user manually
            open_microphone_settings();
        }
    }
}

fn open_microphone_settings() {
    let message = r#"Étape 2/3 : Microphone

Cette permission permet à l'app d'enregistrer votre voix pour la dictée.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Trouvez "Claude Sleep Preventer"
2. Activez l'accès au microphone
3. Revenez ici"#;

    if show_confirm_dialog(message, "Microphone", "Ouvrir", "Passer") {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
            .spawn();

        show_dialog("Cliquez OK une fois que vous avez activé le microphone.", "Microphone");
    }
}

fn setup_accessibility() {
    if check_accessibility_permission() {
        logging::log("[onboarding] Accessibility already granted");
        return;
    }

    let message = r#"Étape 3/3 : Accessibility

Cette permission permet à l'app d'écrire le texte dicté dans n'importe quelle application.

Cliquez "Ouvrir" pour accéder aux réglages, puis :
1. Cliquez le cadenas pour déverrouiller
2. Cochez "Claude Sleep Preventer"
3. Revenez ici"#;

    if show_confirm_dialog(message, "Accessibility", "Ouvrir", "Passer") {
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();

        show_dialog(
            "Cliquez OK une fois que vous avez activé Accessibility.",
            "Accessibility",
        );
    }
}

fn show_dialog(message: &str, title: &str) {
    native_dialogs::show_dialog(message, title);
}

fn show_confirm_dialog(message: &str, title: &str, confirm: &str, cancel: &str) -> bool {
    native_dialogs::show_confirm_dialog(message, title, confirm, cancel)
}
