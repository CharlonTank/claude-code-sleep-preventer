use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GlobeKeyEvent {
    Ready,
    DictateStart,
    DictateStop,
}

pub struct GlobeKeyManager {
    child: Option<Child>,
    event_rx: Option<Receiver<GlobeKeyEvent>>,
    last_error: Option<String>,
}

impl GlobeKeyManager {
    pub fn new() -> Self {
        Self {
            child: None,
            event_rx: None,
            last_error: None,
        }
    }

    pub fn start(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            return Ok(());
        }

        let binary_path = Self::find_binary()?;
        let (tx, rx) = mpsc::channel();

        let mut child = Command::new(&binary_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn globe listener: {}", e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or("Failed to get stdout from globe listener")?;

        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().flatten() {
                let event = match line.trim() {
                    "READY" => Some(GlobeKeyEvent::Ready),
                    "DICTATE_START" => Some(GlobeKeyEvent::DictateStart),
                    "DICTATE_STOP" => Some(GlobeKeyEvent::DictateStop),
                    _ => None,
                };
                if let Some(evt) = event {
                    if tx.send(evt).is_err() {
                        break;
                    }
                }
            }
        });

        self.child = Some(child);
        self.event_rx = Some(rx);
        self.last_error = None;
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.event_rx = None;
    }

    fn find_binary() -> Result<PathBuf, String> {
        // Try multiple locations
        let exe_path = std::env::current_exe().ok();

        let candidates: Vec<PathBuf> = vec![
            // Development: relative to working directory
            PathBuf::from("target/globe-listener"),
            PathBuf::from("target/release/globe-listener"),
            PathBuf::from("target/debug/globe-listener"),
            // Bundled in app (next to the executable)
            exe_path
                .as_ref()
                .and_then(|p| p.parent())
                .map(|p| p.join("globe-listener"))
                .unwrap_or_default(),
            // Inside .app bundle
            exe_path
                .as_ref()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .map(|p| p.join("Resources/globe-listener"))
                .unwrap_or_default(),
        ];

        for path in &candidates {
            if path.exists() && !path.as_os_str().is_empty() {
                return Ok(path.clone());
            }
        }

        Err(format!(
            "Globe listener binary not found. Searched: {:?}",
            candidates
        ))
    }

    pub fn try_recv(&self) -> Option<GlobeKeyEvent> {
        self.event_rx.as_ref().and_then(|rx| match rx.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        })
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

impl Drop for GlobeKeyManager {
    fn drop(&mut self) {
        self.stop();
    }
}
