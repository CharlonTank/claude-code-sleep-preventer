mod authorization;
mod dictation;
mod logging;
mod native_dialogs;
mod objc_utils;
mod popover;
mod settings;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use core_foundation::base::{kCFAllocatorDefault, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::runloop::{
    kCFRunLoopDefaultMode, CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRun,
};
use core_foundation::string::CFString;
use dictation::{run_onboarding_if_needed, DictationManager};
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager,
};
use io_kit_sys::types::*;
use io_kit_sys::*;
use mach2::port::MACH_PORT_NULL;
use objc::{class, msg_send, sel, sel_impl};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use sysinfo::System;

#[link(name = "IOKit", kind = "framework")]
extern "C" {}
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};

static LID_JUST_CLOSED: AtomicBool = AtomicBool::new(false);
static LID_WAS_CLOSED: AtomicBool = AtomicBool::new(false);
static CURRENT_PID_INDEX: AtomicUsize = AtomicUsize::new(0);
static CURRENT_INACTIVE_INDEX: AtomicUsize = AtomicUsize::new(0);
static MANUAL_SLEEP_PREVENTION: AtomicBool = AtomicBool::new(true);

const PIDS_DIR: &str = "/tmp/agents_working_pids";
const LEGACY_PIDS_DIR: &str = "/tmp/claude_working_pids";
const IDLE_TIMEOUT_SECS: u64 = 30;
const IDLE_CPU_THRESHOLD: f32 = 0.5;
const APP_BINARY_PATH: &str = "/Applications/AgentsSleepPreventer.app/Contents/MacOS/asp";
const OWNED_HOOK_MARKERS: [&str; 4] = [
    "AgentsSleepPreventer.app/Contents/MacOS/asp",
    "/usr/local/bin/asp",
    "/usr/local/bin/agents-sleep-preventer",
    "claude-sleep-preventer",
];

#[derive(Parser)]
#[command(name = "asp")]
#[command(about = "Keep your Mac awake while coding agents are working")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Register current agent process and disable sleep
    Start,
    /// Unregister current agent process and re-enable sleep if no others
    Stop,
    /// Show current status
    Status,
    /// List active/inactive instances as JSON
    List,
    /// Focus an agent instance by PID
    Focus { pid: u32 },
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
    /// Install hooks and configure supported coding agents
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
        /// Keep coding agent hooks
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
        Commands::Uninstall {
            keep_model,
            keep_hooks,
            keep_data,
        } => cmd_uninstall(keep_model, keep_hooks, keep_data)?,
        Commands::Settings => cmd_settings()?,
        Commands::Debug => cmd_debug()?,
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentKind {
    Claude,
    Codex,
}

#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    ppid: u32,
    comm: String,
    args: String,
}

fn load_process_table() -> Vec<ProcessInfo> {
    Command::new("ps")
        .args(["-eo", "pid=,ppid=,comm=,args="])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.lines().filter_map(parse_process_line).collect())
        .unwrap_or_default()
}

fn parse_process_line(line: &str) -> Option<ProcessInfo> {
    let mut parts = line.split_whitespace();
    let pid = parts.next()?.parse().ok()?;
    let ppid = parts.next()?.parse().ok()?;
    let comm = parts.next()?.to_string();
    let args = parts.collect::<Vec<_>>().join(" ");
    Some(ProcessInfo {
        pid,
        ppid,
        comm,
        args,
    })
}

fn executable_basename(token: &str) -> &str {
    token.rsplit('/').next().unwrap_or(token)
}

fn executable_name_is(token: &str, expected: &str) -> bool {
    let basename = executable_basename(token);
    basename == expected || basename == format!("{}.exe", expected)
}

fn process_tokens(process: &ProcessInfo) -> Vec<&str> {
    process.args.split_whitespace().collect()
}

fn codex_command_index(tokens: &[&str]) -> Option<usize> {
    tokens
        .iter()
        .position(|token| executable_name_is(token, "codex"))
}

fn is_codex_app_server(tokens: &[&str]) -> bool {
    codex_command_index(tokens)
        .and_then(|idx| tokens.get(idx + 1))
        .map(|arg| *arg == "app-server")
        .unwrap_or(false)
}

fn is_codex_wrapper_process(process: &ProcessInfo) -> bool {
    let tokens = process_tokens(process);
    tokens
        .first()
        .map(|arg0| executable_name_is(arg0, "node"))
        .unwrap_or(false)
        && tokens
            .get(1)
            .map(|arg1| executable_name_is(arg1, "codex"))
            .unwrap_or(false)
        && !is_codex_app_server(&tokens)
}

fn is_codex_native_process(process: &ProcessInfo) -> bool {
    let tokens = process_tokens(process);
    let arg0 = tokens.first().copied().unwrap_or(&process.comm);
    (executable_name_is(arg0, "codex") || executable_name_is(&process.comm, "codex"))
        && !is_codex_app_server(&tokens)
}

fn classify_agent_process(process: &ProcessInfo) -> Option<AgentKind> {
    let tokens = process_tokens(process);
    let arg0 = tokens.first().copied().unwrap_or(&process.comm);

    if executable_name_is(arg0, "claude") || executable_name_is(&process.comm, "claude") {
        return Some(AgentKind::Claude);
    }

    if is_codex_native_process(process) || is_codex_wrapper_process(process) {
        return Some(AgentKind::Codex);
    }

    None
}

fn find_agent_ancestor() -> Option<u32> {
    let processes = load_process_table();
    let by_pid: HashMap<u32, ProcessInfo> = processes
        .into_iter()
        .map(|process| (process.pid, process))
        .collect();
    let this_pid = std::process::id();
    let mut current_pid = this_pid;

    for _ in 0..20 {
        let Some(process) = by_pid.get(&current_pid) else {
            break;
        };

        if current_pid != this_pid && classify_agent_process(process).is_some() {
            return Some(current_pid);
        }

        if process.ppid == 0 || process.ppid == current_pid {
            break;
        }
        current_pid = process.ppid;
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
    let output = Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", value])
        .output()
        .context("Failed to run pmset")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        logging::log(&format!(
            "[pmset] Failed to set disablesleep={}: {}",
            value,
            stderr.trim()
        ));
    }
    Ok(())
}

fn sleep_prevention_enabled_from_settings() -> bool {
    settings::AppSettings::load().sleep_prevention.enabled
}

fn sync_sleep_state(source: &str, manual_enabled: bool) -> Result<()> {
    let active = count_active_pids();
    let sleep_disabled = is_sleep_disabled();
    let thermal_warning = check_thermal_warning();
    let should_prevent = manual_enabled && active > 0 && !thermal_warning;

    if should_prevent && !sleep_disabled {
        set_sleep_disabled(true)?;
        logging::log(&format!(
            "[{}] Sleep disabled (active PIDs: {})",
            source, active
        ));
    } else if !should_prevent && sleep_disabled {
        enable_sleep_and_trigger_if_lid_closed()?;
        logging::log(&format!("[{}] Sleep re-enabled", source));
    }

    Ok(())
}

fn menubar_sync_sleep() {
    let manual_enabled = MANUAL_SLEEP_PREVENTION.load(Ordering::SeqCst);
    let _ = sync_sleep_state("sync", manual_enabled);
}

fn cleanup_stale_pids() {
    let entries = match fs::read_dir(PIDS_DIR) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut removed = 0;
    let mut total = 0;

    for entry in entries.filter_map(|e| e.ok()) {
        let pid: u32 = match entry.file_name().to_string_lossy().parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        total += 1;
        let path = entry.path();

        if !is_process_alive(pid) {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
            }
            continue;
        }

        let age = get_file_age(&path).unwrap_or(0);
        if age >= IDLE_TIMEOUT_SECS {
            let cpu = get_process_cpu(pid);
            if cpu < IDLE_CPU_THRESHOLD {
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
    }

    if removed > 0 {
        logging::log(&format!(
            "[cleanup] Removed {}/{} stale PIDs",
            removed, total
        ));
    }
}

fn is_sleep_disabled() -> bool {
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
    logging::init_quiet();
    ensure_pids_dir()?;

    let agent_pid = find_agent_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(agent_pid);

    fs::write(&pid_file, "working").context("Failed to write PID file")?;

    sync_sleep_state("hook-start", sleep_prevention_enabled_from_settings())
}

fn cmd_stop() -> Result<()> {
    logging::init_quiet();
    let agent_pid = find_agent_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(agent_pid);

    let _ = fs::remove_file(&pid_file);

    sync_sleep_state("hook-stop", sleep_prevention_enabled_from_settings())
}

fn get_all_agent_processes() -> Vec<ProcessInfo> {
    let processes = load_process_table();
    let codex_native_parents: HashSet<u32> = processes
        .iter()
        .filter(|process| is_codex_native_process(process))
        .map(|process| process.ppid)
        .collect();
    let mut seen_pids = HashSet::new();
    let mut agents = processes
        .into_iter()
        .filter(|process| match classify_agent_process(process) {
            Some(AgentKind::Claude) => true,
            Some(AgentKind::Codex) => {
                !(is_codex_wrapper_process(process) && codex_native_parents.contains(&process.pid))
            }
            None => false,
        })
        .filter(|process| seen_pids.insert(process.pid))
        .collect::<Vec<_>>();
    agents.sort_by_key(|process| process.pid);
    agents
}

fn count_agent_processes() -> usize {
    get_all_agent_processes().len()
}

fn get_all_agent_pids() -> Vec<u32> {
    get_all_agent_processes()
        .into_iter()
        .map(|process| process.pid)
        .collect()
}

fn get_inactive_agent_pids() -> Vec<u32> {
    let all_pids = get_all_agent_pids();
    let active_pids: HashSet<u32> = get_instance_items()
        .iter()
        .map(|(pid, _, _, _)| *pid)
        .collect();
    all_pids
        .into_iter()
        .filter(|pid| !active_pids.contains(pid))
        .collect()
}

fn cmd_status() -> Result<()> {
    let sleep_disabled = is_sleep_disabled();
    let active_count = count_active_pids();
    let thermal_warning = check_thermal_warning();
    let agent_count = count_agent_processes();

    println!("Agents Sleep Preventer v{}", env!("CARGO_PKG_VERSION"));
    println!("==========================================");
    println!("Working instances: {}", active_count);
    println!("Agent processes: {}", agent_count);
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
    let inactive = get_inactive_agent_pids();
    let sleep_disabled = is_sleep_disabled();
    let payload = json!({
        "active": active,
        "inactive": inactive,
        "sleep_disabled": sleep_disabled,
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

fn resolve_user_home() -> Result<PathBuf> {
    if let Ok(sudo_user) = std::env::var("SUDO_USER") {
        let sudo_user = sudo_user.trim();
        if !sudo_user.is_empty() && sudo_user != "root" {
            let user_record = format!("/Users/{}", sudo_user);
            if let Ok(output) = Command::new("dscl")
                .args([".", "-read", &user_record, "NFSHomeDirectory"])
                .output()
            {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    for line in stdout.lines() {
                        if let Some(home) = line.trim().strip_prefix("NFSHomeDirectory:") {
                            let home = home.trim();
                            if !home.is_empty() {
                                return Ok(PathBuf::from(home));
                            }
                        }
                    }
                }
            }
            return Ok(PathBuf::from(user_record));
        }
    }

    dirs::home_dir().context("Could not find home directory")
}

#[cfg(unix)]
fn fix_user_ownership(path: &Path) {
    let Ok(sudo_user) = std::env::var("SUDO_USER") else {
        return;
    };
    let sudo_user = sudo_user.trim();
    if sudo_user.is_empty() || sudo_user == "root" {
        return;
    }
    let Some(path) = path.to_str() else {
        return;
    };
    let _ = Command::new("chown").args(["-R", sudo_user, path]).status();
}

fn toml_section_name(line: &str) -> Option<&str> {
    let code = line.split('#').next().unwrap_or("").trim();
    if !code.starts_with('[') || !code.ends_with(']') {
        return None;
    }
    Some(code.trim_matches(&['[', ']'][..]).trim())
}

fn set_toml_feature_true(content: &str, feature: &str) -> String {
    let mut lines = content.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut features_start = None;
    let mut features_end = lines.len();

    for (idx, line) in lines.iter().enumerate() {
        let Some(section) = toml_section_name(line) else {
            continue;
        };
        if section == "features" {
            features_start = Some(idx);
            features_end = lines.len();
        } else if features_start.is_some() {
            features_end = idx;
            break;
        }
    }

    if let Some(start) = features_start {
        for line in lines.iter_mut().take(features_end).skip(start + 1) {
            let code = line.split('#').next().unwrap_or("").trim_start();
            if let Some(rest) = code.strip_prefix(feature) {
                if rest.trim_start().starts_with('=') {
                    let indent = line
                        .chars()
                        .take_while(|ch| ch.is_whitespace())
                        .collect::<String>();
                    *line = format!("{}{} = true", indent, feature);
                    return format!("{}\n", lines.join("\n"));
                }
            }
        }
        let mut insert_at = features_end;
        while insert_at > start + 1
            && lines
                .get(insert_at - 1)
                .map(|line| line.trim().is_empty())
                .unwrap_or(false)
        {
            insert_at -= 1;
        }
        lines.insert(insert_at, format!("{} = true", feature));
    } else {
        if !lines.is_empty()
            && lines
                .last()
                .map(|line| !line.trim().is_empty())
                .unwrap_or(false)
        {
            lines.push(String::new());
        }
        lines.push("[features]".to_string());
        lines.push(format!("{} = true", feature));
    }

    format!("{}\n", lines.join("\n"))
}

fn remove_toml_feature(content: &str, feature: &str) -> String {
    let mut changed = false;
    let mut in_features = false;
    let mut lines = Vec::new();

    for line in content.lines() {
        if let Some(section) = toml_section_name(line) {
            in_features = section == "features";
        }

        if in_features {
            let code = line.split('#').next().unwrap_or("").trim_start();
            if let Some(rest) = code.strip_prefix(feature) {
                if rest.trim_start().starts_with('=') {
                    changed = true;
                    continue;
                }
            }
        }

        lines.push(line.to_string());
    }

    if changed {
        format!("{}\n", lines.join("\n"))
    } else {
        content.to_string()
    }
}

fn set_codex_hooks_feature(content: &str) -> String {
    let without_legacy = remove_toml_feature(content, "codex_hooks");
    set_toml_feature_true(&without_legacy, "hooks")
}

fn enable_codex_hooks_feature(config_file: &Path) -> Result<()> {
    let content = fs::read_to_string(config_file).unwrap_or_default();
    let updated = set_codex_hooks_feature(&content);
    if updated != content {
        fs::write(config_file, updated)
            .with_context(|| format!("Failed to write {}", config_file.display()))?;
    }
    Ok(())
}

fn hook_value_contains_owned_command(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => OWNED_HOOK_MARKERS
            .iter()
            .any(|marker| text.contains(marker)),
        serde_json::Value::Array(values) => values.iter().any(hook_value_contains_owned_command),
        serde_json::Value::Object(map) => map.values().any(hook_value_contains_owned_command),
        _ => false,
    }
}

fn remove_owned_hooks_from_group(group: &mut serde_json::Value) -> bool {
    let Some(hooks) = group
        .get_mut("hooks")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };

    let before = hooks.len();
    hooks.retain(|hook| !hook_value_contains_owned_command(hook));
    before != hooks.len()
}

fn remove_owned_hook_groups(hooks: &mut serde_json::Value) -> bool {
    let Some(events) = hooks.as_object_mut() else {
        return false;
    };

    let mut changed = false;
    for groups in events.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };

        for group in groups.iter_mut() {
            if remove_owned_hooks_from_group(group) {
                changed = true;
            }
        }

        let before = groups.len();
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(serde_json::Value::as_array)
                .map(|hooks| !hooks.is_empty())
                .unwrap_or(true)
        });
        changed |= before != groups.len();
    }

    changed
}

fn prune_empty_hook_events(hooks: &mut serde_json::Value) {
    if let Some(events) = hooks.as_object_mut() {
        events.retain(|_, groups| {
            groups
                .as_array()
                .map(|groups| !groups.is_empty())
                .unwrap_or(true)
        });
    }
}

fn command_hook_group(command: &str, matcher: Option<&str>) -> serde_json::Value {
    let mut group = json!({
        "hooks": [
            {
                "type": "command",
                "command": command,
                "timeout": 5
            }
        ]
    });
    if let Some(matcher) = matcher {
        group["matcher"] = json!(matcher);
    }
    group
}

fn append_codex_hook_group(
    hooks: &mut serde_json::Map<String, serde_json::Value>,
    event_name: &str,
    group: serde_json::Value,
) {
    let event = hooks
        .entry(event_name.to_string())
        .or_insert_with(|| json!([]));
    if !event.is_array() {
        *event = json!([]);
    }
    if let Some(groups) = event.as_array_mut() {
        groups.push(group);
    }
}

fn install_codex_hooks(home: &Path, app_binary: &str) -> Result<()> {
    let codex_dir = home.join(".codex");
    fs::create_dir_all(&codex_dir)
        .with_context(|| format!("Failed to create {}", codex_dir.display()))?;

    let config_file = codex_dir.join("config.toml");
    enable_codex_hooks_feature(&config_file)?;

    let hooks_file = codex_dir.join("hooks.json");
    let mut hooks_json = if hooks_file.exists() {
        let content = fs::read_to_string(&hooks_file)
            .with_context(|| format!("Failed to read {}", hooks_file.display()))?;
        serde_json::from_str::<serde_json::Value>(&content)
            .with_context(|| format!("Failed to parse {}", hooks_file.display()))?
    } else {
        json!({})
    };

    if !hooks_json.is_object() {
        hooks_json = json!({});
    }
    if !hooks_json
        .get("hooks")
        .map(serde_json::Value::is_object)
        .unwrap_or(false)
    {
        hooks_json["hooks"] = json!({});
    }

    if let Some(hooks) = hooks_json.get_mut("hooks") {
        remove_owned_hook_groups(hooks);
        prune_empty_hook_events(hooks);
    }

    let start_command =
        format!("[ -x \"{app_binary}\" ] && \"{app_binary}\" start 2>/dev/null || true");
    let stop_command =
        format!("[ -x \"{app_binary}\" ] && \"{app_binary}\" stop 2>/dev/null || true");

    let hooks = hooks_json
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
        .context("Failed to prepare Codex hooks object")?;
    append_codex_hook_group(
        hooks,
        "UserPromptSubmit",
        command_hook_group(&start_command, None),
    );
    append_codex_hook_group(
        hooks,
        "PreToolUse",
        command_hook_group(&start_command, Some("*")),
    );
    append_codex_hook_group(
        hooks,
        "PostToolUse",
        command_hook_group(&start_command, Some("*")),
    );
    append_codex_hook_group(hooks, "Stop", command_hook_group(&stop_command, None));

    fs::write(&hooks_file, serde_json::to_string_pretty(&hooks_json)?)
        .with_context(|| format!("Failed to write {}", hooks_file.display()))?;

    #[cfg(unix)]
    fix_user_ownership(&codex_dir);

    println!("  Updated {}", config_file.display());
    println!("  Updated {}", hooks_file.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_codex_hooks_feature_adds_current_flag() {
        let updated = set_codex_hooks_feature("model = \"gpt-5.5\"\n");

        assert_eq!(updated, "model = \"gpt-5.5\"\n\n[features]\nhooks = true\n");
    }

    #[test]
    fn set_codex_hooks_feature_replaces_deprecated_flag() {
        let config = "\
model = \"gpt-5.5\"

[features]
unified_exec = true
codex_hooks = true

[plugins.github]
enabled = true
";

        let updated = set_codex_hooks_feature(config);

        assert!(!updated.contains("codex_hooks"));
        assert!(updated.contains("[features]\nunified_exec = true\nhooks = true"));
        assert!(updated.contains("[plugins.github]\nenabled = true"));
    }

    #[test]
    fn set_codex_hooks_feature_updates_existing_hooks_flag() {
        let config = "\
[features]
hooks = false
";

        let updated = set_codex_hooks_feature(config);

        assert_eq!(updated, "[features]\nhooks = true\n");
    }
}

fn remove_codex_hooks(home: &Path) -> Result<bool> {
    let hooks_file = home.join(".codex/hooks.json");
    if !hooks_file.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&hooks_file)
        .with_context(|| format!("Failed to read {}", hooks_file.display()))?;
    let Ok(mut hooks_json) = serde_json::from_str::<serde_json::Value>(&content) else {
        eprintln!(
            "Warning: could not parse {}, leaving it unchanged",
            hooks_file.display()
        );
        return Ok(false);
    };

    let changed = hooks_json
        .get_mut("hooks")
        .map(remove_owned_hook_groups)
        .unwrap_or(false);
    if !changed {
        return Ok(false);
    }

    if let Some(hooks) = hooks_json.get_mut("hooks") {
        prune_empty_hook_events(hooks);
    }

    if let Some(root) = hooks_json.as_object_mut() {
        let hooks_empty = root
            .get("hooks")
            .and_then(serde_json::Value::as_object)
            .map(|hooks| hooks.is_empty())
            .unwrap_or(false);
        if hooks_empty {
            root.remove("hooks");
        }
        if root.is_empty() {
            fs::remove_file(&hooks_file)
                .with_context(|| format!("Failed to remove {}", hooks_file.display()))?;
            return Ok(true);
        }
    }

    fs::write(&hooks_file, serde_json::to_string_pretty(&hooks_json)?)
        .with_context(|| format!("Failed to write {}", hooks_file.display()))?;

    Ok(true)
}

fn is_codex_hooks_installed(home: &Path) -> bool {
    let hooks_file = home.join(".codex/hooks.json");
    fs::read_to_string(hooks_file)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .map(|hooks_json| hook_value_contains_owned_command(&hooks_json))
        .unwrap_or(false)
}

fn is_claude_hooks_installed(home: &Path) -> bool {
    home.join(".claude/hooks/prevent-sleep.sh").exists()
}

fn is_installed() -> bool {
    let home = resolve_user_home().unwrap_or_default();
    is_claude_hooks_installed(&home) && is_codex_hooks_installed(&home)
}

fn run_first_time_setup() -> Result<()> {
    let message =
        "Agents Sleep Preventer needs to be configured to work with Claude Code and Codex.

This will:
• Install the CLI tool
• Configure coding agent hooks
• Set up automatic startup

Administrator password required.";

    if !native_dialogs::show_confirm_dialog(message, "Agents Sleep Preventer", "Set Up", "Cancel") {
        return Ok(());
    }

    let script = "/Applications/AgentsSleepPreventer.app/Contents/MacOS/asp install -y";

    match authorization::execute_script_with_privileges(script) {
        Ok(true) => {
            native_dialogs::show_dialog(
                "Setup complete!\n\nRestart Claude Code or Codex to activate sleep prevention.",
                "Agents Sleep Preventer",
            );
            relaunch_app_after_install();
        }
        Ok(false) => {
            // User cancelled
        }
        Err(e) => {
            native_dialogs::show_dialog(&format!("Setup failed: {}", e), "Agents Sleep Preventer");
        }
    }

    Ok(())
}

fn relaunch_app_after_install() {
    logging::log("[main] Relaunching app after install...");
    match Command::new("open")
        .args(["-n", "/Applications/AgentsSleepPreventer.app"])
        .status()
    {
        Ok(status) if status.success() => {
            std::process::exit(0);
        }
        Ok(status) => {
            logging::log(&format!("[main] Relaunch failed with status: {}", status));
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
        .with_tooltip("Agents Sleep Preventer")
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
                    let _ = set_sleep_disabled(false);
                    logging::log("[thermal] Sleep re-enabled due to thermal warning");
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
                    MANUAL_SLEEP_PREVENTION
                        .store(new_settings.sleep_prevention.enabled, Ordering::SeqCst);
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
                        inactive: get_inactive_agent_pids(),
                        thermal_warning: check_thermal_warning(),
                        dictation_enabled: dictation_manager.is_enabled(),
                        dictation_available: dictation_manager.is_available(),
                        sleep_disabled: is_sleep_disabled(),
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
                        let idx =
                            CURRENT_PID_INDEX.fetch_add(1, Ordering::SeqCst) % instances.len();
                        focus_terminal_by_pid(instances[idx].0);
                    }
                } else if event.id == hotkey_inactive_id {
                    let inactive = get_inactive_agent_pids();
                    if !inactive.is_empty() {
                        let idx =
                            CURRENT_INACTIVE_INDEX.fetch_add(1, Ordering::SeqCst) % inactive.len();
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

    // Load settings and initialize sleep prevention state
    let app_settings = settings::AppSettings::load();
    MANUAL_SLEEP_PREVENTION.store(app_settings.sleep_prevention.enabled, Ordering::SeqCst);
    logging::log(&format!(
        "[agent] Loaded settings: sleep_prevention={}",
        app_settings.sleep_prevention.enabled
    ));

    start_clamshell_notifications();

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

    let mut tick_counter = 0u64;

    loop {
        dictation_manager.update();
        objc_utils::pump_run_loop_once();
        std::thread::sleep(Duration::from_millis(50));
        tick_counter += 1;

        // Every 1s: cleanup stale PIDs and sync sleep state
        if tick_counter % 20 == 0 {
            cleanup_stale_pids();
            menubar_sync_sleep();
        }

        // Every 3s: thermal check + lid close
        if tick_counter % 60 == 0 {
            if check_thermal_warning() {
                let _ = set_sleep_disabled(false);
                logging::log("[agent][thermal] Sleep re-enabled due to thermal warning");
            }

            if LID_JUST_CLOSED.swap(false, Ordering::SeqCst) {
                let active = count_active_pids();
                if active > 0 {
                    play_lid_close_sound();
                }
            }
        }

        // Every 30s: reload settings from disk
        if tick_counter % 600 == 0 {
            let new_settings = settings::AppSettings::load();
            MANUAL_SLEEP_PREVENTION.store(new_settings.sleep_prevention.enabled, Ordering::SeqCst);
        }
    }
}

fn acquire_agent_lock() -> Result<Option<std::fs::File>> {
    let lock_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("AgentsSleepPreventer");
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

fn sync_installed_cli() -> Result<bool> {
    let current_exe = std::env::current_exe().context("Could not find current executable")?;
    fs::create_dir_all("/usr/local/bin")?;

    let mut updated = false;
    for target in [
        Path::new("/usr/local/bin/asp"),
        Path::new("/usr/local/bin/agents-sleep-preventer"),
    ] {
        if current_exe == target {
            continue;
        }

        fs::copy(&current_exe, target).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                current_exe.display(),
                target.display()
            )
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(target, fs::Permissions::from_mode(0o755))?;
        }
        updated = true;
    }

    let _ = fs::remove_file("/usr/local/bin/claude-sleep-preventer");

    Ok(updated)
}

fn cmd_install(auto_yes: bool) -> Result<()> {
    let home = resolve_user_home()?;
    let hooks_dir = home.join(".claude").join("hooks");
    let settings_file = home.join(".claude").join("settings.json");
    let launch_agents_dir = home.join("Library/LaunchAgents");

    match sync_installed_cli() {
        Ok(true) => println!("Updated /usr/local/bin/asp"),
        Ok(false) => {}
        Err(e) => eprintln!("Warning: could not update /usr/local/bin/asp: {}", e),
    }

    fs::create_dir_all(&hooks_dir)?;

    let prevent_script = format!(
        "#!/bin/bash\n[ -x \"{}\" ] && \"{}\" start 2>/dev/null || true\n",
        APP_BINARY_PATH, APP_BINARY_PATH
    );
    let allow_script = format!(
        "#!/bin/bash\n[ -x \"{}\" ] && \"{}\" stop 2>/dev/null || true\n",
        APP_BINARY_PATH, APP_BINARY_PATH
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

        fix_user_ownership(&hooks_dir);
    }

    println!("Setting up passwordless sudo for pmset...");
    // Get the real user (not root) for sudoers entry
    let real_user = std::env::var("SUDO_USER")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let sudoers_content = format!("{} ALL=(ALL) NOPASSWD: /usr/bin/pmset\n", real_user);

    // Write directly if we're root, otherwise use sudo
    let is_root = unsafe { libc::geteuid() == 0 };
    let mut child = if is_root {
        Command::new("tee")
            .arg("/etc/sudoers.d/agents-pmset")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()?
    } else {
        Command::new("sudo")
            .args(["tee", "/etc/sudoers.d/agents-pmset"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()?
    };

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(sudoers_content.as_bytes())?;
    }
    child.wait()?;

    if is_root {
        Command::new("chmod")
            .args(["440", "/etc/sudoers.d/agents-pmset"])
            .output()?;
        let _ = Command::new("rm")
            .args(["-f", "/etc/sudoers.d/claude-pmset"])
            .output();
    } else {
        Command::new("sudo")
            .args(["chmod", "440", "/etc/sudoers.d/agents-pmset"])
            .output()?;
        let _ = Command::new("sudo")
            .args(["rm", "-f", "/etc/sudoers.d/claude-pmset"])
            .output();
    }

    println!("Configuring Claude Code hooks...");

    let prevent_path = hooks_dir.join("prevent-sleep.sh");
    let allow_path = hooks_dir.join("allow-sleep.sh");
    let hooks_json = format!(
        r#"{{
    "UserPromptSubmit": [{{ "hooks": [{{ "type": "command", "command": "{prevent}" }}] }}],
    "PreToolUse": [{{ "hooks": [{{ "type": "command", "command": "{prevent}" }}] }}],
    "PreCompact": [{{ "hooks": [{{ "type": "command", "command": "{prevent}" }}] }}],
    "Stop": [{{ "hooks": [{{ "type": "command", "command": "{allow}" }}] }}]
}}"#,
        prevent = prevent_path.display(),
        allow = allow_path.display(),
    );

    let hooks: serde_json::Value =
        serde_json::from_str(&hooks_json).context("Failed to parse hooks JSON")?;

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

    #[cfg(unix)]
    fix_user_ownership(&settings_file);

    println!("Configuring Codex hooks...");
    install_codex_hooks(&home, APP_BINARY_PATH)?;

    if is_root {
        Command::new("pmset").args(["-a", "sleep", "5"]).output()?;
        Command::new("pmset")
            .args(["-a", "disablesleep", "0"])
            .output()?;
    } else {
        Command::new("sudo")
            .args(["pmset", "-a", "sleep", "5"])
            .output()?;
        Command::new("sudo")
            .args(["pmset", "-a", "disablesleep", "0"])
            .output()?;
    }

    println!();
    if auto_yes || ask_yes_no("Launch menu bar app at login?") {
        fs::create_dir_all(&launch_agents_dir)?;

        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.charlontank.agents-sleep-preventer</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/bin/open</string>
        <string>/Applications/AgentsSleepPreventer.app</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <false/>
</dict>
</plist>"#;

        let plist_path = launch_agents_dir.join("com.charlontank.agents-sleep-preventer.plist");
        fs::write(&plist_path, plist)?;

        println!("  Created LaunchAgent for login startup");
        println!("  Note: Copy AgentsSleepPreventer.app to /Applications");
    }

    println!("\n✅ Installation complete!");
    println!("\nRestart Claude Code or Codex to activate.");
    println!("\nCommands:");
    println!("  asp status   - Show current state");
    println!("  asp cleanup  - Clean up stale PIDs");
    println!("  asp reset    - Force enable sleep");
    println!("  asp menubar  - Run native menu bar");
    println!("  asp daemon   - Run background daemon");

    // Try to launch the app
    let _ = Command::new("open")
        .arg("/Applications/AgentsSleepPreventer.app")
        .spawn();

    Ok(())
}

fn cmd_uninstall(keep_model: bool, keep_hooks: bool, keep_data: bool) -> Result<()> {
    let home = resolve_user_home()?;
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
                        let _ =
                            fs::write(&settings_file, serde_json::to_string_pretty(&json).unwrap());
                        println!("Removed hooks from settings.json");
                    }
                }
            }
        }
        if remove_codex_hooks(&home)? {
            println!("Removed Codex hooks");
        }
        println!("Removed coding agent hooks");
    }

    // Remove LaunchAgents
    for label in [
        "com.charlontank.agents-sleep-preventer.plist",
        "com.charlontank.claude-sleep-preventer.plist",
    ] {
        let plist_path = launch_agents_dir.join(label);
        if plist_path.exists() {
            let _ = Command::new("launchctl")
                .args(["unload", plist_path.to_str().unwrap()])
                .output();
            let _ = fs::remove_file(&plist_path);
            println!("Removed LaunchAgent {}", label);
        }
    }

    // Remove sudoers config
    for sudoers_path in ["/etc/sudoers.d/agents-pmset", "/etc/sudoers.d/claude-pmset"] {
        Command::new("sudo")
            .args(["rm", "-f", sudoers_path])
            .output()?;
    }

    // Remove PID tracking directory
    let _ = fs::remove_dir_all(PIDS_DIR);
    let _ = fs::remove_dir_all(LEGACY_PIDS_DIR);

    // Reset sleep settings
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    // Remove app data and preferences (unless keeping data)
    if !keep_data {
        for app_support in [
            home.join("Library/Application Support/AgentsSleepPreventer"),
            home.join("Library/Application Support/ClaudeSleepPreventer"),
        ] {
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
        }

        // Remove logs
        let _ = fs::remove_dir_all(home.join("Library/Logs/AgentsSleepPreventer"));
        let _ = fs::remove_dir_all(home.join("Library/Logs/ClaudeSleepPreventer"));
        println!("Removed logs");
    }

    // Remove the app from /Applications
    Command::new("sudo")
        .args([
            "rm",
            "-rf",
            "/Applications/AgentsSleepPreventer.app",
            "/Applications/ClaudeSleepPreventer.app",
        ])
        .output()?;
    println!("Removed app from /Applications");

    let _ = fs::remove_file("/usr/local/bin/asp");
    let _ = fs::remove_file("/usr/local/bin/agents-sleep-preventer");
    let _ = fs::remove_file("/usr/local/bin/claude-sleep-preventer");

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
            new_settings.sleep_prevention.enabled, new_settings.speech_to_text.language
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
        let lower = name.to_lowercase();
        if lower.contains("claude") || lower.contains("codex") {
            println!("  PID {}: name={:?}", pid.as_u32(), name);
        }
    }

    println!("\nps command:");
    let output = Command::new("ps")
        .args(["-eo", "pid=,ppid=,comm=,args="])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains("claude") || lower.contains("codex") {
            println!("  {}", line.trim());
        }
    }

    println!("\nDetected agent PIDs:");
    for process in get_all_agent_processes() {
        let kind = match classify_agent_process(&process) {
            Some(AgentKind::Claude) => "claude",
            Some(AgentKind::Codex) => "codex",
            None => "unknown",
        };
        println!(
            "  PID {}: kind={}, ppid={}, comm={}, args={}",
            process.pid, kind, process.ppid, process.comm, process.args
        );
    }

    Ok(())
}
