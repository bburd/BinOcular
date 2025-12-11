use crate::ast::{FieldType, OffsetKind, Schema};
use crate::error::SchemaError;

pub fn validate_schema(schema: &Schema) -> Result<(), SchemaError> {
    if schema.schema_name.trim().is_empty() {
        return Err(SchemaError::Validation(
            "Schema name must not be empty".to_string(),
        ));
    }

    if schema.schema_version < 1 {
        return Err(SchemaError::Validation(format!(
            "Schema version must be at least 1 (found {})",
            schema.schema_version
        )));
    }

    for (idx, field) in schema.fields.iter().enumerate() {
        if field.name.trim().is_empty() {
            return Err(SchemaError::Validation(format!(
                "Field #{idx} has an empty name"
            )));
        }

        let field_label = format!("Field `{}`", field.name);

        if let OffsetKind::Expr(expr) = &field.offset {
            return Err(SchemaError::Validation(format!(
                "{field_label} uses expression offset `{expr}`, which is not supported in schema v1"
            )));
        }

        if field.repeat.is_some() {
            return Err(SchemaError::Validation(format!(
                "{field_label} uses repeat, which is not allowed in schema v1"
            )));
        }

        match field.ty {
            FieldType::Bytes | FieldType::Ascii => {
                match field.length {
                    Some(length) if length > 0 => {}
                    Some(_) => {
                        return Err(SchemaError::Validation(format!(
                            "{field_label} has length 0; length must be greater than 0"
                        )))
                    }
                    None => {
                        return Err(SchemaError::Validation(format!(
                            "{field_label} must specify length for type {:?}",
                            field.ty
                        )))
                    }
                }
            }
            _ => {
                if field.length.is_some() {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} specifies length but type {:?} does not support length",
                        field.ty
                    )));
                }
            }
        }
    }

    Ok(())
}

pub fn parse_schema_str(yaml: &str) -> Result<Schema, SchemaError> {
    let schema: Schema =
        serde_yaml::from_str(yaml).map_err(|e| SchemaError::Yaml(e.to_string()))?;
    validate_schema(&schema)?;
    Ok(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Endianness, FieldDef, OffsetKind, RepeatInfo};

    fn base_schema() -> Schema {
        Schema {
            schema_name: "test".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![FieldDef {
                name: "field1".to_string(),
                ty: FieldType::Bytes,
                offset: OffsetKind::Absolute(0),
                length: Some(4),
                endianness: None,
                description: None,
                repeat: None,
            }],
        }
    }

    #[test]
    fn validate_schema_accepts_valid_schema() {
        let schema = base_schema();
        assert!(validate_schema(&schema).is_ok());
    }

    #[test]
    fn validate_schema_rejects_empty_schema_name() {
        let mut schema = base_schema();
        schema.schema_name = "  ".into();
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("Schema name"));
    }

    #[test]
    fn validate_schema_rejects_old_version() {
        let mut schema = base_schema();
        schema.schema_version = 0;
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("version"));
    }

    #[test]
    fn validate_schema_rejects_empty_field_name() {
        let mut schema = base_schema();
        schema.fields[0].name = "".into();
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("empty name"));
    }

    #[test]
    fn validate_schema_rejects_expr_offset() {
        let mut schema = base_schema();
        schema.fields[0].offset = OffsetKind::Expr("a+b".into());
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("uses expression offset"));
    }

    #[test]
    fn validate_schema_rejects_repeat() {
        let mut schema = base_schema();
        schema.fields[0].repeat = Some(RepeatInfo { count: 2 });
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("repeat"));
    }

    #[test]
    fn validate_schema_rejects_length_on_numeric() {
        let mut schema = base_schema();
        schema.fields[0].ty = FieldType::U16;
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("length"));
    }

    #[test]
    fn validate_schema_rejects_zero_length() {
        let mut schema = base_schema();
        schema.fields[0].length = Some(0);
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("length 0"));
    }

    #[test]
    fn parse_schema_str_accepts_valid_yaml() {
        let yaml = r#"
schema_name: "Packet"
schema_version: 1
endianness: little
fields:
  - name: "header"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length: 4
  - name: "value"
    type: u16
    offset:
      kind: Absolute
      value: 4
"#;

        let schema = parse_schema_str(yaml).expect("valid schema should parse");
        assert_eq!(schema.schema_name, "Packet");
        assert_eq!(schema.fields.len(), 2);
    }

    fn expect_validation_error(yaml: &str) {
        match parse_schema_str(yaml) {
            Err(SchemaError::Validation(_)) => {}
            other => panic!("expected validation error, got {other:?}"),
        }
    }

    #[test]
    fn parse_schema_rejects_bytes_without_length() {
        let yaml = r#"
schema_name: "NoLength"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_numeric_with_length() {
        let yaml = r#"
schema_name: "NumericLength"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u32
    offset:
      kind: Absolute
      value: 0
    length: 4
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_expression_offset() {
        let yaml = r#"
schema_name: "ExprOffset"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u16
    offset:
      kind: Expr
      value: "a+b"
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_empty_field_name() {
        let yaml = r#"
schema_name: "EmptyField"
schema_version: 1
endianness: little
fields:
  - name: ""
    type: u16
    offset:
      kind: Absolute
      value: 0
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_repeat_usage() {
        let yaml = r#"
schema_name: "WithRepeat"
schema_version: 1
endianness: little
fields:
  - name: "values"
    type: u8
    offset:
      kind: Absolute
      value: 0
    repeat:
      count: 2
"#;

        expect_validation_error(yaml);
    }
}
