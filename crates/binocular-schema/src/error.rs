use thiserror::Error;

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("YAML parse error: {0}")]
    Yaml(String),

    #[error("Schema validation error: {0}")]
    Validation(String),
}
