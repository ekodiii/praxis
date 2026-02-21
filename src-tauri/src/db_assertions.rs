use std::collections::HashMap;
use std::path::Path;

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
    let db_path = Path::new(project_folder).join(&block.database);

    let conn = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => {
            return vec![format!("DB: could not open '{}': {}", block.database, e)];
        }
    };

    let mut failures = Vec::new();

    for query in &block.queries {
        let sql = interpolate(&query.sql, vars);
        let query_failures = evaluate_query(&conn, &query.id, &sql, &query.assert);
        for f in query_failures {
            failures.push(format!("DB: {}", f));
        }
    }

    failures
}

// ── Query evaluation ──────────────────────────────────────────────────────────

fn evaluate_query(conn: &Connection, id: &str, sql: &str, assert: &Value) -> Vec<String> {
    // Execute the query and collect all rows as Vec<HashMap<col_name, Value>>
    let mut stmt = match conn.prepare(sql) {
        Ok(s) => s,
        Err(e) => return vec![format!("[{}] SQL error: {}", id, e)],
    };

    let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let rows_result: Result<Vec<HashMap<String, Value>>, _> = stmt
        .query_map([], |row| {
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

// ── Variable interpolation ────────────────────────────────────────────────────

fn interpolate(s: &str, vars: &HashMap<String, Value>) -> String {
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
