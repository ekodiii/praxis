use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use serde::Serialize;

use crate::error::PraxisError;

#[cfg(target_os = "macos")]
use std::os::unix::process::CommandExt;

// ── Status type ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Crashed,
}

// ── Shared server state (stored in AppState) ──────────────────────────────────

pub struct ServerState {
    pub status: ServerStatus,
    pub child: Option<Child>,
    pub output: VecDeque<String>,   // combined stdout + stderr lines, capped at 500
    pub port: u16,
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            status: ServerStatus::Stopped,
            child: None,
            output: VecDeque::new(),
            port: 0,
        }
    }

    fn append_line(&mut self, line: String) {
        self.output.push_back(line);
        if self.output.len() > 500 {
            self.output.pop_front();
        }
    }
}

// ── Port check ───────────────────────────────────────────────────────────────

pub fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

// ── Kill process tree ─────────────────────────────────────────────────────────

fn kill_process_tree(child: &mut Child) {
    let pid = child.id();

    #[cfg(target_os = "macos")]
    unsafe {
        // Kill the entire process group so child processes (uvicorn workers) die too.
        // This works because start() calls setsid() via pre_exec, giving the server
        // its own process group.
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }

    #[cfg(target_os = "windows")]
    {
        // taskkill /F /T kills the process and all its children
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output();
    }

    // Best-effort kill on the parent regardless
    let _ = child.kill();
    let _ = child.wait();
}

// ── Start server ─────────────────────────────────────────────────────────────

pub fn start(
    server_state: Arc<Mutex<ServerState>>,
    project_folder: &str,
    command_template: &str,
    port: u16,
    python_bin: Option<&str>,
) -> Result<(), PraxisError> {
    // Check port is free before spawning
    if !port_is_free(port) {
        return Err(PraxisError::PortInUse(port));
    }

    // Substitute {port} in command template
    let command_str = command_template.replace("{port}", &port.to_string());

    // Split into binary + args. On macOS/Windows with a venv, prepend the venv
    // python directory to PATH so uvicorn resolves correctly.
    let mut parts = command_str.split_whitespace();
    let bin = parts.next().ok_or_else(|| PraxisError::Other("Empty server command".to_string()))?;
    let args: Vec<&str> = parts.collect();

    let mut cmd = Command::new(bin);
    cmd.args(&args)
        .current_dir(project_folder)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // If a venv python path is provided, prepend its bin/Scripts dir to PATH
    // so uvicorn and dependencies resolve from the venv.
    if let Some(py) = python_bin {
        if let Some(bin_dir) = Path::new(py).parent() {
            let current_path = std::env::var("PATH").unwrap_or_default();
            let separator = if cfg!(target_os = "windows") { ";" } else { ":" };
            let new_path = format!("{}{}{}", bin_dir.display(), separator, current_path);
            cmd.env("PATH", new_path);
        }
    }

    // On macOS, put the child in its own process group (setsid) so that
    // killpg() in kill_process_tree() only kills the server and its children,
    // not the Praxis app itself.
    #[cfg(target_os = "macos")]
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }

    // Spawn
    let mut child = cmd.spawn()?;

    // Grab stdout/stderr handles before storing child
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    {
        let mut s = server_state.lock().unwrap();
        s.status = ServerStatus::Starting;
        s.child  = Some(child);
        s.output.clear();
        s.port   = port;
    }

    // Stream stdout in background thread
    if let Some(out) = stdout {
        let state_clone = Arc::clone(&server_state);
        thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines().flatten() {
                let mut s = state_clone.lock().unwrap();
                s.append_line(format!("[out] {}", line));
            }
        });
    }

    // Stream stderr in background thread
    if let Some(err) = stderr {
        let state_clone = Arc::clone(&server_state);
        thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().flatten() {
                let mut s = state_clone.lock().unwrap();
                s.append_line(format!("[err] {}", line));
            }
            // When stderr closes, the process has exited — mark crashed if not stopped
            let mut s = state_clone.lock().unwrap();
            if s.status == ServerStatus::Running || s.status == ServerStatus::Starting {
                s.status = ServerStatus::Crashed;
            }
        });
    }

    Ok(())
}

// ── Stop server ───────────────────────────────────────────────────────────────

pub fn stop(server_state: Arc<Mutex<ServerState>>) {
    let mut s = server_state.lock().unwrap();
    if let Some(ref mut child) = s.child {
        kill_process_tree(child);
    }
    s.child  = None;
    s.status = ServerStatus::Stopped;
    s.port   = 0;
}

// ── Health check ──────────────────────────────────────────────────────────────

pub fn check_health(port: u16, endpoint: &str) -> bool {
    let url = format!("http://127.0.0.1:{}{}", port, endpoint);
    matches!(ureq_get(&url), Ok(200))
}

// Minimal HTTP GET — avoids pulling in reqwest for a simple health check.
// Returns status code or error. Uses std::net directly.
fn ureq_get(url: &str) -> Result<u16, String> {
    // Parse host + port + path from url
    let without_scheme = url.strip_prefix("http://").ok_or("bad url")?;
    let (host_port, path) = without_scheme
        .split_once('/')
        .map(|(h, p)| (h, format!("/{}", p)))
        .unwrap_or((without_scheme, "/".to_string()));

    let mut stream = std::net::TcpStream::connect_timeout(
        &host_port.parse().map_err(|e: std::net::AddrParseError| e.to_string())?,
        std::time::Duration::from_millis(800),
    ).map_err(|e| e.to_string())?;

    stream.set_read_timeout(Some(std::time::Duration::from_millis(800)))
        .map_err(|e| e.to_string())?;

    use std::io::Write;
    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nConnection: close\r\n\r\n",
        path, host_port
    );
    stream.write_all(request.as_bytes()).map_err(|e| e.to_string())?;

    let mut response = String::new();
    use std::io::Read;
    let _ = stream.read_to_string(&mut response);

    // Parse "HTTP/1.x NNN ..." status line
    let status: u16 = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or("Could not parse status")?;

    Ok(status)
}

// ── Wait for server (polls health endpoint) ───────────────────────────────────

pub fn wait_for_healthy(
    server_state: Arc<Mutex<ServerState>>,
    port: u16,
    endpoint: &str,
    timeout_ms: u64,
) -> Result<(), PraxisError> {
    let start    = std::time::Instant::now();
    let deadline = std::time::Duration::from_millis(timeout_ms);

    loop {
        // Check if process crashed while we were waiting
        {
            let s = server_state.lock().unwrap();
            if s.status == ServerStatus::Crashed {
                return Err(PraxisError::ServerCrashed);
            }
        }

        if check_health(port, endpoint) {
            let mut s = server_state.lock().unwrap();
            s.status = ServerStatus::Running;
            return Ok(());
        }

        if start.elapsed() >= deadline {
            return Err(PraxisError::Other(format!(
                "Server did not become healthy within {}ms",
                timeout_ms
            )));
        }

        thread::sleep(std::time::Duration::from_millis(500));
    }
}
