use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
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
use std::fs;
use std::io::Write;
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

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(stateID: i32) -> u64;
}

const CG_EVENT_SOURCE_STATE_COMBINED: i32 = 0;
const CG_EVENT_FLAG_MASK_ALTERNATE: u64 = 0x00080000;

const IOPM_ASSERTION_LEVEL_ON: u32 = 255;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder,
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
    /// Clean up stale PIDs (interrupted sessions)
    Cleanup,
    /// Run as daemon with cleanup + thermal monitoring
    Daemon {
        #[arg(short, long, default_value = "1")]
        interval: u64,
    },
    /// Run native menu bar app
    Menubar,
    /// Force reset: clear all PIDs and re-enable sleep
    Reset,
    /// Check thermal state
    Thermal,
    /// Install hooks and configure Claude Code
    Install,
    /// Uninstall hooks and restore defaults
    Uninstall,
    /// Debug: list process names
    Debug,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::Menubar) {
        Commands::Start => cmd_start()?,
        Commands::Stop => cmd_stop()?,
        Commands::Status => cmd_status()?,
        Commands::Cleanup => cmd_cleanup()?,
        Commands::Daemon { interval } => cmd_daemon(interval)?,
        Commands::Menubar => cmd_menubar()?,
        Commands::Reset => cmd_reset()?,
        Commands::Thermal => cmd_thermal()?,
        Commands::Install => cmd_install()?,
        Commands::Uninstall => cmd_uninstall()?,
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
    let result = Command::new("osascript")
        .args([
            "-e",
            r#"display dialog "Claude Sleep Preventer needs to be configured to work with Claude Code.

This will:
• Install the CLI tool
• Configure Claude Code hooks
• Set up automatic startup

Administrator password required." buttons {"Cancel", "Set Up"} default button "Set Up" with title "Claude Sleep Preventer" with icon note"#,
        ])
        .output()?;

    if !result.status.success() || String::from_utf8_lossy(&result.stdout).contains("Cancel") {
        return Ok(());
    }

    let script = r#"do shell script "echo 'y' | /Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer install" with administrator privileges"#;

    let install_result = Command::new("osascript").args(["-e", script]).output()?;

    if install_result.status.success() {
        let _ = Command::new("osascript")
            .args([
                "-e",
                r#"display dialog "Setup complete!

Restart Claude Code to activate sleep prevention." buttons {"OK"} default button "OK" with title "Claude Sleep Preventer" with icon note"#,
            ])
            .output();
    } else {
        let error = String::from_utf8_lossy(&install_result.stderr);
        let _ = Command::new("osascript")
            .args([
                "-e",
                &format!(r#"display dialog "Setup failed: {}" buttons {{"OK"}} default button "OK" with title "Claude Sleep Preventer" with icon stop"#, error.lines().next().unwrap_or("Unknown error")),
            ])
            .output();
    }

    Ok(())
}

fn run_uninstall_flow() -> Result<()> {
    let result = Command::new("osascript")
        .args([
            "-e",
            r#"display dialog "Are you sure you want to uninstall Claude Sleep Preventer?

This will remove:
• Claude Code hooks
• Launch agent
• Sudoers configuration

The app will remain in /Applications." buttons {"Cancel", "Uninstall"} default button "Cancel" with title "Uninstall" with icon caution"#,
        ])
        .output()?;

    if !result.status.success() || String::from_utf8_lossy(&result.stdout).contains("Cancel") {
        return Ok(());
    }

    let script = r#"do shell script "/Applications/ClaudeSleepPreventer.app/Contents/MacOS/claude-sleep-preventer uninstall" with administrator privileges"#;

    let _ = Command::new("osascript").args(["-e", script]).output();

    let _ = Command::new("osascript")
        .args([
            "-e",
            r#"display dialog "Uninstall complete.

You can delete the app from /Applications manually." buttons {"OK"} default button "OK" with title "Uninstall" with icon note"#,
        ])
        .output();

    Ok(())
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

fn is_option_key_pressed() -> bool {
    unsafe {
        let flags = CGEventSourceFlagsState(CG_EVENT_SOURCE_STATE_COMBINED);
        (flags & CG_EVENT_FLAG_MASK_ALTERNATE) != 0
    }
}

fn kill_inactive_claudes() {
    let inactive = get_inactive_claude_pids();
    for pid in inactive {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
    }
}

struct MenuState {
    toggle_item: CheckMenuItem,
    instances_header: MenuItem,
    instance_items: Vec<(MenuItem, u32)>,
    inactive_header: MenuItem,
    inactive_items: Vec<(MenuItem, u32)>,
    kill_inactive_item: MenuItem,
    thermal_item: MenuItem,
    uninstall_item: MenuItem,
    quit_item: MenuItem,
}

fn build_menu(
    instances: &[(u32, u64, f32, String)],
    inactive: &[u32],
    manual_enabled: bool,
    thermal: bool,
) -> (Menu, MenuState) {
    let menu = Menu::new();

    let toggle_text = if manual_enabled {
        if instances.is_empty() {
            "🔵─⚪ Sleep Prevention (Idle)"
        } else {
            "🔵─⚪ Sleep Prevention (Working)"
        }
    } else {
        "⚪─⚫ Sleep Prevention"
    };
    let toggle_item = CheckMenuItem::new(toggle_text, true, false, None);
    menu.append(&toggle_item).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    let instances_header = MenuItem::new(
        if instances.is_empty() {
            "No Active Instances"
        } else {
            "Active Instances"
        },
        false,
        None,
    );
    menu.append(&instances_header).unwrap();

    let mut instance_items: Vec<(MenuItem, u32)> = Vec::new();
    for (pid, age, cpu, location) in instances {
        let item = MenuItem::new(
            &format!("  {} - {}s - {:.1}%", location, age, cpu),
            true,
            None,
        );
        menu.append(&item).unwrap();
        instance_items.push((item, *pid));
    }

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    let inactive_header = MenuItem::new(
        if inactive.is_empty() {
            "No Inactive Instances"
        } else {
            "Inactive Instances"
        },
        false,
        None,
    );
    menu.append(&inactive_header).unwrap();

    let mut inactive_items: Vec<(MenuItem, u32)> = Vec::new();
    for pid in inactive {
        let location = format_process_location(*pid);
        let item = MenuItem::new(&format!("  {} (⌥ kill)", location), true, None);
        menu.append(&item).unwrap();
        inactive_items.push((item, *pid));
    }

    let kill_inactive_item = MenuItem::new("Kill All Inactive", !inactive.is_empty(), None);
    menu.append(&kill_inactive_item).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    let thermal_item = MenuItem::new(
        if thermal {
            "Thermal: WARNING!"
        } else {
            "Thermal: OK"
        },
        false,
        None,
    );
    menu.append(&thermal_item).unwrap();

    menu.append(&PredefinedMenuItem::separator()).unwrap();

    let uninstall_item = MenuItem::new("Uninstall...", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&uninstall_item).unwrap();
    menu.append(&quit_item).unwrap();

    (
        menu,
        MenuState {
            toggle_item,
            instances_header,
            instance_items,
            inactive_header,
            inactive_items,
            kill_inactive_item,
            thermal_item,
            uninstall_item,
            quit_item,
        },
    )
}

fn cmd_menubar() -> Result<()> {
    if !is_installed() {
        run_first_time_setup()?;
        if !is_installed() {
            return Ok(());
        }
    }

    let mut event_loop = EventLoopBuilder::new().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);

    let initial_instances = get_instance_items();
    let initial_inactive = get_inactive_claude_pids();
    let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);
    let thermal = check_thermal_warning();

    let (menu, mut state) = build_menu(&initial_instances, &initial_inactive, manual_enabled, thermal);

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title(&create_tray_title(initial_instances.len(), manual_enabled))
        .with_tooltip("Claude Code Sleep Preventer")
        .build()?;

    let menu_channel = MenuEvent::receiver();

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

    std::thread::spawn(|| {
        let mut tick_counter = 0u64;

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
        }
    });

    let mut last_update = std::time::Instant::now() - Duration::from_secs(10);
    let mut last_instance_count = initial_instances.len();
    let mut last_inactive_count = initial_inactive.len();

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(std::time::Instant::now() + Duration::from_secs(2));

        if last_update.elapsed() >= Duration::from_secs(2) {
            last_update = std::time::Instant::now();

            let instances = get_instance_items();
            let inactive = get_inactive_claude_pids();
            let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);
            let thermal = check_thermal_warning();

            tray.set_title(Some(&create_tray_title(instances.len(), manual_enabled)));

            if instances.len() != last_instance_count || inactive.len() != last_inactive_count {
                last_instance_count = instances.len();
                last_inactive_count = inactive.len();
                let (new_menu, new_state) = build_menu(&instances, &inactive, manual_enabled, thermal);
                tray.set_menu(Some(Box::new(new_menu)));
                state = new_state;
            } else {
                let toggle_text = if manual_enabled {
                    if instances.is_empty() {
                        "🔵─⚪ Sleep Prevention (Idle)"
                    } else {
                        "🔵─⚪ Sleep Prevention (Working)"
                    }
                } else {
                    "⚪─⚫ Sleep Prevention"
                };
                state.toggle_item.set_text(toggle_text);
                state.thermal_item.set_text(if thermal {
                    "Thermal: WARNING!"
                } else {
                    "Thermal: OK"
                });
                state.instances_header.set_text(if instances.is_empty() {
                    "No Active Instances"
                } else {
                    "Active Instances"
                });
                for (i, (item, stored_pid)) in state.instance_items.iter_mut().enumerate() {
                    if i < instances.len() {
                        let (pid, age, cpu, location) = &instances[i];
                        item.set_text(&format!("  {} - {}s - {:.1}%", location, age, cpu));
                        *stored_pid = *pid;
                    }
                }
                state.inactive_header.set_text(if inactive.is_empty() {
                    "No Inactive Instances"
                } else {
                    "Inactive Instances"
                });
            }
        }

        if let Ok(event) = menu_channel.try_recv() {
            if event.id == state.toggle_item.id() {
                let new_state = !MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);
                MANUAL_SLEEP_PREVENTION.store(new_state, Ordering::SeqCst);
                menubar_sync_sleep();
            } else if event.id == state.kill_inactive_item.id() {
                kill_inactive_claudes();
            } else if event.id == state.uninstall_item.id() {
                let _ = run_uninstall_flow();
                *control_flow = ControlFlow::Exit;
            } else if event.id == state.quit_item.id() {
                release_sleep_assertion();
                *control_flow = ControlFlow::Exit;
            } else {
                for (item, pid) in &state.instance_items {
                    if event.id == item.id() && *pid > 0 {
                        focus_terminal_by_pid(*pid);
                        break;
                    }
                }
                for (item, pid) in &state.inactive_items {
                    if event.id == item.id() && *pid > 0 {
                        if is_option_key_pressed() {
                            unsafe { libc::kill(*pid as i32, libc::SIGTERM); }
                        } else {
                            focus_terminal_by_pid(*pid);
                        }
                        break;
                    }
                }
            }
        }

        // Handle global hotkeys
        if let Ok(event) = hotkey_receiver.try_recv() {
            if event.state == global_hotkey::HotKeyState::Pressed {
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

fn cmd_install() -> Result<()> {
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

    if settings_file.exists() {
        let content = fs::read_to_string(&settings_file)?;
        if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&content) {
            let hooks: serde_json::Value = serde_json::from_str(hooks_json)?;
            json["hooks"] = hooks;
            fs::write(&settings_file, serde_json::to_string_pretty(&json)?)?;
        }
    } else {
        fs::create_dir_all(settings_file.parent().unwrap())?;
        let json = format!(r#"{{"hooks": {}}}"#, hooks_json);
        let parsed: serde_json::Value = serde_json::from_str(&json)?;
        fs::write(&settings_file, serde_json::to_string_pretty(&parsed)?)?;
    }

    Command::new("sudo")
        .args(["pmset", "-a", "sleep", "5"])
        .output()?;
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    println!();
    if ask_yes_no("Launch menu bar app at login?") {
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

fn cmd_uninstall() -> Result<()> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let hooks_dir = home.join(".claude").join("hooks");
    let settings_file = home.join(".claude").join("settings.json");
    let launch_agents_dir = home.join("Library/LaunchAgents");

    let _ = fs::remove_file(hooks_dir.join("prevent-sleep.sh"));
    let _ = fs::remove_file(hooks_dir.join("allow-sleep.sh"));

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

    let plist_path = launch_agents_dir.join("com.charlontank.claude-sleep-preventer.plist");
    if plist_path.exists() {
        let _ = Command::new("launchctl")
            .args(["unload", plist_path.to_str().unwrap()])
            .output();
        let _ = fs::remove_file(&plist_path);
        println!("Removed LaunchAgent");
    }

    Command::new("sudo")
        .args(["rm", "-f", "/etc/sudoers.d/claude-pmset"])
        .output()?;

    let _ = fs::remove_dir_all(PIDS_DIR);

    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    println!("✅ Uninstalled successfully");

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
