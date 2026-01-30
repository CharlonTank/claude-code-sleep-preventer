mod authorization;
mod dictation;
mod logging;
mod native_dialogs;
mod objc_utils;
mod popover;
mod settings;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dictation::{run_onboarding_if_needed, DictationManager};
use core_foundation::base::{kCFAllocatorDefault, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun,
};
use core_foundation::string::CFString;
use core_foundation::string::CFStringRef;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use io_kit_sys::types::*;
use io_kit_sys::*;
use mach2::port::MACH_PORT_NULL;
use objc::{class, msg_send, sel, sel_impl};
use serde_json::json;
use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::time::Duration;
use sysinfo::System;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: CFStringRef,
        assertion_level: u32,
        assertion_name: CFStringRef,
        assertion_id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(assertion_id: u32) -> i32;
}

const IOPM_ASSERTION_LEVEL_ON: u32 = 255;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton,
    MouseButtonState,
    TrayIconBuilder,
    TrayIconEvent,
};

static LID_JUST_CLOSED: AtomicBool = AtomicBool::new(false);
static LID_WAS_CLOSED: AtomicBool = AtomicBool::new(false);
static CURRENT_PID_INDEX: AtomicUsize = AtomicUsize::new(0);
static CURRENT_INACTIVE_INDEX: AtomicUsize = AtomicUsize::new(0);
static CURRENT_ASSERTION_ID: AtomicU32 = AtomicU32::new(0);
static MANUAL_SLEEP_PREVENTION: AtomicBool = AtomicBool::new(true);

const PIDS_DIR: &str = "/tmp/claude_working_pids";
const IDLE_TIMEOUT_SECS: u64 = 30;
const IDLE_CPU_THRESHOLD: f32 = 0.5;

#[derive(Parser)]
#[command(name = "claude-sleep-preventer")]
#[command(about = "Keep your Mac awake while Claude Code is working")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register current Claude process and disable sleep
    Start,
    /// Unregister current Claude process and re-enable sleep if no others
    Stop,
    /// Show current status
    Status,
    /// List active/inactive instances as JSON
    List,
    /// Focus a Claude instance by PID
    Focus {
        pid: u32,
    },
    /// Clean up stale PIDs (interrupted sessions)
    Cleanup,
    /// Run as daemon with cleanup + thermal monitoring
    Daemon {
        #[arg(short, long, default_value = "1")]
        interval: u64,
    },
    /// Run background agent (dictation + permissions, no UI)
    Agent,
    /// Run native menu bar app
    Menubar,
    /// Force reset: clear all PIDs and re-enable sleep
    Reset,
    /// Check thermal state
    Thermal,
    /// Install hooks and configure Claude Code
    Install {
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Uninstall hooks and restore defaults
    Uninstall {
        /// Keep Whisper model data (~1.5 GB)
        #[arg(short = 'k', long)]
        keep_model: bool,
        /// Keep Claude Code hooks
        #[arg(long)]
        keep_hooks: bool,
        /// Keep app data and logs
        #[arg(long)]
        keep_data: bool,
    },
    /// Open the settings window
    Settings,
    /// Debug: list process names
    Debug,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Menubar) {
        Commands::Start => cmd_start()?,
        Commands::Stop => cmd_stop()?,
        Commands::Status => cmd_status()?,
        Commands::List => cmd_list()?,
        Commands::Focus { pid } => cmd_focus(pid)?,
        Commands::Cleanup => cmd_cleanup()?,
        Commands::Daemon { interval } => cmd_daemon(interval)?,
        Commands::Agent => cmd_agent()?,
        Commands::Menubar => cmd_menubar()?,
        Commands::Reset => cmd_reset()?,
        Commands::Thermal => cmd_thermal()?,
        Commands::Install { yes } => cmd_install(yes)?,
        Commands::Uninstall { keep_model, keep_hooks, keep_data } => cmd_uninstall(keep_model, keep_hooks, keep_data)?,
        Commands::Settings => cmd_settings()?,
        Commands::Debug => cmd_debug()?,
    }

    Ok(())
}

fn find_claude_ancestor() -> Option<u32> {
    let mut current_pid = std::process::id();

    for _ in 0..10 {
        let output = Command::new("ps")
            .args(["-p", &current_pid.to_string(), "-o", "ppid=,comm="])
            .output()
            .ok()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout.trim();

        if line.is_empty() {
            break;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let ppid: u32 = parts[0].parse().ok()?;

            let parent_output = Command::new("ps")
                .args(["-p", &ppid.to_string(), "-o", "comm="])
                .output()
                .ok()?;
            let parent_comm = String::from_utf8_lossy(&parent_output.stdout)
                .trim()
                .to_string();

            if parent_comm == "claude" {
                return Some(ppid);
            }
            current_pid = ppid;
        } else {
            break;
        }
    }

    Some(std::os::unix::process::parent_id())
}

fn ensure_pids_dir() -> Result<()> {
    fs::create_dir_all(PIDS_DIR).context("Failed to create PIDs directory")?;
    Ok(())
}

fn get_pid_file(pid: u32) -> PathBuf {
    PathBuf::from(PIDS_DIR).join(pid.to_string())
}

fn count_active_pids() -> usize {
    fs::read_dir(PIDS_DIR)
        .map(|entries| entries.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

fn set_sleep_disabled(disabled: bool) -> Result<()> {
    let value = if disabled { "1" } else { "0" };
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", value])
        .output()
        .context("Failed to run pmset")?;
    Ok(())
}

fn create_sleep_assertion() -> bool {
    let current = CURRENT_ASSERTION_ID.load(Ordering::SeqCst);
    if current != 0 {
        return true;
    }

    unsafe {
        let assertion_type = CFString::new("PreventUserIdleSystemSleep");
        let assertion_name = CFString::new("Claude Code Sleep Preventer");
        let mut assertion_id: u32 = 0;

        let result = IOPMAssertionCreateWithName(
            assertion_type.as_concrete_TypeRef(),
            IOPM_ASSERTION_LEVEL_ON,
            assertion_name.as_concrete_TypeRef(),
            &mut assertion_id,
        );

        if result == 0 {
            CURRENT_ASSERTION_ID.store(assertion_id, Ordering::SeqCst);
            true
        } else {
            false
        }
    }
}

fn release_sleep_assertion() {
    let assertion_id = CURRENT_ASSERTION_ID.swap(0, Ordering::SeqCst);
    if assertion_id != 0 {
        unsafe {
            IOPMAssertionRelease(assertion_id);
        }
    }
}

fn has_active_assertion() -> bool {
    CURRENT_ASSERTION_ID.load(Ordering::SeqCst) != 0
}

fn menubar_sync_sleep() {
    let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);
    let active = count_active_pids();
    let has_assertion = has_active_assertion();
    let should_prevent = manual_enabled && active > 0;

    if should_prevent && !has_assertion {
        create_sleep_assertion();
    } else if !should_prevent && has_assertion {
        release_sleep_assertion();
        if is_lid_closed() {
            force_sleep_now();
        }
    }
}

fn cleanup_stale_pids() {
    if let Ok(entries) = fs::read_dir(PIDS_DIR) {
        for entry in entries.filter_map(|e| e.ok()) {
            let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let path = entry.path();

            if !is_process_alive(pid) {
                let _ = fs::remove_file(&path);
                continue;
            }

            let age = get_file_age(&path).unwrap_or(0);
            if age >= IDLE_TIMEOUT_SECS {
                let cpu = get_process_cpu(pid);
                if cpu < IDLE_CPU_THRESHOLD {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }
}

fn is_sleep_disabled() -> bool {
    if has_active_assertion() {
        return true;
    }

    unsafe {
        let service_name = b"IOPMrootDomain\0";
        let matching = IOServiceMatching(service_name.as_ptr() as *const i8);
        if matching.is_null() {
            return false;
        }

        let root_domain = IOServiceGetMatchingService(kIOMasterPortDefault, matching);
        if root_domain == MACH_PORT_NULL {
            return false;
        }

        let key = CFString::new("SleepDisabled");
        let property = IORegistryEntryCreateCFProperty(
            root_domain,
            key.as_concrete_TypeRef(),
            kCFAllocatorDefault,
            0,
        );

        IOObjectRelease(root_domain);

        if property.is_null() {
            return false;
        }

        let result = CFBoolean::wrap_under_create_rule(property as _).into();
        result
    }
}

fn check_thermal_warning() -> bool {
    Command::new("pmset")
        .args(["-g", "therm"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| {
            (s.contains("CPU_Scheduler_Limit") && !s.contains("No CPU"))
                || (s.contains("thermal warning level") && !s.contains("No thermal warning"))
        })
        .unwrap_or(false)
}

fn get_process_cwd(pid: u32) -> Option<String> {
    Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with('n'))
                .map(|l| l[1..].to_string())
        })
}

fn get_git_branch(path: &str) -> Option<String> {
    Command::new("git")
        .args(["-C", path, "branch", "--show-current"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn format_process_location(pid: u32) -> String {
    get_process_cwd(pid)
        .map(|cwd| {
            let dir_name = std::path::Path::new(&cwd)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| cwd.clone());
            match get_git_branch(&cwd) {
                Some(branch) => format!("{} git:({})", dir_name, branch),
                None => dir_name,
            }
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn cmd_start() -> Result<()> {
    ensure_pids_dir()?;

    let claude_pid = find_claude_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(claude_pid);

    fs::write(&pid_file, "working").context("Failed to write PID file")?;

    Ok(())
}

fn cmd_stop() -> Result<()> {
    let claude_pid = find_claude_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(claude_pid);

    let _ = fs::remove_file(&pid_file);

    Ok(())
}

fn count_claude_processes() -> usize {
    Command::new("ps")
        .args(["-eo", "comm"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().filter(|l| l.trim() == "claude").count())
        .unwrap_or(0)
}

fn get_all_claude_pids() -> Vec<u32> {
    Command::new("ps")
        .args(["-eo", "pid,comm"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| {
            s.lines()
                .filter_map(|l| {
                    let parts: Vec<&str> = l.trim().split_whitespace().collect();
                    if parts.len() >= 2 && parts[1] == "claude" {
                        parts[0].parse().ok()
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn get_inactive_claude_pids() -> Vec<u32> {
    let all_pids = get_all_claude_pids();
    let active_pids: Vec<u32> = get_instance_items().iter().map(|(pid, _, _, _)| *pid).collect();
    all_pids
        .into_iter()
        .filter(|pid| !active_pids.contains(pid))
        .collect()
}

fn cmd_status() -> Result<()> {
    let sleep_disabled = is_sleep_disabled();
    let active_count = count_active_pids();
    let thermal_warning = check_thermal_warning();
    let claude_count = count_claude_processes();

    println!("Claude Code Sleep Preventer v{}", env!("CARGO_PKG_VERSION"));
    println!("==========================================");
    println!("Working instances: {}", active_count);
    println!("Claude processes: {}", claude_count);
    println!(
        "Sleep disabled: {}",
        if sleep_disabled { "Yes" } else { "No" }
    );
    println!(
        "Thermal warning: {}",
        if thermal_warning { "YES!" } else { "No" }
    );

    if active_count > 0 {
        println!("\nActive PIDs:");
        if let Ok(entries) = fs::read_dir(PIDS_DIR) {
            for entry in entries.filter_map(|e| e.ok()) {
                let pid: u32 = entry.file_name().to_string_lossy().parse().unwrap_or(0);
                if pid > 0 {
                    let age = get_file_age(&entry.path()).unwrap_or(0);
                    let cpu = get_process_cpu(pid);
                    let alive = is_process_alive(pid);
                    println!(
                        "  PID {}: age={}s, cpu={:.1}%, alive={}",
                        pid, age, cpu, alive
                    );
                }
            }
        }
    }

    Ok(())
}

fn cmd_list() -> Result<()> {
    let active = get_instance_items()
        .into_iter()
        .map(|(pid, age, cpu, location)| {
            json!({
                "pid": pid,
                "age_secs": age,
                "cpu": cpu,
                "location": location,
            })
        })
        .collect::<Vec<_>>();
    let inactive = get_inactive_claude_pids();
    let payload = json!({
        "active": active,
        "inactive": inactive,
    });
    println!("{}", payload);
    Ok(())
}

fn get_file_age(path: &PathBuf) -> Option<u64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .elapsed()
        .ok()
        .map(|d| d.as_secs())
}

fn get_process_cpu(pid: u32) -> f32 {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "%cpu="])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0.0)
}

fn is_process_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

fn is_lid_closed() -> bool {
    unsafe {
        let service_name = b"IOPMrootDomain\0";
        let matching = IOServiceMatching(service_name.as_ptr() as *const i8);
        if matching.is_null() {
            return false;
        }

        let root_domain = IOServiceGetMatchingService(kIOMasterPortDefault, matching);
        if root_domain == MACH_PORT_NULL {
            return false;
        }

        let key = CFString::new("AppleClamshellState");
        let property = IORegistryEntryCreateCFProperty(
            root_domain,
            key.as_concrete_TypeRef(),
            kCFAllocatorDefault,
            0,
        );

        IOObjectRelease(root_domain);

        if property.is_null() {
            return false;
        }

        let result = CFBoolean::wrap_under_create_rule(property as _).into();
        result
    }
}

fn force_sleep_now() {
    let _ = Command::new("sudo").args(["pmset", "sleepnow"]).output();
}

fn enable_sleep_and_trigger_if_lid_closed() -> Result<()> {
    set_sleep_disabled(false)?;
    if is_lid_closed() {
        force_sleep_now();
    }
    Ok(())
}

fn play_lid_close_sound() {
    std::thread::spawn(|| {
        let current_vol = Command::new("osascript")
            .args(["-e", "output volume of (get volume settings)"])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(50);

        let _ = Command::new("osascript")
            .args(["-e", "set volume output volume 100"])
            .output();

        let _ = Command::new("afplay")
            .args(["/System/Library/Sounds/Pop.aiff"])
            .output();

        let _ = Command::new("osascript")
            .args(["-e", &format!("set volume output volume {}", current_vol)])
            .output();
    });
}

unsafe extern "C" fn clamshell_notification_callback(
    _refcon: *mut std::ffi::c_void,
    _service: io_service_t,
    _message_type: u32,
    _message_argument: *mut std::ffi::c_void,
) {
    let lid_closed = is_lid_closed();
    let was_closed = LID_WAS_CLOSED.swap(lid_closed, Ordering::SeqCst);

    if lid_closed && !was_closed {
        LID_JUST_CLOSED.store(true, Ordering::SeqCst);
    }
}

fn start_clamshell_notifications() {
    std::thread::spawn(|| unsafe {
        let notify_port = IONotificationPortCreate(kIOMasterPortDefault);
        if notify_port.is_null() {
            eprintln!("Failed to create IONotificationPort");
            return;
        }

        let run_loop_source = IONotificationPortGetRunLoopSource(notify_port);
        if run_loop_source.is_null() {
            eprintln!("Failed to get run loop source");
            IONotificationPortDestroy(notify_port);
            return;
        }

        CFRunLoopAddSource(
            CFRunLoopGetCurrent(),
            run_loop_source as *mut _,
            kCFRunLoopDefaultMode,
        );

        let service_name = b"IOPMrootDomain\0";
        let matching = IOServiceMatching(service_name.as_ptr() as *const i8);
        if matching.is_null() {
            eprintln!("Failed to create matching dictionary");
            IONotificationPortDestroy(notify_port);
            return;
        }

        let root_domain = IOServiceGetMatchingService(kIOMasterPortDefault, matching);
        if root_domain == MACH_PORT_NULL {
            eprintln!("Failed to find IOPMrootDomain");
            IONotificationPortDestroy(notify_port);
            return;
        }

        let interest_type = b"IOGeneralInterest\0";
        let mut notification: io_object_t = 0;

        let result = IOServiceAddInterestNotification(
            notify_port,
            root_domain,
            interest_type.as_ptr() as *const i8,
            clamshell_notification_callback,
            ptr::null_mut(),
            &mut notification,
        );

        IOObjectRelease(root_domain);

        if result != 0 {
            eprintln!("Failed to add interest notification: {}", result);
            IONotificationPortDestroy(notify_port);
            return;
        }

        CFRunLoopRun();
    });
}

fn cmd_cleanup() -> Result<()> {
    if let Ok(entries) = fs::read_dir(PIDS_DIR) {
        for entry in entries.filter_map(|e| e.ok()) {
            let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let path = entry.path();

            if !is_process_alive(pid) {
                let _ = fs::remove_file(&path);
                continue;
            }

            let age = get_file_age(&path).unwrap_or(0);
            if age >= IDLE_TIMEOUT_SECS {
                let cpu = get_process_cpu(pid);
                if cpu < IDLE_CPU_THRESHOLD {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    // Fix sleep state
    let active = count_active_pids();
    let sleep_disabled = is_sleep_disabled();

    if active > 0 && !sleep_disabled {
        set_sleep_disabled(true)?;
    } else if active == 0 && sleep_disabled {
        enable_sleep_and_trigger_if_lid_closed()?;
    }

    Ok(())
}

fn cmd_reset() -> Result<()> {
    let _ = fs::remove_dir_all(PIDS_DIR);
    let _ = fs::create_dir_all(PIDS_DIR);
    enable_sleep_and_trigger_if_lid_closed()?;
    println!("Reset complete. Sleep re-enabled.");
    Ok(())
}

fn cmd_thermal() -> Result<()> {
    let warning = check_thermal_warning();
    if warning {
        println!("THERMAL WARNING DETECTED!");
        // Force reset if thermal warning
        cmd_reset()?;
    } else {
        println!("Thermal state: OK");
    }
    Ok(())
}

fn cmd_daemon(interval: u64) -> Result<()> {
    eprintln!(
        "Daemon started (interval: {}s, thermal check: 30s)",
        interval
    );

    let mut thermal_counter = 0u64;

    loop {
        // Cleanup every interval
        let _ = cmd_cleanup();

        // Thermal check every 30 seconds
        thermal_counter += interval;
        if thermal_counter >= 30 {
            thermal_counter = 0;
            if check_thermal_warning() {
                eprintln!("Thermal warning! Forcing sleep re-enable.");
                let _ = cmd_reset();
            }
        }

        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn create_tray_title(count: usize, manual_enabled: bool) -> String {
    if manual_enabled && count > 0 {
        format!("☕ {}", count)
    } else {
        "😴".to_string()
    }
}

fn is_installed() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    home.join(".claude/hooks/prevent-sleep.sh").exists()
}

fn run_first_time_setup() -> Result<()> {
    let message = "Claude Sleep Preventer needs to be configured to work with Claude Code.

This will:
• Install the CLI tool
• Configure Claude Code hooks
• Set up automatic startup

Administrator password required.";

    if !native_dialogs::show_confirm_dialog(message, "Claude Sleep Preventer", "Set Up", "Cancel") {
        return Ok(());
    }

    let script = "/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer install -y";

    match authorization::execute_script_with_privileges(script) {
        Ok(true) => {
            native_dialogs::show_dialog(
                "Setup complete!\n\nRestart Claude Code to activate sleep prevention.",
                "Claude Sleep Preventer",
            );
            relaunch_app_after_install();
        }
        Ok(false) => {
            // User cancelled
        }
        Err(e) => {
            native_dialogs::show_dialog(
                &format!("Setup failed: {}", e),
                "Claude Sleep Preventer",
            );
        }
    }

    Ok(())
}

fn relaunch_app_after_install() {
    logging::log("[main] Relaunching app after install...");
    match Command::new("open")
        .args(["-n", "/Applications/ClaudeSleepPreventer.app"])
        .status()
    {
        Ok(status) if status.success() => {
            std::process::exit(0);
        }
        Ok(status) => {
            logging::log(&format!(
                "[main] Relaunch failed with status: {}",
                status
            ));
        }
        Err(e) => {
            logging::log(&format!("[main] Relaunch failed: {}", e));
        }
    }
}

fn get_process_tty(pid: u32) -> Option<String> {
    Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "tty="])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "??")
}

fn focus_terminal_by_tty(tty: &str) {
    let tty_path = if tty.starts_with("/dev/") {
        tty.to_string()
    } else {
        format!("/dev/{}", tty)
    };

    // Try Terminal.app first
    let terminal_script = format!(
        r#"
        tell application "Terminal"
            set windowList to every window
            repeat with w in windowList
                set tabList to every tab of w
                repeat with t in tabList
                    if tty of t is "{}" then
                        set frontmost of w to true
                        set selected of t to true
                        activate
                        return true
                    end if
                end repeat
            end repeat
        end tell
        return false
        "#,
        tty_path
    );

    let result = Command::new("osascript")
        .args(["-e", &terminal_script])
        .output();

    if let Ok(output) = result {
        if String::from_utf8_lossy(&output.stdout).trim() == "true" {
            return;
        }
    }

    // Try iTerm2 - double activate to ensure space switch
    let iterm_script = format!(
        r#"
        tell application "iTerm2"
            set windowList to every window
            repeat with w in windowList
                set tabList to every tab of w
                repeat with t in tabList
                    set sessionList to every session of t
                    repeat with s in sessionList
                        if tty of s is "{}" then
                            activate
                            delay 0.1
                            select w
                            select t
                            select s
                            activate
                            return true
                        end if
                    end repeat
                end repeat
            end repeat
        end tell
        return false
        "#,
        tty_path
    );

    let _ = Command::new("osascript")
        .args(["-e", &iterm_script])
        .output();
}

fn focus_terminal_by_pid(pid: u32) {
    if let Some(tty) = get_process_tty(pid) {
        focus_terminal_by_tty(&tty);
    }
}

fn cmd_focus(pid: u32) -> Result<()> {
    logging::init();
    logging::log(&format!("[focus] requested pid={}", pid));
    focus_terminal_by_pid(pid);
    Ok(())
}

fn get_instance_items() -> Vec<(u32, u64, f32, String)> {
    let mut items = Vec::new();
    if let Ok(entries) = fs::read_dir(PIDS_DIR) {
        for entry in entries.filter_map(|e| e.ok()) {
            let pid: u32 = entry.file_name().to_string_lossy().parse().unwrap_or(0);
            if pid > 0 {
                let age = get_file_age(&entry.path()).unwrap_or(0);
                let cpu = get_process_cpu(pid);
                let location = format_process_location(pid);
                items.push((pid, age, cpu, location));
            }
        }
    }
    items
}

fn quit_app() {
    unsafe {
        let app: objc_utils::Id = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let _: () = msg_send![app, terminate: objc_utils::NIL];
    }
}

fn cmd_menubar() -> Result<()> {
    // Initialize logging
    logging::init();
    logging::log("[main] Starting menubar app");

    if !is_installed() {
        run_first_time_setup()?;
        if !is_installed() {
            return Ok(());
        }
    }

    // Load settings and initialize sleep prevention state
    let app_settings = settings::AppSettings::load();
    MANUAL_SLEEP_PREVENTION.store(app_settings.sleep_prevention.enabled, Ordering::SeqCst);
    logging::log(&format!(
        "[main] Loaded settings: sleep_prevention={}",
        app_settings.sleep_prevention.enabled
    ));

    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    let tick_proxy = event_loop.create_proxy();

    let initial_instances = get_instance_items();
    let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);

    // Initialize dictation manager
    let mut dictation_manager = DictationManager::new();

    // Create a minimal menu for right-click, but show popover on left-click
    let minimal_menu = Menu::new();
    let settings_item = MenuItem::new("Settings...", true, None);
    let settings_item_id = settings_item.id().clone();
    let _ = minimal_menu.append(&settings_item);
    let quit_item = MenuItem::new("Quit", true, None);
    let quit_item_id = quit_item.id().clone();
    let _ = minimal_menu.append(&quit_item);

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(minimal_menu))
        .with_menu_on_left_click(false) // Left-click shows popover, right-click shows menu
        .with_title(&create_tray_title(initial_instances.len(), manual_enabled))
        .with_tooltip("Claude Code Sleep Preventer")
        .build()?;

    let menu_channel = MenuEvent::receiver();
    let tray_event_channel = TrayIconEvent::receiver();

    // Initialize popover window
    let mut popover = popover::PopoverWindow::new();

    // Register global hotkeys
    let hotkey_manager = GlobalHotKeyManager::new().unwrap();
    let hotkey_active = HotKey::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER),
        Code::KeyJ,
    );
    let hotkey_inactive = HotKey::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER),
        Code::KeyL,
    );
    let hotkey_active_id = hotkey_active.id();
    let hotkey_inactive_id = hotkey_inactive.id();
    hotkey_manager.register(hotkey_active).unwrap();
    hotkey_manager.register(hotkey_inactive).unwrap();
    let hotkey_receiver = GlobalHotKeyEvent::receiver();

    start_clamshell_notifications();

    std::thread::spawn(move || {
        let mut tick_counter = 0u64;
        let _ = tick_proxy.send_event(());

        loop {
            std::thread::sleep(Duration::from_millis(100));
            tick_counter += 1;

            if LID_JUST_CLOSED.swap(false, Ordering::SeqCst) {
                let active = count_active_pids();
                if active > 0 {
                    play_lid_close_sound();
                }
            }

            if tick_counter % 100 == 0 {
                cleanup_stale_pids();
                menubar_sync_sleep();
            }

            if tick_counter % 300 == 0 {
                if check_thermal_warning() {
                    release_sleep_assertion();
                }
            }

            let _ = tick_proxy.send_event(());
        }
    });

    let mut last_update = std::time::Instant::now() - Duration::from_secs(10);

    let mut click_check_counter = 0u64;
    let mut onboarding_checked = false;
    event_loop.run(move |event, _, control_flow| {
        // Drive updates from the tick thread to avoid relying on user input events.
        *control_flow = ControlFlow::Wait;

        if !onboarding_checked {
            if let Event::NewEvents(StartCause::Init) = event {
                onboarding_checked = true;
                run_onboarding_if_needed(false);
                let dictation_available = dictation_manager.is_available();
                let dictation_enabled = dictation_manager.is_enabled();
                logging::log(&format!(
                    "[dictation] available={}, enabled={}",
                    dictation_available, dictation_enabled
                ));
                if dictation_available {
                    if let Err(e) = dictation_manager.start() {
                        logging::log(&format!("[dictation] Failed to start: {}", e));
                    }
                }
            }
        }

        click_check_counter += 1;
        if click_check_counter % 100 == 0 {
            eprintln!("[DEBUG] Event loop iteration #{}", click_check_counter);
        }
        while let Ok(menu_event) = menu_channel.try_recv() {
            if menu_event.id == settings_item_id {
                logging::log("[menu] Settings selected");
                popover.hide();
                if let Some(new_settings) = settings::window::show_settings() {
                    // Update manual sleep prevention based on settings
                    MANUAL_SLEEP_PREVENTION.store(new_settings.sleep_prevention.enabled, Ordering::SeqCst);
                    menubar_sync_sleep();
                    logging::log(&format!(
                        "[menu] Settings saved: sleep_enabled={}",
                        new_settings.sleep_prevention.enabled
                    ));
                }
            } else if menu_event.id == quit_item_id {
                logging::log("[menu] Quit selected");
                dictation_manager.stop();
                quit_app();
                logging::log("[menu] Forcing process exit");
                unsafe {
                    libc::_exit(0);
                }
            }
        }

        // Check for tray events every iteration
        while let Ok(tray_event) = tray_event_channel.try_recv() {
            eprintln!("[TRAY EVENT] {:?}", tray_event);
            logging::log(&format!("[tray] EVENT: {:?}", tray_event));
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                rect,
                ..
            } = tray_event
            {
                logging::log(&format!("[tray] Left click at rect: {:?}", rect));
                if popover.is_visible() {
                    popover.hide();
                } else {
                    let icon_rect = objc_utils::tray_rect_to_appkit((
                        rect.position.x,
                        rect.position.y,
                        rect.size.width as f64,
                        rect.size.height as f64,
                    ));
                    let pstate = popover::PopoverState {
                        manual_enabled: MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst),
                        instances: get_instance_items(),
                        inactive: get_inactive_claude_pids(),
                        thermal_warning: check_thermal_warning(),
                        dictation_enabled: dictation_manager.is_enabled(),
                        dictation_available: dictation_manager.is_available(),
                    };
                    popover.show(icon_rect, &pstate);
                }
            }
        }

        // Update dictation manager (handles Fn+Space events)
        dictation_manager.update();

        if last_update.elapsed() >= Duration::from_secs(2) {
            last_update = std::time::Instant::now();

            let instances = get_instance_items();
            let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);

            tray.set_title(Some(&create_tray_title(instances.len(), manual_enabled)));
        }

        // Menu events commented out - we use popover instead
        // The menu handling code has been removed since we no longer attach a menu

        // Handle global hotkeys
        if let Ok(event) = hotkey_receiver.try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
                logging::log(&format!("[hotkey] Pressed id={}", event.id));
                if event.id == hotkey_active_id {
                    let instances = get_instance_items();
                    if !instances.is_empty() {
                        let idx = CURRENT_PID_INDEX.fetch_add(1, Ordering::SeqCst) % instances.len();
                        focus_terminal_by_pid(instances[idx].0);
                    }
                } else if event.id == hotkey_inactive_id {
                    let inactive = get_inactive_claude_pids();
                    if !inactive.is_empty() {
                        let idx = CURRENT_INACTIVE_INDEX.fetch_add(1, Ordering::SeqCst) % inactive.len();
                        focus_terminal_by_pid(inactive[idx]);
                    }
                }
            }
        }

    });
}

fn cmd_agent() -> Result<()> {
    logging::init();
    logging::log("[agent] Starting background agent");

    let _agent_lock = match acquire_agent_lock() {
        Ok(Some(lock)) => lock,
        Ok(None) => {
            logging::log("[agent] Another agent is already running; exiting");
            return Ok(());
        }
        Err(e) => {
            logging::log(&format!("[agent] Failed to acquire lock: {}", e));
            return Ok(());
        }
    };

    unsafe {
        let _: objc_utils::Id = msg_send![class!(NSApplication), sharedApplication];
    }

    run_onboarding_if_needed(true);

    let mut dictation_manager = DictationManager::new();
    let dictation_available = dictation_manager.is_available();
    let dictation_enabled = dictation_manager.is_enabled();
    logging::log(&format!(
        "[agent] dictation available={}, enabled={}",
        dictation_available, dictation_enabled
    ));
    if dictation_available {
        if let Err(e) = dictation_manager.start() {
            logging::log(&format!("[agent] Failed to start dictation: {}", e));
        }
    }

    loop {
        dictation_manager.update();
        objc_utils::pump_run_loop_once();
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn acquire_agent_lock() -> Result<Option<std::fs::File>> {
    let lock_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("ClaudeSleepPreventer");
    fs::create_dir_all(&lock_dir)?;
    let lock_path = lock_dir.join("agent.lock");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .open(&lock_path)?;
    let fd = file.as_raw_fd();
    let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return Ok(None);
    }
    let _ = file.set_len(0);
    let _ = writeln!(file, "pid={}", std::process::id());
    Ok(Some(file))
}

fn ask_yes_no(prompt: &str) -> bool {
    use std::io::{self, BufRead};
    print!("{} [Y/n]: ", prompt);
    let _ = io::Write::flush(&mut io::stdout());
    let stdin = io::stdin();
    let answer = stdin
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    answer.is_empty() || answer == "y" || answer == "yes"
}

fn cmd_install(auto_yes: bool) -> Result<()> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let hooks_dir = home.join(".claude").join("hooks");
    let settings_file = home.join(".claude").join("settings.json");
    let launch_agents_dir = home.join("Library/LaunchAgents");

    fs::create_dir_all(&hooks_dir)?;

    let app_binary = "/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer";
    let prevent_script = format!(
        "#!/bin/bash\n[ -x \"{}\" ] && \"{}\" start 2>/dev/null || true\n",
        app_binary, app_binary
    );
    let allow_script = format!(
        "#!/bin/bash\n[ -x \"{}\" ] && \"{}\" stop 2>/dev/null || true\n",
        app_binary, app_binary
    );

    fs::write(hooks_dir.join("prevent-sleep.sh"), prevent_script)?;
    fs::write(hooks_dir.join("allow-sleep.sh"), allow_script)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            hooks_dir.join("prevent-sleep.sh"),
            fs::Permissions::from_mode(0o755),
        )?;
        fs::set_permissions(
            hooks_dir.join("allow-sleep.sh"),
            fs::Permissions::from_mode(0o755),
        )?;
    }

    println!("Setting up passwordless sudo for pmset...");
    let sudoers_content = format!(
        "{} ALL=(ALL) NOPASSWD: /usr/bin/pmset\n",
        std::env::var("USER").unwrap_or_default()
    );

    let mut child = Command::new("sudo")
        .args(["tee", "/etc/sudoers.d/claude-pmset"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(sudoers_content.as_bytes())?;
    }
    child.wait()?;

    Command::new("sudo")
        .args(["chmod", "440", "/etc/sudoers.d/claude-pmset"])
        .output()?;

    println!("Configuring Claude Code hooks...");

    let hooks_json = r#"{
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "PreToolUse": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "PreCompact": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/allow-sleep.sh" }] }]
}"#;

    let hooks: serde_json::Value =
        serde_json::from_str(hooks_json).context("Failed to parse hooks JSON")?;

    if settings_file.exists() {
        let content = fs::read_to_string(&settings_file)
            .with_context(|| format!("Failed to read {}", settings_file.display()))?;
        let mut json: serde_json::Value = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", settings_file.display()))?;
        json["hooks"] = hooks;
        fs::write(&settings_file, serde_json::to_string_pretty(&json)?)
            .with_context(|| format!("Failed to write {}", settings_file.display()))?;
        println!("  Updated {}", settings_file.display());
    } else {
        fs::create_dir_all(settings_file.parent().unwrap())?;
        let mut json = serde_json::json!({});
        json["hooks"] = hooks;
        fs::write(&settings_file, serde_json::to_string_pretty(&json)?)
            .with_context(|| format!("Failed to write {}", settings_file.display()))?;
        println!("  Created {}", settings_file.display());
    }

    Command::new("sudo")
        .args(["pmset", "-a", "sleep", "5"])
        .output()?;
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    println!();
    if auto_yes || ask_yes_no("Launch menu bar app at login?") {
        fs::create_dir_all(&launch_agents_dir)?;

        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.charlontank.claude-sleep-preventer</string>
    <key>ProgramArguments</key>
    <array>
        <string>/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#;

        let plist_path = launch_agents_dir.join("com.charlontank.claude-sleep-preventer.plist");
        fs::write(&plist_path, plist)?;

        println!("  Created LaunchAgent for login startup");
        println!("  Note: Copy ClaudeSleepPreventer.app to /Applications");
    }

    println!("\n✅ Installation complete!");
    println!("\nRestart Claude Code to activate.");
    println!("\nCommands:");
    println!("  claude-sleep-preventer status   - Show current state");
    println!("  claude-sleep-preventer cleanup  - Clean up stale PIDs");
    println!("  claude-sleep-preventer reset    - Force enable sleep");
    println!("  claude-sleep-preventer menubar  - Run native menu bar");
    println!("  claude-sleep-preventer daemon   - Run background daemon");

    // Try to launch the app
    let _ = Command::new("open")
        .arg("/Applications/ClaudeSleepPreventer.app")
        .spawn();

    Ok(())
}

fn cmd_uninstall(keep_model: bool, keep_hooks: bool, keep_data: bool) -> Result<()> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let hooks_dir = home.join(".claude").join("hooks");
    let settings_file = home.join(".claude").join("settings.json");
    let launch_agents_dir = home.join("Library/LaunchAgents");

    // Remove hook scripts (unless keeping hooks)
    if !keep_hooks {
        let _ = fs::remove_file(hooks_dir.join("prevent-sleep.sh"));
        let _ = fs::remove_file(hooks_dir.join("allow-sleep.sh"));

        // Remove hooks from settings.json
        if settings_file.exists() {
            if let Ok(content) = fs::read_to_string(&settings_file) {
                if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if json.get("hooks").is_some() {
                        json.as_object_mut().unwrap().remove("hooks");
                        let _ = fs::write(&settings_file, serde_json::to_string_pretty(&json).unwrap());
                        println!("Removed hooks from settings.json");
                    }
                }
            }
        }
        println!("Removed Claude Code hooks");
    }

    // Remove LaunchAgent
    let plist_path = launch_agents_dir.join("com.charlontank.claude-sleep-preventer.plist");
    if plist_path.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", plist_path.to_str().unwrap()])
            .output();
        let _ = fs::remove_file(&plist_path);
        println!("Removed LaunchAgent");
    }

    // Remove sudoers config
    Command::new("sudo")
        .args(["rm", "-f", "/etc/sudoers.d/claude-pmset"])
        .output()?;

    // Remove PID tracking directory
    let _ = fs::remove_dir_all(PIDS_DIR);

    // Reset sleep settings
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    // Remove app data and preferences (unless keeping data)
    if !keep_data {
        let app_support = home.join("Library/Application Support/ClaudeSleepPreventer");
        if app_support.exists() {
            if keep_model {
                if let Ok(entries) = fs::read_dir(&app_support) {
                    for entry in entries.flatten() {
                        let name = entry.file_name();
                        if name == std::ffi::OsStr::new("models") {
                            continue;
                        }
                        let path = entry.path();
                        if path.is_dir() {
                            let _ = fs::remove_dir_all(&path);
                        } else {
                            let _ = fs::remove_file(&path);
                        }
                    }
                }
                println!("Removed app data (kept Whisper model)");
            } else {
                let _ = fs::remove_dir_all(&app_support);
                println!("Removed app data and Whisper model");
            }
        }

        // Remove logs
        let logs_dir = home.join("Library/Logs/ClaudeSleepPreventer");
        let _ = fs::remove_dir_all(&logs_dir);
        println!("Removed logs");
    }

    // Remove the app from /Applications
    Command::new("sudo")
        .args(["rm", "-rf", "/Applications/ClaudeSleepPreventer.app"])
        .output()?;
    println!("Removed app from /Applications");

    println!("Uninstalled successfully");

    Ok(())
}

fn cmd_settings() -> Result<()> {
    logging::init();
    logging::log("[settings] Opening settings window");

    // Initialize NSApplication for GUI
    unsafe {
        let _: objc_utils::Id = msg_send![class!(NSApplication), sharedApplication];
    }

    if let Some(new_settings) = settings::window::show_settings() {
        logging::log(&format!(
            "[settings] Settings saved: sleep_enabled={}, language={}",
            new_settings.sleep_prevention.enabled,
            new_settings.speech_to_text.language
        ));
    } else {
        logging::log("[settings] Settings cancelled");
    }

    Ok(())
}

fn cmd_debug() -> Result<()> {
    println!("sysinfo processes:");
    let sys = System::new_all();
    for (pid, proc) in sys.processes() {
        let name = proc.name().to_string_lossy();
        if name.to_lowercase().contains("claude") {
            println!("  PID {}: name={:?}", pid.as_u32(), name);
        }
    }

    println!("\nps command:");
    let output = Command::new("ps").args(["-eo", "pid,comm"]).output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.to_lowercase().contains("claude") {
            println!("  {}", line.trim());
        }
    }

    Ok(())
}
