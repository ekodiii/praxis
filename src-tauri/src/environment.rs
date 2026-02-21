use std::path::{Path, PathBuf};
use std::process::Command;
use serde::{Deserialize, Serialize};

// ── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PythonInfo {
    pub found: bool,
    pub binary: String,      // e.g. "python3" or full venv path
    pub version: String,     // e.g. "3.11.4"
    pub meets_min: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VenvInfo {
    pub found: bool,
    pub path: String,        // absolute path to venv dir
    pub python_bin: String,  // absolute path to venv python binary
    pub pip_bin: String,     // absolute path to venv pip binary
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DepStatus {
    pub name: String,
    pub installed: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvStatus {
    pub python: PythonInfo,
    pub venv: VenvInfo,
    pub dependencies: Vec<DepStatus>,
}

// ── Version comparison ────────────────────────────────────────────────────────

fn parse_version(v: &str) -> (u32, u32, u32) {
    // Handle output like "Python 3.11.4" or "3.11.4"
    let stripped = v.trim_start_matches("Python").trim_start_matches("python").trim();
    let parts: Vec<u32> = stripped
        .split('.')
        .filter_map(|p| p.split_whitespace().next()?.parse().ok())
        .collect();
    (
        parts.get(0).copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    )
}

fn version_meets_min(version: &str, min: &str) -> bool {
    parse_version(version) >= parse_version(min)
}

// ── Run a binary and capture stdout ─────────────────────────────────────────

// Run in `cwd` so pyenv shims pick up the project's .python-version file.
fn run_version(bin: &str, cwd: &str) -> Option<String> {
    Command::new(bin)
        .arg("--version")
        .current_dir(cwd)
        .output()
        .ok()
        .and_then(|o| {
            // python --version prints to stderr on older versions, stdout on 3.x
            let out = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            if !out.is_empty() { Some(out) } else if !err.is_empty() { Some(err) } else { None }
        })
}

// ── Python detection ─────────────────────────────────────────────────────────

// project_folder is used as cwd so pyenv shims resolve the correct version.
pub fn detect_python(version_min: &str, project_folder: &str) -> PythonInfo {
    for bin in &["python3", "python"] {
        if let Some(ver) = run_version(bin, project_folder) {
            if ver.to_lowercase().contains("python") {
                let meets = version_meets_min(&ver, version_min);
                return PythonInfo {
                    found: true,
                    binary: bin.to_string(),
                    version: ver,
                    meets_min: meets,
                };
            }
        }
    }
    PythonInfo {
        found: false,
        binary: String::new(),
        version: String::new(),
        meets_min: false,
    }
}

// ── Venv detection ───────────────────────────────────────────────────────────

pub fn detect_venv(project_folder: &str) -> VenvInfo {
    let base = Path::new(project_folder);
    for name in &["venv", ".venv", "env"] {
        let venv_dir = base.join(name);
        if venv_dir.is_dir() {
            let (python_bin, pip_bin) = venv_binaries(&venv_dir);
            if python_bin.exists() {
                return VenvInfo {
                    found: true,
                    path: venv_dir.to_string_lossy().to_string(),
                    python_bin: python_bin.to_string_lossy().to_string(),
                    pip_bin: pip_bin.to_string_lossy().to_string(),
                };
            }
        }
    }
    VenvInfo {
        found: false,
        path: String::new(),
        python_bin: String::new(),
        pip_bin: String::new(),
    }
}

// Returns (python_path, pip_path) inside a venv dir — cross-platform.
fn venv_binaries(venv_dir: &Path) -> (PathBuf, PathBuf) {
    #[cfg(target_os = "windows")]
    {
        (
            venv_dir.join("Scripts").join("python.exe"),
            venv_dir.join("Scripts").join("pip.exe"),
        )
    }
    #[cfg(not(target_os = "windows"))]
    {
        (
            venv_dir.join("bin").join("python"),
            venv_dir.join("bin").join("pip"),
        )
    }
}

// ── Dependency checks ────────────────────────────────────────────────────────
//
// We always invoke pip as `python_bin -m pip` rather than calling a pip binary
// directly. This guarantees we check/install into the exact Python environment
// that will run the server, regardless of pyenv shims, homebrew paths, etc.

pub fn check_dependencies(python_bin: &str, project_folder: &str, deps: &[String]) -> Vec<DepStatus> {
    deps.iter().map(|dep| {
        // Strip extras like "passlib[bcrypt]" -> "passlib" for pip show
        let base = dep.split('[').next().unwrap_or(dep).trim();
        let installed = Command::new(python_bin)
            .args(["-m", "pip", "show", base])
            // Run in the project folder so pyenv shims resolve the correct
            // Python version from the project's .python-version file.
            .current_dir(project_folder)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        DepStatus { name: dep.clone(), installed }
    }).collect()
}

pub fn install_dependencies(python_bin: &str, project_folder: &str, deps: &[String]) -> Result<String, String> {
    let dep_args: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
    let output = Command::new(python_bin)
        .args(["-m", "pip", "install"])
        .args(&dep_args)
        .current_dir(project_folder)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
}

pub fn create_venv(project_folder: &str, python_bin: &str) -> Result<String, String> {
    let venv_path = Path::new(project_folder).join("venv");
    let output = Command::new(python_bin)
        .args(["-m", "venv"])
        .arg(&venv_path)
        .current_dir(project_folder)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(venv_path.to_string_lossy().to_string())
    } else {
        Err(if !stderr.is_empty() { stderr } else { stdout })
    }
}
