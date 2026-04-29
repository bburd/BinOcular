use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("YAML parse error: {0}")]
    Yaml(String),

    #[error("Schema validation error: {0}")]
    Validation(String),

    #[error("Failed to read schema file `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("Schema include cycle detected: {cycle}")]
    IncludeCycle { cycle: String },
}
