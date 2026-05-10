use std::path::PathBuf;

use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaLocation {
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("YAML parse error{location_suffix}: {message}", location_suffix = format_location_suffix(location))]
    Yaml {
        message: String,
        location: Option<SchemaLocation>,
    },

    #[error("Schema validation error{path_suffix}{location_suffix}: {message}", path_suffix = format_path_suffix(path), location_suffix = format_location_suffix(location))]
    Validation {
        message: String,
        path: Option<PathBuf>,
        location: Option<SchemaLocation>,
    },

    #[error("Failed to read schema file `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Schema include cycle detected: {cycle}")]
    IncludeCycle { cycle: String },
}

impl SchemaError {
    pub fn yaml(message: impl Into<String>, location: Option<SchemaLocation>) -> Self {
        Self::Yaml {
            message: message.into(),
            location,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            path: None,
            location: None,
        }
    }

    pub fn with_path(self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        match self {
            Self::Validation {
                message,
                path: None,
                location,
            } => Self::Validation {
                message,
                path: Some(path),
                location,
            },
            other => other,
        }
    }
}

fn format_location_suffix(location: &Option<SchemaLocation>) -> String {
    location
        .as_ref()
        .map(|location| format!(" at line {}, column {}", location.line, location.column))
        .unwrap_or_default()
}

fn format_path_suffix(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| format!(" in `{}`", path.display()))
        .unwrap_or_default()
}
