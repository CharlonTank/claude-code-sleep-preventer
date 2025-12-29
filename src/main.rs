use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use sysinfo::{Pid, ProcessesToUpdate, System};

const PIDS_DIR: &str = "/tmp/claude_working_pids";
const GRACE_PERIOD_SECS: u64 = 10;
const CPU_IDLE_THRESHOLD: f32 = 1.0;

#[derive(Parser)]
#[command(name = "claude-sleep-preventer")]
#[command(about = "Keep your Mac awake while Claude Code is working")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
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
    /// Run as daemon, monitoring and cleaning up stale PIDs
    Daemon {
        /// Check interval in seconds
        #[arg(short, long, default_value = "1")]
        interval: u64,
    },
    /// Install hooks and configure Claude Code
    Install,
    /// Uninstall hooks and restore defaults
    Uninstall,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Start => cmd_start()?,
        Commands::Stop => cmd_stop()?,
        Commands::Status => cmd_status()?,
        Commands::Cleanup => cmd_cleanup()?,
        Commands::Daemon { interval } => cmd_daemon(interval)?,
        Commands::Install => cmd_install()?,
        Commands::Uninstall => cmd_uninstall()?,
    }

    Ok(())
}

fn get_parent_pid() -> u32 {
    std::os::unix::process::parent_id()
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

fn cmd_start() -> Result<()> {
    ensure_pids_dir()?;

    let ppid = get_parent_pid();
    let pid_file = get_pid_file(ppid);

    // Create/touch the PID file
    fs::write(&pid_file, "working").context("Failed to write PID file")?;

    // If this is the first working instance, disable sleep
    if count_active_pids() == 1 {
        set_sleep_disabled(true)?;
    }

    Ok(())
}

fn cmd_stop() -> Result<()> {
    let ppid = get_parent_pid();
    let pid_file = get_pid_file(ppid);

    // Remove PID file
    let _ = fs::remove_file(&pid_file);

    // If no more working instances, enable sleep
    if count_active_pids() == 0 {
        set_sleep_disabled(false)?;
    }

    Ok(())
}

fn cmd_status() -> Result<()> {
    let sleep_disabled = is_sleep_disabled();
    let active_count = count_active_pids();

    // Count running claude processes
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All);
    let claude_count = sys
        .processes()
        .values()
        .filter(|p| p.name().to_string_lossy().contains("claude"))
        .count();

    println!("Claude Code Sleep Preventer");
    println!("===========================");
    println!("Working instances: {}", active_count);
    println!("Total Claude processes: {}", claude_count);
    println!(
        "Sleep disabled: {}",
        if sleep_disabled { "Yes" } else { "No" }
    );

    if active_count > 0 {
        println!("\nActive PIDs:");
        if let Ok(entries) = fs::read_dir(PIDS_DIR) {
            for entry in entries.filter_map(|e| e.ok()) {
                let pid: u32 = entry.file_name().to_string_lossy().parse().unwrap_or(0);
                if pid > 0 {
                    let age = get_file_age(&entry.path()).unwrap_or(0);
                    let cpu = get_process_cpu(pid);
                    println!("  PID {}: age={}s, cpu={:.1}%", pid, age, cpu);
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
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All);

    sys.process(Pid::from_u32(pid))
        .map(|p| p.cpu_usage())
        .unwrap_or(0.0)
}

fn is_process_alive(pid: u32) -> bool {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All);
    sys.process(Pid::from_u32(pid)).is_some()
}

fn cmd_cleanup() -> Result<()> {
    let mut cleaned = 0;

    if let Ok(entries) = fs::read_dir(PIDS_DIR) {
        for entry in entries.filter_map(|e| e.ok()) {
            let pid: u32 = match entry.file_name().to_string_lossy().parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let path = entry.path();

            // Remove if process doesn't exist
            if !is_process_alive(pid) {
                let _ = fs::remove_file(&path);
                cleaned += 1;
                continue;
            }

            // Check age and CPU for idle detection
            let age = get_file_age(&path).unwrap_or(0);
            if age >= GRACE_PERIOD_SECS {
                let cpu = get_process_cpu(pid);
                if cpu < CPU_IDLE_THRESHOLD {
                    let _ = fs::remove_file(&path);
                    cleaned += 1;
                }
            }
        }
    }

    // Fix sleep state if needed
    let active = count_active_pids();
    let sleep_disabled = is_sleep_disabled();

    if active > 0 && !sleep_disabled {
        set_sleep_disabled(true)?;
    } else if active == 0 && sleep_disabled {
        set_sleep_disabled(false)?;
    }

    if cleaned > 0 {
        eprintln!("Cleaned up {} stale PID(s)", cleaned);
    }

    Ok(())
}

fn cmd_daemon(interval: u64) -> Result<()> {
    println!("Starting daemon (interval: {}s)...", interval);
    println!("Press Ctrl+C to stop");

    loop {
        cmd_cleanup()?;
        std::thread::sleep(Duration::from_secs(interval));
    }
}

fn cmd_install() -> Result<()> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let hooks_dir = home.join(".claude").join("hooks");
    let settings_file = home.join(".claude").join("settings.json");

    // Create hooks directory
    fs::create_dir_all(&hooks_dir)?;

    // Get path to this binary
    let bin_path = std::env::current_exe()?;
    let bin_str = bin_path.to_string_lossy();

    // Create wrapper scripts that call our binary
    let prevent_script = format!("#!/bin/bash\n\"{}\" start\n", bin_str);
    let allow_script = format!("#!/bin/bash\n\"{}\" stop\n", bin_str);

    fs::write(hooks_dir.join("prevent-sleep.sh"), &prevent_script)?;
    fs::write(hooks_dir.join("allow-sleep.sh"), &allow_script)?;

    // Make executable
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

    // Set up passwordless sudo
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

    // Update settings.json with hooks
    println!("Configuring Claude Code hooks...");

    let hooks_json = r#"{
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "PreToolUse": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "PreCompact": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/prevent-sleep.sh" }] }],
    "Stop": [{ "hooks": [{ "type": "command", "command": "$HOME/.claude/hooks/allow-sleep.sh" }] }]
}"#;

    if settings_file.exists() {
        // Read existing settings and merge hooks
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

    // Set default sleep timeout
    Command::new("sudo")
        .args(["pmset", "-a", "sleep", "5"])
        .output()?;
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    println!("\n✅ Installation complete!");
    println!("\nRestart Claude Code to activate.");
    println!("\nCommands:");
    println!("  claude-sleep-preventer status   - Show current state");
    println!("  claude-sleep-preventer cleanup  - Clean up stale PIDs");
    println!("  claude-sleep-preventer daemon   - Run cleanup daemon");

    Ok(())
}

fn cmd_uninstall() -> Result<()> {
    let home = dirs::home_dir().context("Could not find home directory")?;
    let hooks_dir = home.join(".claude").join("hooks");

    // Remove hook scripts
    let _ = fs::remove_file(hooks_dir.join("prevent-sleep.sh"));
    let _ = fs::remove_file(hooks_dir.join("allow-sleep.sh"));

    // Remove sudoers file
    Command::new("sudo")
        .args(["rm", "-f", "/etc/sudoers.d/claude-pmset"])
        .output()?;

    // Clean up PIDs
    let _ = fs::remove_dir_all(PIDS_DIR);

    // Re-enable sleep
    Command::new("sudo")
        .args(["pmset", "-a", "disablesleep", "0"])
        .output()?;

    println!("✅ Uninstalled successfully");
    println!("\nNote: Remove hooks from ~/.claude/settings.json manually if needed");

    Ok(())
}
