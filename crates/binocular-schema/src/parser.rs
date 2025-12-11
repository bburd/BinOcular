use crate::ast::Schema;
use crate::error::SchemaError;

pub fn parse_schema_str(yaml: &str) -> Result<Schema, SchemaError> {
    let schema: Schema = serde_yaml::from_str(yaml)
        .map_err(|e| SchemaError::Yaml(e.to_string()))?;
    // TODO: add structural validation here.
    Ok(schema)
}
