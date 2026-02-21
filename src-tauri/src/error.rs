use std::fmt;

/// Unified error type for the Praxis backend.
///
/// Tauri commands return `Result<T, String>`, so this type implements
/// `Into<String>` for seamless conversion at the command boundary.
/// Internally, modules use `Result<T, PraxisError>` to preserve context.
#[derive(Debug)]
pub enum PraxisError {
    /// Filesystem I/O errors (read, write, delete, create_dir)
    Io(std::io::Error),
    /// JSON parse / serialize errors
    Json(serde_json::Error),
    /// HTTP client errors (reqwest)
    Http(reqwest::Error),
    /// Zip archive errors
    Zip(zip::result::ZipError),
    /// Resource not found (chapter, course, file, etc.)
    NotFound(String),
    /// Server crashed during operation
    ServerCrashed,
    /// Port already in use
    PortInUse(u16),
    /// Version incompatibility
    VersionMismatch { required: String, actual: String },
    /// Generic error with a message (escape hatch for one-off cases)
    Other(String),
}

impl fmt::Display for PraxisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PraxisError::Io(e) => write!(f, "{}", e),
            PraxisError::Json(e) => write!(f, "{}", e),
            PraxisError::Http(e) => write!(f, "{}", e),
            PraxisError::Zip(e) => write!(f, "{}", e),
            PraxisError::NotFound(what) => write!(f, "{}", what),
            PraxisError::ServerCrashed => write!(f, "Server crashed"),
            PraxisError::PortInUse(port) => write!(f, "Port {} is already in use", port),
            PraxisError::VersionMismatch { required, actual } => {
                write!(f, "Course requires app version >= {} but this is {}", required, actual)
            }
            PraxisError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl From<std::io::Error> for PraxisError {
    fn from(e: std::io::Error) -> Self {
        PraxisError::Io(e)
    }
}

impl From<serde_json::Error> for PraxisError {
    fn from(e: serde_json::Error) -> Self {
        PraxisError::Json(e)
    }
}

impl From<reqwest::Error> for PraxisError {
    fn from(e: reqwest::Error) -> Self {
        PraxisError::Http(e)
    }
}

impl From<zip::result::ZipError> for PraxisError {
    fn from(e: zip::result::ZipError) -> Self {
        PraxisError::Zip(e)
    }
}

/// Convert to String for Tauri command boundaries.
impl From<PraxisError> for String {
    fn from(e: PraxisError) -> String {
        e.to_string()
    }
}
