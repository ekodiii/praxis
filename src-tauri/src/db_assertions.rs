use std::collections::HashMap;
use std::path::Path;

use regex::Regex;
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use serde_json::Value;

// ── Schema ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct DbAssertBlock {
    pub database: String,
    pub queries: Vec<DbQuery>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct DbQuery {
    pub id: String,
    #[serde(default)]
    pub sql: String,
    pub assert: Value,
}

// ── Entry point ───────────────────────────────────────────────────────────────

/// Evaluate all DB assertions for a single test.
/// Returns a Vec of human-readable failure strings (empty = all passed).
/// Each failure is prefixed with "DB: " so the frontend can identify it.
pub fn evaluate_db_assertions(
    project_folder: &str,
    block: &DbAssertBlock,
    vars: &HashMap<String, Value>,
) -> Vec<String> {
    // Resolve the project root so we can enforce a containment boundary.
    let project_root = match Path::new(project_folder).canonicalize() {
        Ok(p) => p,
        Err(e) => return vec![format!("DB: invalid project folder: {}", e)],
    };

    // Prevent path traversal: the database file must live inside the project folder.
    let db_path = project_root.join(&block.database);
    let db_canonical = match db_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // File doesn't exist yet — normalize without hitting the filesystem,
            // then check the boundary.
            normalize_path(&db_path)
        }
    };
    if !db_canonical.starts_with(&project_root) {
        return vec![format!(
            "DB: database path '{}' escapes the project folder — aborting.",
            block.database
        )];
    }

    let conn = match Connection::open_with_flags(&db_canonical, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => {
            return vec![format!("DB: could not open '{}': {}", block.database, e)];
        }
    };

    let mut failures = Vec::new();

    for query in &block.queries {
        let query_failures = evaluate_query(&conn, &query.id, &query.sql, &query.assert, vars);
        for f in query_failures {
            failures.push(format!("DB: {}", f));
        }
    }

    failures
}

// Normalize a path without touching the filesystem (strips `..` and `.`).
fn normalize_path(path: &Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => { out.pop(); }
            Component::CurDir    => {}
            other                => out.push(other),
        }
    }
    out
}

// ── Query evaluation ──────────────────────────────────────────────────────────

// Replace {{variable}} placeholders in SQL with `?` parameters and return the
// ordered list of corresponding values. This prevents SQL injection: variable
// values are never interpolated as raw SQL text.
fn parameterize_sql(sql: &str, vars: &HashMap<String, Value>) -> (String, Vec<rusqlite::types::Value>) {
    let re = Regex::new(r"\{\{(\w+)\}\}").expect("static regex");
    let mut params: Vec<rusqlite::types::Value> = Vec::new();
    let parameterized = re.replace_all(sql, |caps: &regex::Captures| {
        let name = &caps[1];
        let val = vars.get(name).cloned().unwrap_or(Value::Null);
        params.push(json_to_sqlite(val));
        "?"
    });
    (parameterized.into_owned(), params)
}

fn json_to_sqlite(v: Value) -> rusqlite::types::Value {
    match v {
        Value::Null         => rusqlite::types::Value::Null,
        Value::Bool(b)      => rusqlite::types::Value::Integer(if b { 1 } else { 0 }),
        Value::Number(n)    => {
            if let Some(i) = n.as_i64() {
                rusqlite::types::Value::Integer(i)
            } else {
                rusqlite::types::Value::Real(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s)    => rusqlite::types::Value::Text(s),
        other               => rusqlite::types::Value::Text(other.to_string()),
    }
}

fn evaluate_query(
    conn: &Connection,
    id: &str,
    sql_template: &str,
    assert: &Value,
    vars: &HashMap<String, Value>,
) -> Vec<String> {
    // Build a parameterized SQL string and collect bound values — never
    // interpolate variable content as raw SQL text.
    let (sql, params) = parameterize_sql(sql_template, vars);

    // Execute the query and collect all rows as Vec<HashMap<col_name, Value>>
    let mut stmt = match conn.prepare(&sql) {
        Ok(s) => s,
        Err(e) => return vec![format!("[{}] SQL error: {}", id, e)],
    };

    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|v| v as &dyn rusqlite::types::ToSql).collect();

    let rows_result: Result<Vec<HashMap<String, Value>>, _> = stmt
        .query_map(params_refs.as_slice(), |row| {
            let mut map = HashMap::new();
            for (i, col) in col_names.iter().enumerate() {
                let val: Value = match row.get_ref(i) {
                    Ok(rusqlite::types::ValueRef::Null) => Value::Null,
                    Ok(rusqlite::types::ValueRef::Integer(n)) => Value::from(n),
                    Ok(rusqlite::types::ValueRef::Real(f)) => {
                        Value::from(serde_json::Number::from_f64(f).unwrap_or(serde_json::Number::from(0)))
                    }
                    Ok(rusqlite::types::ValueRef::Text(s)) => {
                        Value::String(String::from_utf8_lossy(s).to_string())
                    }
                    Ok(rusqlite::types::ValueRef::Blob(b)) => {
                        Value::String(format!("<blob {} bytes>", b.len()))
                    }
                    Err(_) => Value::Null,
                };
                map.insert(col.clone(), val);
            }
            Ok(map)
        })
        .and_then(|mapped| mapped.collect());

    let rows = match rows_result {
        Ok(r) => r,
        Err(e) => return vec![format!("[{}] query failed: {}", id, e)],
    };

    let mut failures = Vec::new();

    let assert_map = match assert.as_object() {
        Some(m) => m,
        None => return vec![format!("[{}] assert must be an object", id)],
    };

    for (op, expected) in assert_map {
        match op.as_str() {
            "$row_count" => {
                let count = rows.len();
                let count_val = Value::from(count as i64);
                // expected can be a number (exact match) or an operator object
                let assertion = if expected.is_object() {
                    expected.clone()
                } else {
                    serde_json::json!({ "$eq": expected })
                };
                let f = crate::test_runner::evaluate_body_assertion(
                    &assertion,
                    &count_val,
                    &format!("[{}] $row_count", id),
                );
                failures.extend(f);
            }

            "$column_names" => {
                // expected: { "$contains_all": ["col1", "col2"] }
                if let Some(inner) = expected.as_object() {
                    if let Some(required) = inner.get("$contains_all") {
                        if let Some(req_arr) = required.as_array() {
                            let actual_cols: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
                            for req_col in req_arr {
                                let req_col_str = req_col.as_str().unwrap_or("");
                                if !actual_cols.contains(&req_col_str) {
                                    failures.push(format!(
                                        "[{}] $column_names: missing column '{}'",
                                        id, req_col_str
                                    ));
                                }
                            }
                        } else {
                            failures.push(format!(
                                "[{}] $column_names.$contains_all must be an array",
                                id
                            ));
                        }
                    } else {
                        failures.push(format!("[{}] $column_names: unknown operator", id));
                    }
                } else {
                    failures.push(format!("[{}] $column_names value must be an object", id));
                }
            }

            "$row" => {
                // expected: { "index": N, "fields": { "col": assertion, ... } }
                let index = expected
                    .get("index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let fields = expected.get("fields").cloned().unwrap_or(Value::Null);

                let row = match rows.get(index) {
                    Some(r) => r,
                    None => {
                        failures.push(format!(
                            "[{}] $row: no row at index {} (got {} rows)",
                            id,
                            index,
                            rows.len()
                        ));
                        continue;
                    }
                };

                // Convert row HashMap to a serde_json Object for assertion
                let row_val: Value = Value::Object(
                    row.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect::<serde_json::Map<_, _>>(),
                );

                if let Some(fields_map) = fields.as_object() {
                    for (field, field_assert) in fields_map {
                        let actual_field = row_val.get(field).unwrap_or(&Value::Null);
                        let assertion = if field_assert.is_object()
                            && field_assert
                                .as_object()
                                .map(|m| m.keys().any(|k| k.starts_with('$')))
                                .unwrap_or(false)
                        {
                            field_assert.clone()
                        } else {
                            serde_json::json!({ "$eq": field_assert })
                        };
                        let f = crate::test_runner::evaluate_body_assertion(
                            &assertion,
                            actual_field,
                            &format!("[{}] row[{}].{}", id, index, field),
                        );
                        failures.extend(f);
                    }
                } else if !fields.is_null() {
                    failures.push(format!("[{}] $row.fields must be an object", id));
                }
            }

            unknown => {
                failures.push(format!("[{}] unknown db assert operator '{}'", id, unknown));
            }
        }
    }

    failures
}

