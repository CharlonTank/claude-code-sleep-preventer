use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use sysinfo::System;
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    TrayIconBuilder,
};

const PIDS_DIR: &str = "/tmp/claude_working_pids";
const GRACE_PERIOD_SECS: u64 = 10;
const CPU_IDLE_THRESHOLD: f32 = 1.0;

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
            let parent_comm = String::from_utf8_lossy(&parent_output.stdout).trim().to_string();

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

fn is_sleep_disabled() -> bool {
    Command::new("pmset")
        .arg("-g")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.contains("SleepDisabled\t\t1"))
        .unwrap_or(false)
}

fn check_thermal_warning() -> bool {
    Command::new("pmset")
        .args(["-g", "therm"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| {
            (s.contains("CPU_Scheduler_Limit") && !s.contains("No CPU")) ||
            (s.contains("thermal warning level") && !s.contains("No thermal warning"))
        })
        .unwrap_or(false)
}

fn cmd_start() -> Result<()> {
    ensure_pids_dir()?;

    let claude_pid = find_claude_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(claude_pid);

    fs::write(&pid_file, "working").context("Failed to write PID file")?;

    if !is_sleep_disabled() {
        set_sleep_disabled(true)?;
    }

    Ok(())
}

fn cmd_stop() -> Result<()> {
    let claude_pid = find_claude_ancestor().unwrap_or(std::process::id());
    let pid_file = get_pid_file(claude_pid);

    let _ = fs::remove_file(&pid_file);

    if count_active_pids() == 0 {
        set_sleep_disabled(false)?;
    }

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

fn cmd_status() -> Result<()> {
    let sleep_disabled = is_sleep_disabled();
    let active_count = count_active_pids();
    let thermal_warning = check_thermal_warning();
    let claude_count = count_claude_processes();

    println!("Claude Code Sleep Preventer v{}", env!("CARGO_PKG_VERSION"));
    println!("==========================================");
    println!("Working instances: {}", active_count);
    println!("Claude processes: {}", claude_count);
    println!("Sleep disabled: {}", if sleep_disabled { "Yes" } else { "No" });
    println!("Thermal warning: {}", if thermal_warning { "YES!" } else { "No" });

    if active_count > 0 {
        println!("\nActive PIDs:");
        if let Ok(entries) = fs::read_dir(PIDS_DIR) {
            for entry in entries.filter_map(|e| e.ok()) {
                let pid: u32 = entry.file_name().to_string_lossy().parse().unwrap_or(0);
                if pid > 0 {
                    let age = get_file_age(&entry.path()).unwrap_or(0);
                    let cpu = get_process_cpu(pid);
                    let alive = is_process_alive(pid);
                    println!("  PID {}: age={}s, cpu={:.1}%, alive={}", pid, age, cpu, alive);
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
    Command::new("ps")
        .args(["-p", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn is_lid_open() -> bool {
    Command::new("ioreg")
        .args(["-r", "-k", "AppleClamshellState", "-d", "4"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| !s.contains("\"AppleClamshellState\" = Yes"))
        .unwrap_or(true)
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
            if age >= GRACE_PERIOD_SECS {
                let cpu = get_process_cpu(pid);
                if cpu < CPU_IDLE_THRESHOLD {
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
        set_sleep_disabled(false)?;
    }

    Ok(())
}

fn cmd_reset() -> Result<()> {
    let _ = fs::remove_dir_all(PIDS_DIR);
    let _ = fs::create_dir_all(PIDS_DIR);
    set_sleep_disabled(false)?;
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
    eprintln!("Daemon started (interval: {}s, thermal check: 30s)", interval);

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

fn create_tray_title(count: usize, sleep_disabled: bool) -> String {
    if count > 0 || sleep_disabled {
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

    let install_result = Command::new("osascript")
        .args(["-e", script])
        .output()?;

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

    let _ = Command::new("osascript")
        .args(["-e", script])
        .output();

    let _ = Command::new("osascript")
        .args([
            "-e",
            r#"display dialog "Uninstall complete.

You can delete the app from /Applications manually." buttons {"OK"} default button "OK" with title "Uninstall" with icon note"#,
        ])
        .output();

    Ok(())
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

    let menu = Menu::new();
    let status_item = MenuItem::new("Loading...", false, None);
    let thermal_item = MenuItem::new("Thermal: OK", false, None);
    let sep1 = PredefinedMenuItem::separator();
    let cleanup_item = MenuItem::new("Cleanup Now", true, None);
    let reset_item = MenuItem::new("Force Enable Sleep", true, None);
    let sep2 = PredefinedMenuItem::separator();
    let uninstall_item = MenuItem::new("Uninstall...", true, None);
    let quit_item = MenuItem::new("Quit", true, None);

    menu.append(&status_item)?;
    menu.append(&thermal_item)?;
    menu.append(&sep1)?;
    menu.append(&cleanup_item)?;
    menu.append(&reset_item)?;
    menu.append(&sep2)?;
    menu.append(&uninstall_item)?;
    menu.append(&quit_item)?;

    let count = count_active_pids();
    let sleep_disabled = is_sleep_disabled();

    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_title(&create_tray_title(count, sleep_disabled))
        .with_tooltip("Claude Code Sleep Preventer")
        .build()?;

    let menu_channel = MenuEvent::receiver();
    let cleanup_id = cleanup_item.id().clone();
    let reset_id = reset_item.id().clone();
    let uninstall_id = uninstall_item.id().clone();
    let quit_id = quit_item.id().clone();

    std::thread::spawn(|| {
        let mut thermal_counter = 0;
        let mut lid_was_open = true;

        loop {
            std::thread::sleep(Duration::from_secs(10));
            let _ = cmd_cleanup();

            thermal_counter += 1;
            if thermal_counter >= 3 {
                thermal_counter = 0;
                if check_thermal_warning() {
                    let _ = cmd_reset();
                }
            }

            let lid_open = is_lid_open();
            let active = count_active_pids();

            if lid_was_open && !lid_open && active > 0 {
                play_lid_close_sound();
            }
            lid_was_open = lid_open;
        }
    });

    let mut last_update = std::time::Instant::now() - Duration::from_secs(10);

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_secs(10)
        );

        // Only update display every 10 seconds
        if last_update.elapsed() >= Duration::from_secs(10) {
            last_update = std::time::Instant::now();

            let count = count_active_pids();
            let sleep_disabled = is_sleep_disabled();
            let thermal = check_thermal_warning();

            let status = if count > 0 {
                format!("{} instance(s) - Sleep disabled", count)
            } else if sleep_disabled {
                "Sleep stuck disabled!".to_string()
            } else {
                "Sleep enabled".to_string()
            };
            status_item.set_text(&status);

            thermal_item.set_text(if thermal { "Thermal: WARNING!" } else { "Thermal: OK" });

            _tray.set_title(Some(&create_tray_title(count, sleep_disabled)));
        }

        // Handle menu events
        if let Ok(event) = menu_channel.try_recv() {
            if event.id == cleanup_id {
                let _ = cmd_cleanup();
            } else if event.id == reset_id {
                let _ = cmd_reset();
            } else if event.id == uninstall_id {
                let _ = run_uninstall_flow();
                *control_flow = ControlFlow::Exit;
            } else if event.id == quit_id {
                *control_flow = ControlFlow::Exit;
            }
        }
    });
}

fn ask_yes_no(prompt: &str) -> bool {
    use std::io::{self, BufRead};
    print!("{} [Y/n]: ", prompt);
    let _ = io::Write::flush(&mut io::stdout());
    let stdin = io::stdin();
    let answer = stdin.lock().lines().next()
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
    let prevent_script = format!("#!/bin/bash\n[ -x \"{}\" ] && \"{}\" start 2>/dev/null || true\n", app_binary, app_binary);
    let allow_script = format!("#!/bin/bash\n[ -x \"{}\" ] && \"{}\" stop 2>/dev/null || true\n", app_binary, app_binary);

    fs::write(hooks_dir.join("prevent-sleep.sh"), prevent_script)?;
    fs::write(hooks_dir.join("allow-sleep.sh"), allow_script)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hooks_dir.join("prevent-sleep.sh"), fs::Permissions::from_mode(0o755))?;
        fs::set_permissions(hooks_dir.join("allow-sleep.sh"), fs::Permissions::from_mode(0o755))?;
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

    Command::new("sudo").args(["pmset", "-a", "sleep", "5"]).output()?;
    Command::new("sudo").args(["pmset", "-a", "disablesleep", "0"]).output()?;

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
    let output = Command::new("ps")
        .args(["-eo", "pid,comm"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.to_lowercase().contains("claude") {
            println!("  {}", line.trim());
        }
    }

    Ok(())
}
