use std::collections::HashMap;
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db_assertions::{evaluate_db_assertions, DbAssertBlock};
use crate::error::PraxisError;
use crate::server;

// ── Public result types ───────────────────────────────────────────────────────

#[derive(Debug, Serialize, Clone)]
pub struct TestResult {
    pub id: String,
    pub name: String,
    pub passed: bool,
    pub error: bool,       // true if the test could not run (e.g. server crashed)
    pub message: String,   // error/crash message when error=true
    pub failures: Vec<String>, // human-readable assertion failure descriptions
}

#[derive(Debug, Serialize)]
pub struct RunResult {
    pub results: Vec<TestResult>,
    pub passed: usize,
    pub failed: usize,
    pub total: usize,
    pub server_crashed: bool,
    pub setup_error: Option<String>,
}

// ── Test file schema (deserialized from the course test JSON) ─────────────────

#[derive(Debug, Deserialize, Clone)]
struct TestFile {
    #[serde(default)]
    setup: Vec<SetupStep>,
    tests: Vec<Test>,
}

#[derive(Debug, Deserialize, Clone)]
struct SetupStep {
    id: String,
    #[serde(default)]
    _description: String,
    request: RequestDef,
    expect_status: Option<u16>,
    #[serde(default)]
    capture: HashMap<String, String>,
}

#[derive(Debug, Deserialize, Clone)]
struct Test {
    id: String,
    name: String,
    #[serde(default)]
    request: Option<RequestDef>,
    #[serde(default)]
    assert: AssertDef,
    #[serde(default)]
    capture: HashMap<String, String>,
    #[serde(default)]
    db_assert: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone, Default)]
struct AssertDef {
    status: Option<u16>,
    #[serde(default)]
    body: Value, // null or object of assertion clauses
}

#[derive(Debug, Deserialize, Clone)]
struct RequestDef {
    method: String,
    path: String,
    #[serde(default)]
    headers: HashMap<String, String>,
    #[serde(default)]
    body: Option<Value>,
}

// ── Variable store ────────────────────────────────────────────────────────────

type Vars = HashMap<String, Value>;

// Interpolate {{variable}} placeholders in a string using the variable store.
fn interpolate(s: &str, vars: &Vars) -> String {
    let mut result = s.to_string();
    for (k, v) in vars {
        let placeholder = format!("{{{{{}}}}}", k);
        let replacement = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        result = result.replace(&placeholder, &replacement);
    }
    result
}

// Recursively interpolate all string values in a JSON value.
fn interpolate_value(val: &Value, vars: &Vars) -> Value {
    match val {
        Value::String(s) => Value::String(interpolate(s, vars)),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), interpolate_value(v, vars));
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| interpolate_value(v, vars)).collect()),
        other => other.clone(),
    }
}

// Extract a value from a JSON response using a dot-path like "body.token" or "body.data.0.id".
fn extract_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for segment in path.split('.') {
        cur = if let Ok(idx) = segment.parse::<usize>() {
            cur.get(idx)?
        } else {
            cur.get(segment)?
        };
    }
    Some(cur)
}

// Capture variables from a response body given a capture map.
fn apply_captures(captures: &HashMap<String, String>, body: &Value, vars: &mut Vars) {
    // The capture paths start with "body." — we wrap the body under "body" key.
    let wrapped = serde_json::json!({ "body": body });
    for (var_name, path) in captures {
        if let Some(val) = extract_path(&wrapped, path) {
            vars.insert(var_name.clone(), val.clone());
        }
    }
}

// ── HTTP client ───────────────────────────────────────────────────────────────

struct Response {
    status: u16,
    body: Value,
}

fn send_request(
    client: &reqwest::blocking::Client,
    base_url: &str,
    req: &RequestDef,
    vars: &Vars,
) -> Result<Response, String> {
    let path = interpolate(&req.path, vars);
    let url  = format!("{}{}", base_url, path);

    let mut builder = match req.method.to_uppercase().as_str() {
        "GET"    => client.get(&url),
        "POST"   => client.post(&url),
        "PUT"    => client.put(&url),
        "PATCH"  => client.patch(&url),
        "DELETE" => client.delete(&url),
        other    => return Err(format!("Unsupported HTTP method: {}", other)),
    };

    // Headers
    for (k, v) in &req.headers {
        let key = interpolate(k, vars);
        let val = interpolate(v, vars);
        builder = builder.header(key, val);
    }

    // Body — interpolate all string values before sending
    if let Some(body) = &req.body {
        let interp = interpolate_value(body, vars);
        builder = builder
            .header("Content-Type", "application/json")
            .json(&interp);
    }

    let resp = builder.send().map_err(|e| format!("Request failed: {}", e))?;
    let status = resp.status().as_u16();
    let body: Value = resp.json().unwrap_or(Value::Null);

    Ok(Response { status, body })
}

// ── Assertion engine ──────────────────────────────────────────────────────────

// Evaluate all assertions defined in assert_def against the actual response.
// Returns a list of human-readable failure strings (empty = all passed).
fn evaluate_assertions(assert_def: &AssertDef, resp: &Response) -> Vec<String> {
    let mut failures = Vec::new();

    // Status assertion
    if let Some(expected_status) = assert_def.status {
        if resp.status != expected_status {
            failures.push(format!(
                "status: expected {} got {}",
                expected_status, resp.status
            ));
        }
    }

    // Body assertions
    if !assert_def.body.is_null() {
        let body_failures = evaluate_body_assertion(&assert_def.body, &resp.body, "body");
        failures.extend(body_failures);
    }

    failures
}

// Recursively evaluate body assertions. `path` is used for human-readable messages.
pub fn evaluate_body_assertion(assertion: &Value, actual: &Value, path: &str) -> Vec<String> {
    let mut failures = Vec::new();

    match assertion {
        // Operator object: { "$eq": value } etc.
        Value::Object(map) => {
            // Check if any key is an operator (starts with $)
            let has_operator = map.keys().any(|k| k.starts_with('$'));

            if has_operator {
                for (op, expected) in map {
                    match op.as_str() {
                        "$eq" => {
                            if actual != expected {
                                failures.push(format!(
                                    "{}: expected {} got {}",
                                    path, expected, actual
                                ));
                            }
                        }
                        "$not_eq" => {
                            if actual == expected {
                                failures.push(format!(
                                    "{}: expected value to not equal {}",
                                    path, expected
                                ));
                            }
                        }
                        "$type" => {
                            let expected_type = expected.as_str().unwrap_or("");
                            let actual_type = json_type_name(actual);
                            if actual_type != expected_type {
                                failures.push(format!(
                                    "{}: expected type '{}' got '{}'",
                                    path, expected_type, actual_type
                                ));
                            }
                        }
                        "$exists" => {
                            if expected.as_bool().unwrap_or(true) && actual.is_null() {
                                failures.push(format!("{}: field must exist", path));
                            }
                        }
                        "$gt" => {
                            if let (Some(a), Some(e)) = (actual.as_f64(), expected.as_f64()) {
                                if a <= e {
                                    failures.push(format!("{}: expected > {} got {}", path, e, a));
                                }
                            } else {
                                failures.push(format!("{}: $gt requires numeric values", path));
                            }
                        }
                        "$gte" => {
                            if let (Some(a), Some(e)) = (actual.as_f64(), expected.as_f64()) {
                                if a < e {
                                    failures.push(format!("{}: expected >= {} got {}", path, e, a));
                                }
                            } else {
                                failures.push(format!("{}: $gte requires numeric values", path));
                            }
                        }
                        "$lt" => {
                            if let (Some(a), Some(e)) = (actual.as_f64(), expected.as_f64()) {
                                if a >= e {
                                    failures.push(format!("{}: expected < {} got {}", path, e, a));
                                }
                            } else {
                                failures.push(format!("{}: $lt requires numeric values", path));
                            }
                        }
                        "$lte" => {
                            if let (Some(a), Some(e)) = (actual.as_f64(), expected.as_f64()) {
                                if a > e {
                                    failures.push(format!("{}: expected <= {} got {}", path, e, a));
                                }
                            } else {
                                failures.push(format!("{}: $lte requires numeric values", path));
                            }
                        }
                        "$contains_string" => {
                            let needle = expected.as_str().unwrap_or("");
                            let haystack = actual.as_str().unwrap_or("");
                            if !haystack.contains(needle) {
                                failures.push(format!(
                                    "{}: expected string to contain '{}', got '{}'",
                                    path, needle, haystack
                                ));
                            }
                        }
                        "$matches" => {
                            let pattern = expected.as_str().unwrap_or("");
                            let s = actual.as_str().unwrap_or("");
                            match Regex::new(pattern) {
                                Ok(re) => {
                                    if !re.is_match(s) {
                                        failures.push(format!(
                                            "{}: '{}' did not match regex /{}/",
                                            path, s, pattern
                                        ));
                                    }
                                }
                                Err(e) => {
                                    failures.push(format!("{}: invalid regex '{}': {}", path, pattern, e));
                                }
                            }
                        }
                        "$is_array" => {
                            if expected.as_bool().unwrap_or(true) && !actual.is_array() {
                                failures.push(format!(
                                    "{}: expected array, got {}",
                                    path, json_type_name(actual)
                                ));
                            }
                        }
                        "$length" => {
                            if let Some(arr) = actual.as_array() {
                                let expected_len = expected.as_u64().unwrap_or(0) as usize;
                                if arr.len() != expected_len {
                                    failures.push(format!(
                                        "{}: expected length {} got {}",
                                        path, expected_len, arr.len()
                                    ));
                                }
                            } else {
                                failures.push(format!("{}: $length requires an array", path));
                            }
                        }
                        "$min_length" => {
                            if let Some(arr) = actual.as_array() {
                                let min = expected.as_u64().unwrap_or(0) as usize;
                                if arr.len() < min {
                                    failures.push(format!(
                                        "{}: expected min length {} got {}",
                                        path, min, arr.len()
                                    ));
                                }
                            } else {
                                failures.push(format!("{}: $min_length requires an array", path));
                            }
                        }
                        "$contains" => {
                            // Array contains an object matching all given key/value pairs
                            if let Some(arr) = actual.as_array() {
                                let matches = arr.iter().any(|item| {
                                    object_matches(item, expected)
                                });
                                if !matches {
                                    failures.push(format!(
                                        "{}: array does not contain an element matching {}",
                                        path, expected
                                    ));
                                }
                            } else {
                                failures.push(format!("{}: $contains requires an array", path));
                            }
                        }
                        unknown => {
                            failures.push(format!("{}: unknown operator '{}'", path, unknown));
                        }
                    }
                }
            } else {
                // Plain object — recurse into each field.
                // Shorthand: { "field": value } where value is a scalar is sugar for
                // { "field": { "$eq": value } }. If value is already an operator object,
                // pass it through directly.
                for (field, field_assertion) in map {
                    let field_path = format!("{}.{}", path, field);
                    let field_actual = actual.get(field).unwrap_or(&Value::Null);

                    let is_operator_obj = field_assertion
                        .as_object()
                        .map(|m| m.keys().any(|k| k.starts_with('$')))
                        .unwrap_or(false);

                    let normalized = if !is_operator_obj && !field_assertion.is_object() {
                        // Scalar shorthand — wrap in $eq
                        serde_json::json!({ "$eq": field_assertion })
                    } else {
                        field_assertion.clone()
                    };

                    failures.extend(evaluate_body_assertion(&normalized, field_actual, &field_path));
                }
            }
        }
        // Bare scalar at top level — treat as $eq (shouldn't normally happen per spec but be safe)
        other => {
            if actual != other {
                failures.push(format!("{}: expected {} got {}", path, other, actual));
            }
        }
    }

    failures
}

// Check that an array item matches all key/value pairs in `pattern`.
// Values in pattern that are plain scalars use equality; objects use operator logic.
fn object_matches(item: &Value, pattern: &Value) -> bool {
    let pattern_map = match pattern.as_object() {
        Some(m) => m,
        None => return item == pattern,
    };

    for (k, v) in pattern_map {
        let actual_field = item.get(k).unwrap_or(&Value::Null);
        let check_val = if v.is_object() && v.as_object().map(|m| m.keys().any(|k| k.starts_with('$'))).unwrap_or(false) {
            v.clone()
        } else {
            serde_json::json!({ "$eq": v })
        };
        let fail = evaluate_body_assertion(&check_val, actual_field, k);
        if !fail.is_empty() {
            return false;
        }
    }
    true
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null    => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_)  => "array",
        Value::Object(_) => "object",
    }
}

// ── Port utility ──────────────────────────────────────────────────────────────

pub fn find_free_port(start: u16) -> Result<u16, PraxisError> {
    let mut port = start;
    loop {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
        port = port.checked_add(1)
            .ok_or_else(|| PraxisError::Other("No free port found in range".to_string()))?;
        if port > start + 100 {
            return Err(PraxisError::Other("No free port found in range".to_string()));
        }
    }
}

// ── Main orchestration ────────────────────────────────────────────────────────

pub fn run_chapter_tests(
    server_state: Arc<Mutex<server::ServerState>>,
    project_folder: &str,
    server_command: &str,
    port: u16,
    python_bin: Option<&str>,
    health_endpoint: &str,
    clean_before_run: &[String],
    test_data: &Value,
) -> Result<RunResult, PraxisError> {
    // 1. Stop any existing sandboxed server
    server::stop(Arc::clone(&server_state));

    // 2. Delete clean_before_run files
    for rel_path in clean_before_run {
        let full = Path::new(project_folder).join(rel_path);
        if full.exists() {
            fs::remove_file(&full)?;
        }
    }

    // 3. Spawn server
    server::start(
        Arc::clone(&server_state),
        project_folder,
        server_command,
        port,
        python_bin,
    )?;

    // 4. Wait for health (15 second timeout)
    server::wait_for_healthy(Arc::clone(&server_state), port, health_endpoint, 15_000)?;

    // 5. Parse test file
    let tf: TestFile = serde_json::from_value(test_data.clone())?;

    // 6. Build HTTP client (10s per-request timeout)
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| PraxisError::Other(e.to_string()))?;

    let base_url = format!("http://127.0.0.1:{}", port);
    let mut vars: Vars = HashMap::new();
    let mut results: Vec<TestResult> = Vec::new();

    // 7. Run setup steps
    for step in &tf.setup {
        let resp = match send_request(&client, &base_url, &step.request, &vars) {
            Ok(r) => r,
            Err(e) => {
                // Server may have crashed
                return Ok(RunResult {
                    results,
                    passed: 0,
                    failed: 0,
                    total: tf.tests.len(),
                    server_crashed: is_crashed(&server_state),
                    setup_error: Some(format!("Setup step '{}' failed: {}", step.id, e)),
                });
            }
        };

        if let Some(expect) = step.expect_status {
            if resp.status != expect {
                return Ok(RunResult {
                    results,
                    passed: 0,
                    failed: 0,
                    total: tf.tests.len(),
                    server_crashed: is_crashed(&server_state),
                    setup_error: Some(format!(
                        "Setup step '{}': expected status {} got {}",
                        step.id, expect, resp.status
                    )),
                });
            }
        }

        apply_captures(&step.capture, &resp.body, &mut vars);
    }

    // 8. Run tests
    let mut crashed = false;

    for test in &tf.tests {
        // Check if server crashed between tests
        if is_crashed(&server_state) {
            crashed = true;
            results.push(TestResult {
                id: test.id.clone(),
                name: test.name.clone(),
                passed: false,
                error: true,
                message: "Server crashed during test run. Check your code for errors.".into(),
                failures: vec![],
            });
            continue;
        }

        let mut failures: Vec<String> = Vec::new();

        // HTTP request + assertions (optional — DB-only tests have no request)
        if let Some(req) = &test.request {
            let resp = match send_request(&client, &base_url, req, &vars) {
                Ok(r) => r,
                Err(e) => {
                    let is_crash = is_crashed(&server_state);
                    crashed = is_crash;
                    results.push(TestResult {
                        id: test.id.clone(),
                        name: test.name.clone(),
                        passed: false,
                        error: true,
                        message: if is_crash {
                            "Server crashed during test run. Check your code for errors.".into()
                        } else {
                            e
                        },
                        failures: vec![],
                    });
                    continue;
                }
            };

            // Captures first (even on failure) so subsequent tests can use them
            apply_captures(&test.capture, &resp.body, &mut vars);

            // HTTP assertions
            failures.extend(evaluate_assertions(&test.assert, &resp));
        }

        // DB assertions (run regardless of HTTP result — let the test author decide)
        if let Some(db_assert_val) = &test.db_assert {
            match serde_json::from_value::<DbAssertBlock>(db_assert_val.clone()) {
                Ok(block) => {
                    failures.extend(evaluate_db_assertions(project_folder, &block, &vars));
                }
                Err(e) => {
                    failures.push(format!("DB: invalid db_assert schema: {}", e));
                }
            }
        }

        let passed = failures.is_empty();

        results.push(TestResult {
            id: test.id.clone(),
            name: test.name.clone(),
            passed,
            error: false,
            message: String::new(),
            failures,
        });
    }

    let server_crashed = crashed || is_crashed(&server_state);
    let passed = results.iter().filter(|r| r.passed).count();
    let failed = results.iter().filter(|r| !r.passed).count();
    let total  = results.len();

    Ok(RunResult {
        results,
        passed,
        failed,
        total,
        server_crashed,
        setup_error: None,
    })
}

fn is_crashed(server_state: &Arc<Mutex<server::ServerState>>) -> bool {
    server_state.lock().unwrap().status == server::ServerStatus::Crashed
}
