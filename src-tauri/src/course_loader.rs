use std::fs;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

use crate::error::PraxisError;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Structs matching course.json schema ──────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CourseRuntime {
    pub name: String,
    pub version_min: String,
    pub dependencies: Vec<String>,
    pub server_command: String,
    pub health_endpoint: String,
    pub default_port: u16,
    pub clean_before_run: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Chapter {
    pub id: String,
    pub title: String,
    #[serde(rename = "type", default = "default_chapter_type")]
    pub chapter_type: String,
    pub content_file: String,
    pub test_file: Option<String>,
    pub depends_on: Option<String>,
}

fn default_chapter_type() -> String {
    "lesson".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub description: String,
    pub version: String,
    pub app_version_min: String,
    pub author: String,
    #[serde(default)]
    pub format_version: Option<String>,
    pub runtime: CourseRuntime,
    pub chapters: Vec<Chapter>,
    /// Base path of the course folder (not in JSON — injected after loading)
    #[serde(skip_deserializing)]
    pub base_path: String,
}

// ── Version comparison (simple semver major.minor.patch) ─────────────────────

fn version_ok(app_ver: &str, required_min: &str) -> bool {
    let parse = |v: &str| -> (u32, u32, u32) {
        let parts: Vec<u32> = v.split('.').filter_map(|p| p.parse().ok()).collect();
        (
            parts.get(0).copied().unwrap_or(0),
            parts.get(1).copied().unwrap_or(0),
            parts.get(2).copied().unwrap_or(0),
        )
    };
    parse(app_ver) >= parse(required_min)
}

// ── Core loader ──────────────────────────────────────────────────────────────

pub fn load_course_from_folder(path: &Path) -> Result<Course, PraxisError> {
    let course_json = path.join("course.json");
    if !course_json.exists() {
        return Err(PraxisError::NotFound(
            format!("No course.json found in {}", path.display())
        ));
    }

    let content = fs::read_to_string(&course_json)?;
    let mut course: Course = serde_json::from_str(&content)?;

    if !version_ok(APP_VERSION, &course.app_version_min) {
        return Err(PraxisError::VersionMismatch {
            required: course.app_version_min.clone(),
            actual: APP_VERSION.to_string(),
        });
    }

    // Validate format_version: absent or "1.0" = spec 1 (compatible);
    // "2.0" = current; anything with a higher major = reject.
    let supported_major: u32 = 2;
    if let Some(fv) = &course.format_version {
        let major = fv.split('.')
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        if major > supported_major {
            return Err(PraxisError::Other(format!(
                "Course requires format version {} but this app only supports up to {}.x. Please update Praxis.",
                fv, supported_major
            )));
        }
    }

    course.base_path = path.to_string_lossy().to_string();
    Ok(course)
}

pub fn load_course_from_file(course_file: &Path, data_dir: &Path) -> Result<Course, PraxisError> {
    // Read zip to extract course id first
    let file = fs::File::open(course_file)?;
    let mut archive = zip::ZipArchive::new(file)?;

    // Find course.json in the archive
    let course_json_content = {
        let mut entry = archive.by_name("course.json")?;
        let mut content = String::new();
        std::io::Read::read_to_string(&mut entry, &mut content)?;
        content
    };

    let temp: serde_json::Value = serde_json::from_str(&course_json_content)?;
    let course_id = temp["id"].as_str()
        .ok_or_else(|| PraxisError::NotFound("course.json missing 'id' field".to_string()))?
        .to_string();

    // Extract to <data_dir>/courses/<course_id>/
    let extract_path = data_dir.join("courses").join(&course_id);
    fs::create_dir_all(&extract_path)?;

    // Re-open archive (consumed above)
    let file2 = fs::File::open(course_file)?;
    let mut archive2 = zip::ZipArchive::new(file2)?;
    archive2.extract(&extract_path)?;

    load_course_from_folder(&extract_path)
}

// ── Chapter content / tests ──────────────────────────────────────────────────

pub fn read_chapter_content(course: &Course, chapter_id: &str) -> Result<String, PraxisError> {
    let chapter = course.chapters.iter()
        .find(|c| c.id == chapter_id)
        .ok_or_else(|| PraxisError::NotFound(format!("Chapter '{}' not found", chapter_id)))?;

    let path = PathBuf::from(&course.base_path).join(&chapter.content_file);
    Ok(fs::read_to_string(&path)?)
}

pub fn read_chapter_tests(course: &Course, chapter_id: &str) -> Result<serde_json::Value, PraxisError> {
    let chapter = course.chapters.iter()
        .find(|c| c.id == chapter_id)
        .ok_or_else(|| PraxisError::NotFound(format!("Chapter '{}' not found", chapter_id)))?;

    let test_file = chapter.test_file.as_ref()
        .ok_or_else(|| PraxisError::NotFound(format!("Chapter '{}' has no test file", chapter_id)))?;

    let path = PathBuf::from(&course.base_path).join(test_file);
    let content = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&content)?)
}
