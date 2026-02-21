use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

mod course_loader;
mod db_assertions;
mod environment;
mod error;
mod server;
mod test_runner;
use course_loader::Course;

// ── App state ────────────────────────────────────────────────────────────────

struct AppState {
    data_dir: PathBuf,
    course: Mutex<Option<Course>>,
    server: Arc<Mutex<server::ServerState>>,
}

// ── Progress file helpers ────────────────────────────────────────────────────
// All take &PathBuf (the resolved app data dir) — no home dir lookups here.

fn progress_file(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("progress.json")
}

fn courses_dir(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("courses")
}

fn read_progress_json(data_dir: &PathBuf) -> serde_json::Value {
    let path = progress_file(data_dir);
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    }
}

fn write_progress_json(data_dir: &PathBuf, json: &serde_json::Value) -> Result<(), String> {
    fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    fs::write(
        progress_file(data_dir),
        serde_json::to_string_pretty(json).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

// ── Folder selection commands ────────────────────────────────────────────────

#[tauri::command]
async fn select_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let result = rx.recv().map_err(|e| e.to_string())?;
    Ok(result.map(|p| p.to_string()))
}

// Folder is stored per-course: progress.json["courses"][course_id]["project_folder"]
#[tauri::command]
fn get_saved_folder(course_id: String, state: tauri::State<AppState>) -> Result<Option<String>, String> {
    let json = read_progress_json(&state.data_dir);
    Ok(json
        .get("courses")
        .and_then(|c| c.get(&course_id))
        .and_then(|c| c.get("project_folder"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

#[tauri::command]
fn save_folder(course_id: String, folder: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);
    if json.get("courses").is_none() {
        json["courses"] = serde_json::json!({});
    }
    json["courses"][&course_id]["project_folder"] = serde_json::Value::String(folder);
    write_progress_json(&state.data_dir, &json)
}

// Open a .course file picker and load it
#[tauri::command]
async fn pick_and_load_course(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<Course>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Praxis Course", &["course"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let result = rx.recv().map_err(|e| e.to_string())?;
    let Some(file_path) = result else { return Ok(None) };
    let path_str = file_path.to_string();
    let p = std::path::Path::new(&path_str);
    let course = course_loader::load_course_from_file(p, &state.data_dir).map_err(|e| e.to_string())?;
    *state.course.lock().unwrap() = Some(course.clone());
    Ok(Some(course))
}

// ── Progress commands ─────────────────────────────────────────────────────────
// Progress is stored at:
//   ~/Library/Application Support/Praxis/progress.json
//   {
//     "courses": {
//       "<course_id>": {
//         "title": "...",
//         "version": "...",
//         "installed_at": "...",
//         "project_folder": "...",
//         "chapters": { "<chapter_id>": { "status": "...", ... } }
//       }
//     }
//   }

#[tauri::command]
fn get_progress(course_id: String, state: tauri::State<AppState>) -> Result<serde_json::Value, String> {
    let json = read_progress_json(&state.data_dir);
    let chapters = json
        .get("courses")
        .and_then(|c| c.get(&course_id))
        .and_then(|c| c.get("chapters"))
        .cloned()
        .unwrap_or(serde_json::json!({}));
    Ok(chapters)
}

#[tauri::command]
fn save_progress(
    course_id: String,
    chapters: serde_json::Value,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);

    // Ensure "courses" object exists
    if json.get("courses").is_none() {
        json["courses"] = serde_json::json!({});
    }

    // Set title, version, chapters_total from loaded course if available
    let (title, version, chapters_total) = {
        let lock = state.course.lock().unwrap();
        match lock.as_ref() {
            Some(c) => (c.title.clone(), c.version.clone(), c.chapters.len()),
            None => (String::new(), String::new(), 0),
        }
    };

    // Set installed_at only if not already present
    if json["courses"][&course_id].get("installed_at").is_none() {
        json["courses"][&course_id]["installed_at"] = serde_json::Value::String(
            chrono::Utc::now().to_rfc3339()
        );
    }

    if !title.is_empty() {
        json["courses"][&course_id]["title"] = serde_json::Value::String(title);
    }
    json["courses"][&course_id]["version"] = serde_json::Value::String(version);
    if chapters_total > 0 {
        json["courses"][&course_id]["chapters_total"] =
            serde_json::Value::Number(chapters_total.into());
    }
    json["courses"][&course_id]["chapters"] = chapters;

    write_progress_json(&state.data_dir, &json)
}

// ── Installed courses commands ────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct InstalledCourse {
    pub id: String,
    pub title: String,
    pub version: String,
    pub installed_at: String,
    pub chapters_total: usize,
    pub chapters_complete: usize,
    pub project_folder: Option<String>,
}

#[tauri::command]
fn get_installed_courses(state: tauri::State<AppState>) -> Result<Vec<InstalledCourse>, String> {
    let json = read_progress_json(&state.data_dir);
    let courses_obj = match json.get("courses").and_then(|v| v.as_object()) {
        Some(o) => o.clone(),
        None => return Ok(vec![]),
    };

    let mut result: Vec<InstalledCourse> = courses_obj.iter().map(|(id, data)| {
        let title = data.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string();
        let version = data.get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let installed_at = data.get("installed_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let project_folder = data.get("project_folder")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let (chapters_total, chapters_complete) = {
            let complete = match data.get("chapters").and_then(|v| v.as_object()) {
                Some(chs) => chs.values()
                    .filter(|v| v.get("status").and_then(|s| s.as_str()) == Some("complete"))
                    .count(),
                None => 0,
            };
            // Prefer the stored chapters_total (set from the actual course definition).
            // Fall back to counting progress keys only if it was never stored.
            let total = data.get("chapters_total")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or_else(|| {
                    data.get("chapters")
                        .and_then(|v| v.as_object())
                        .map(|o| o.len())
                        .unwrap_or(0)
                });
            (total, complete)
        };

        InstalledCourse { id: id.clone(), title, version, installed_at, chapters_total, chapters_complete, project_folder }
    }).collect();

    // Sort: most recently installed first (fallback: alphabetical)
    result.sort_by(|a, b| b.installed_at.cmp(&a.installed_at));

    Ok(result)
}

#[tauri::command]
fn remove_course(course_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);
    if let Some(courses) = json.get_mut("courses").and_then(|v| v.as_object_mut()) {
        courses.remove(&course_id);
    }
    write_progress_json(&state.data_dir, &json)
}

#[tauri::command]
fn reset_course_progress(course_id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);
    if let Some(course) = json.get_mut("courses").and_then(|v| v.as_object_mut()).and_then(|m| m.get_mut(&course_id)) {
        if let Some(obj) = course.as_object_mut() {
            obj.remove("chapters");
        }
    }
    write_progress_json(&state.data_dir, &json)
}

// ── Course loader commands ───────────────────────────────────────────────────

// Returns the extracted path for a previously-loaded course id, or error if not found.
#[tauri::command]
fn get_installed_course_path(course_id: String, state: tauri::State<AppState>) -> Result<String, String> {
    let path = courses_dir(&state.data_dir).join(&course_id);
    if path.exists() {
        Ok(path.to_string_lossy().to_string())
    } else {
        Err(format!("Course '{}' not found at {}", course_id, path.display()))
    }
}

#[tauri::command]
fn load_course(
    path: String,
    state: tauri::State<AppState>,
) -> Result<Course, String> {
    let p = std::path::Path::new(&path);
    let course = if path.ends_with(".course") {
        course_loader::load_course_from_file(p, &state.data_dir)
    } else {
        course_loader::load_course_from_folder(p)
    }.map_err(|e| e.to_string())?;
    *state.course.lock().unwrap() = Some(course.clone());
    Ok(course)
}

#[tauri::command]
fn get_chapter_content(
    chapter_id: String,
    state: tauri::State<AppState>,
) -> Result<String, String> {
    let lock = state.course.lock().unwrap();
    let course = lock.as_ref().ok_or("No course loaded")?;
    course_loader::read_chapter_content(course, &chapter_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_chapter_tests(
    chapter_id: String,
    state: tauri::State<AppState>,
) -> Result<serde_json::Value, String> {
    let lock = state.course.lock().unwrap();
    let course = lock.as_ref().ok_or("No course loaded")?;
    course_loader::read_chapter_tests(course, &chapter_id).map_err(|e| e.to_string())
}

// ── Environment detector commands ────────────────────────────────────────────

#[tauri::command]
fn check_environment(
    project_folder: String,
    version_min: String,
    deps: Vec<String>,
) -> Result<environment::EnvStatus, String> {
    // Pass project_folder as cwd so pyenv shims resolve the correct Python
    // version from the project's .python-version file.
    let python = environment::detect_python(&version_min, &project_folder);
    let venv   = environment::detect_venv(&project_folder);

    // Always run pip via the Python binary we're actually going to use:
    // venv python if a venv exists, otherwise the system python we detected.
    let python_bin_for_pip = if venv.found {
        venv.python_bin.clone()
    } else {
        python.binary.clone()
    };

    let dependencies = if python.found {
        environment::check_dependencies(&python_bin_for_pip, &project_folder, &deps)
    } else {
        deps.iter().map(|d| environment::DepStatus { name: d.clone(), installed: false }).collect()
    };

    Ok(environment::EnvStatus { python, venv, dependencies })
}

#[tauri::command]
fn install_deps(python_bin: String, project_folder: String, deps: Vec<String>) -> Result<String, String> {
    environment::install_dependencies(&python_bin, &project_folder, &deps)
}

#[tauri::command]
fn create_venv(project_folder: String, python_bin: String) -> Result<String, String> {
    environment::create_venv(&project_folder, &python_bin)
}

// ── Terminal launcher ────────────────────────────────────────────────────────

#[tauri::command]
fn open_terminal(folder: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .args(["-a", "Terminal", folder.as_str()])
        .spawn()
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "windows")]
    {
        let path = std::path::Path::new(&folder);
        std::process::Command::new("cmd")
            .args(["/C", "start", "cmd.exe"])
            .current_dir(path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ── Server subprocess commands ───────────────────────────────────────────────

#[tauri::command]
fn start_server(
    project_folder: String,
    command: String,
    port: u16,
    python_bin: Option<String>,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let server = Arc::clone(&state.server);
    server::start(
        server,
        &project_folder,
        &command,
        port,
        python_bin.as_deref(),
    ).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_server(state: tauri::State<AppState>) {
    server::stop(Arc::clone(&state.server));
}

#[tauri::command]
fn server_status(state: tauri::State<AppState>) -> serde_json::Value {
    let s = state.server.lock().unwrap();
    serde_json::json!({
        "status": s.status,
        "port": s.port,
    })
}

#[tauri::command]
fn server_output(state: tauri::State<AppState>) -> Vec<String> {
    state.server.lock().unwrap().output.iter().cloned().collect()
}

#[tauri::command]
fn wait_for_server(
    port: u16,
    health_endpoint: String,
    timeout_ms: u64,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let server = Arc::clone(&state.server);
    server::wait_for_healthy(server, port, &health_endpoint, timeout_ms)
        .map_err(|e| e.to_string())
}

// ── Port utility ──────────────────────────────────────────────────────────────

// Find a free port starting from start_port.
#[tauri::command]
fn find_free_port(start_port: u16) -> Result<u16, String> {
    test_runner::find_free_port(start_port).map_err(|e| e.to_string())
}

// ── Test runner command ───────────────────────────────────────────────────────

// Orchestrates a full test run for one chapter:
//   1. Stop existing sandboxed server (if any)
//   2. Delete clean_before_run files
//   3. Spawn fresh server on the given port
//   4. Wait for health
//   5. Run setup steps + all tests
//   6. Return full results (server stays running for Postman Lite)
#[tauri::command]
fn run_tests(
    project_folder: String,
    server_command: String,
    port: u16,
    python_bin: Option<String>,
    health_endpoint: String,
    clean_before_run: Vec<String>,
    tests: serde_json::Value,
    state: tauri::State<AppState>,
) -> Result<test_runner::RunResult, String> {
    let server = Arc::clone(&state.server);
    test_runner::run_chapter_tests(
        server,
        &project_folder,
        &server_command,
        port,
        python_bin.as_deref(),
        &health_endpoint,
        &clean_before_run,
        &tests,
    ).map_err(|e| e.to_string())
}

// ── HTTP client command (Postman Lite) ────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: String,
}

#[tauri::command]
fn send_http_request(
    method: String,
    url: String,
    headers: std::collections::HashMap<String, String>,
    body: Option<String>,
) -> Result<HttpResponse, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| e.to_string())?;

    let mut req = client.request(method, &url);

    for (k, v) in &headers {
        req = req.header(k.as_str(), v.as_str());
    }

    if let Some(b) = body {
        if !b.trim().is_empty() {
            req = req.body(b);
        }
    }

    let resp = req.send().map_err(|e| e.to_string())?;

    let status = resp.status().as_u16();
    let mut resp_headers = std::collections::HashMap::new();
    for (k, v) in resp.headers() {
        resp_headers.insert(
            k.as_str().to_string(),
            v.to_str().unwrap_or("").to_string(),
        );
    }
    let body = resp.text().map_err(|e| e.to_string())?;

    Ok(HttpResponse { status, headers: resp_headers, body })
}

// ── Saved requests commands (Postman Lite) ────────────────────────────────────
// Stored in progress.json under "saved_requests": [ { "id", "name", "method", "url", "headers", "body" }, ... ]

#[tauri::command]
fn get_saved_requests(state: tauri::State<AppState>) -> Result<serde_json::Value, String> {
    let json = read_progress_json(&state.data_dir);
    Ok(json.get("saved_requests").cloned().unwrap_or(serde_json::json!([])))
}

#[tauri::command]
fn save_request(request: serde_json::Value, state: tauri::State<AppState>) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);
    let mut arr = json["saved_requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let id = request.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if let Some(pos) = arr.iter().position(|r| r.get("id").and_then(|v| v.as_str()) == Some(&id)) {
        arr[pos] = request;
    } else {
        arr.push(request);
    }
    json["saved_requests"] = serde_json::Value::Array(arr);
    write_progress_json(&state.data_dir, &json)
}

#[tauri::command]
fn delete_saved_request(id: String, state: tauri::State<AppState>) -> Result<(), String> {
    let mut json = read_progress_json(&state.data_dir);
    let arr = json["saved_requests"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let arr: Vec<_> = arr.into_iter()
        .filter(|r| r.get("id").and_then(|v| v.as_str()) != Some(&id))
        .collect();
    json["saved_requests"] = serde_json::Value::Array(arr);
    write_progress_json(&state.data_dir, &json)
}

// ── App entry point ──────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()
                .expect("Could not resolve app data directory");
            fs::create_dir_all(&data_dir)
                .expect("Could not create app data directory");
            app.manage(AppState {
                data_dir,
                course: Mutex::new(None),
                server: Arc::new(Mutex::new(server::ServerState::new())),
            });
            Ok(())
        })
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            select_folder,
            get_saved_folder,
            save_folder,
            pick_and_load_course,
            get_progress,
            save_progress,
            load_course,
            get_chapter_content,
            get_chapter_tests,
            check_environment,
            install_deps,
            create_venv,
            open_terminal,
            start_server,
            stop_server,
            server_status,
            server_output,
            wait_for_server,
            find_free_port,
            run_tests,
            send_http_request,
            get_saved_requests,
            save_request,
            delete_saved_request,
            get_installed_courses,
            get_installed_course_path,
            remove_course,
            reset_course_progress,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
