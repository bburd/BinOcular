use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::ast::{Endianness, FieldDef, FieldType, IntExpr, LengthSpec, OffsetKind, Schema};
use crate::error::SchemaError;

pub const MAX_REPEAT_COUNT: u64 = 10_000;

#[derive(Debug, Deserialize)]
struct RawSchemaDoc {
    schema_name: String,
    schema_version: u32,
    endianness: Option<Endianness>,
    fields: Vec<Value>,
}

#[derive(Debug)]
struct RawSchema {
    schema_name: String,
    schema_version: u32,
    endianness: Option<Endianness>,
    fields: Vec<RawFieldItem>,
}

#[derive(Debug)]
enum RawFieldItem {
    Field(FieldDef),
    Include { include: String },
}

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

        match &field.offset {
            OffsetKind::Absolute(_) => {}
            OffsetKind::FieldRef(referenced) => {
                if referenced.trim().is_empty() {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} references an empty offset field name"
                    )));
                }

                if field.repeat.is_some() {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} cannot use dynamic offset together with repeat in schema v1"
                    )));
                }
            }
            OffsetKind::Expr(expr) => {
                validate_int_expr(expr, &field_label, "offset")?;

                if field.repeat.is_some() {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} cannot use dynamic offset together with repeat in schema v1"
                    )));
                }
            }
        }

        if let Some(repeat) = &field.repeat {
            if repeat.count == 0 {
                return Err(SchemaError::Validation(format!(
                    "{field_label} has repeat count 0; count must be greater than 0"
                )));
            }
            if repeat.count > MAX_REPEAT_COUNT {
                return Err(SchemaError::Validation(format!(
                    "{field_label} has repeat count {}; maximum supported count is {}",
                    repeat.count, MAX_REPEAT_COUNT
                )));
            }
        }

        match field.ty {
            FieldType::Bytes | FieldType::Ascii => match &field.length {
                Some(LengthSpec::Literal(length)) if *length > 0 => {}
                Some(LengthSpec::Literal(_)) => {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} has length 0; length must be greater than 0"
                    )))
                }
                Some(LengthSpec::FieldRef { field: referenced }) => {
                    if referenced.trim().is_empty() {
                        return Err(SchemaError::Validation(format!(
                            "{field_label} references an empty length field name"
                        )));
                    }
                    if field.repeat.is_some() {
                        return Err(SchemaError::Validation(format!(
                            "{field_label} cannot use dynamic length together with repeat in schema v1"
                        )));
                    }
                }
                Some(LengthSpec::Expr { expr }) => {
                    validate_int_expr(expr, &field_label, "length")?;

                    if field.repeat.is_some() {
                        return Err(SchemaError::Validation(format!(
                            "{field_label} cannot use dynamic length together with repeat in schema v1"
                        )));
                    }
                }
                None => {
                    return Err(SchemaError::Validation(format!(
                        "{field_label} must specify length for type {:?}",
                        field.ty
                    )))
                }
            },
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

fn validate_int_expr(expr: &IntExpr, field_label: &str, usage: &str) -> Result<(), SchemaError> {
    match expr {
        IntExpr::Const { .. } => Ok(()),
        IntExpr::FieldRef { field } => {
            if field.trim().is_empty() {
                Err(SchemaError::Validation(format!(
                    "{field_label} references an empty {usage} expression field name"
                )))
            } else {
                Ok(())
            }
        }
        IntExpr::Binary { left, right, .. } => {
            validate_int_expr(left, field_label, usage)?;
            validate_int_expr(right, field_label, usage)
        }
    }
}

pub fn parse_schema_str(yaml: &str) -> Result<Schema, SchemaError> {
    let raw = parse_raw_schema_str(yaml)?;
    if contains_include(&raw) {
        return Err(SchemaError::Validation(
            "Schema includes require file-based loading via parse_schema_file".to_string(),
        ));
    }

    let schema = schema_from_raw(raw, Vec::new());
    validate_schema(&schema)?;
    Ok(schema)
}

pub fn parse_schema_file(path: impl AsRef<Path>) -> Result<Schema, SchemaError> {
    let path = absolutize_path(path.as_ref())?;
    let mut stack = Vec::new();
    parse_schema_file_inner(&path, &mut stack, None)
}

fn parse_schema_file_inner(
    path: &Path,
    stack: &mut Vec<PathBuf>,
    root_endianness: Option<Endianness>,
) -> Result<Schema, SchemaError> {
    let normalized = normalize_for_cycle(path)?;

    if let Some(cycle_start) = stack.iter().position(|seen| seen == &normalized) {
        let cycle = stack[cycle_start..]
            .iter()
            .chain(std::iter::once(&normalized))
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(" -> ");
        return Err(SchemaError::IncludeCycle { cycle });
    }

    stack.push(normalized.clone());

    let result = (|| {
        let yaml = fs::read_to_string(&normalized).map_err(|source| SchemaError::Io {
            path: normalized.clone(),
            source,
        })?;
        let raw = parse_raw_schema_str(&yaml)?;
        let root_endianness = if stack.len() == 1 {
            raw.endianness
        } else {
            root_endianness
        };
        validate_include_endianness(root_endianness, raw.endianness, &normalized)?;
        let base_dir = normalized
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut fields = Vec::new();
        for item in raw.fields {
            match item {
                RawFieldItem::Field(field) => fields.push(field),
                RawFieldItem::Include { include } => {
                    let include_path = base_dir.join(include);
                    let included_schema =
                        parse_schema_file_inner(&include_path, stack, root_endianness)?;
                    fields.extend(included_schema.fields);
                }
            }
        }

        let schema = Schema {
            schema_name: raw.schema_name,
            schema_version: raw.schema_version,
            endianness: raw.endianness,
            fields,
        };
        validate_schema(&schema)?;
        Ok(schema)
    })();

    stack.pop();
    result
}

fn parse_raw_schema_str(yaml: &str) -> Result<RawSchema, SchemaError> {
    let raw_doc: RawSchemaDoc =
        serde_yaml::from_str(yaml).map_err(|e| SchemaError::Yaml(e.to_string()))?;

    let fields = raw_doc
        .fields
        .into_iter()
        .map(parse_raw_field_item)
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RawSchema {
        schema_name: raw_doc.schema_name,
        schema_version: raw_doc.schema_version,
        endianness: raw_doc.endianness,
        fields,
    })
}

fn parse_raw_field_item(value: Value) -> Result<RawFieldItem, SchemaError> {
    let mapping = value.as_mapping().ok_or_else(|| {
        SchemaError::Yaml("invalid type: expected a mapping for each field item".to_string())
    })?;

    if let Some(include_value) = find_string_key(mapping, "include") {
        if mapping.len() != 1 {
            return Err(SchemaError::Validation(
                "Include items must contain exactly one key: `include`".to_string(),
            ));
        }

        if include_value.trim().is_empty() {
            return Err(SchemaError::Validation(
                "Include path must not be empty".to_string(),
            ));
        }

        return Ok(RawFieldItem::Include {
            include: include_value.to_string(),
        });
    }

    let field =
        serde_yaml::from_value::<FieldDef>(value).map_err(|e| SchemaError::Yaml(e.to_string()))?;
    Ok(RawFieldItem::Field(field))
}

fn contains_include(raw: &RawSchema) -> bool {
    raw.fields
        .iter()
        .any(|item| matches!(item, RawFieldItem::Include { .. }))
}

fn schema_from_raw(raw: RawSchema, mut fields: Vec<FieldDef>) -> Schema {
    for item in raw.fields {
        if let RawFieldItem::Field(field) = item {
            fields.push(field);
        }
    }

    Schema {
        schema_name: raw.schema_name,
        schema_version: raw.schema_version,
        endianness: raw.endianness,
        fields,
    }
}

fn validate_include_endianness(
    root_endianness: Option<Endianness>,
    schema_endianness: Option<Endianness>,
    path: &Path,
) -> Result<(), SchemaError> {
    if let (Some(root), Some(schema)) = (root_endianness, schema_endianness) {
        if root != schema {
            return Err(SchemaError::Validation(format!(
                "Included schema `{}` declares {:?} endianness, but root schema declares {:?}; schema-level defaults must match for includes in v1",
                path.display(),
                schema,
                root
            )));
        }
    }

    Ok(())
}

fn find_string_key<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a str> {
    mapping
        .get(Value::String(key.to_string()))
        .and_then(Value::as_str)
}

fn absolutize_path(path: &Path) -> Result<PathBuf, SchemaError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    let cwd = std::env::current_dir().map_err(|source| SchemaError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(cwd.join(path))
}

fn normalize_for_cycle(path: &Path) -> Result<PathBuf, SchemaError> {
    let absolute = absolutize_path(path)?;
    Ok(fs::canonicalize(&absolute).unwrap_or(absolute))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{FieldDef, IntExprOp, RepeatInfo};
    use proptest::prelude::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn base_schema() -> Schema {
        Schema {
            schema_name: "test".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![FieldDef {
                name: "field1".to_string(),
                ty: FieldType::Bytes,
                offset: OffsetKind::Absolute(0),
                length: Some(LengthSpec::Literal(4)),
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
        schema.fields[0].offset = OffsetKind::Expr(IntExpr::FieldRef { field: "".into() });
        let err = validate_schema(&schema).unwrap_err();
        assert!(err
            .to_string()
            .contains("empty offset expression field name"));
    }

    #[test]
    fn validate_schema_accepts_repeat() {
        let mut schema = base_schema();
        schema.fields[0].repeat = Some(RepeatInfo { count: 2 });
        assert!(validate_schema(&schema).is_ok());
    }

    #[test]
    fn validate_schema_rejects_zero_repeat_count() {
        let mut schema = base_schema();
        schema.fields[0].repeat = Some(RepeatInfo { count: 0 });
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("repeat count 0"));
    }

    #[test]
    fn validate_schema_rejects_repeat_count_over_max() {
        let mut schema = base_schema();
        schema.fields[0].repeat = Some(RepeatInfo {
            count: MAX_REPEAT_COUNT + 1,
        });
        let err = validate_schema(&schema).unwrap_err();
        assert!(err.to_string().contains("maximum supported count"));
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
        schema.fields[0].length = Some(LengthSpec::Literal(0));
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

    #[test]
    fn parse_schema_str_rejects_include_usage() {
        let yaml = r#"
schema_name: "Packet"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#;

        let err = parse_schema_str(yaml).unwrap_err();
        assert!(err.to_string().contains("file-based loading"));
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
    fn parse_schema_accepts_dynamic_length() {
        let yaml = r#"
schema_name: "DynamicLength"
schema_version: 1
endianness: little
fields:
  - name: "block_len"
    type: u16
    offset:
      kind: Absolute
      value: 0
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 2
    length:
      field: "block_len"
"#;

        let schema = parse_schema_str(yaml).expect("dynamic length should parse");
        assert!(matches!(
            schema.fields[1].length,
            Some(LengthSpec::FieldRef { ref field }) if field == "block_len"
        ));
    }

    #[test]
    fn parse_schema_accepts_expression_length() {
        let yaml = r#"
schema_name: "ExprLength"
schema_version: 1
endianness: little
fields:
  - name: "block_len"
    type: u16
    offset:
      kind: Absolute
      value: 0
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 2
    length:
      expr:
        op: sub
        left:
          field: "block_len"
        right:
          const: 4
"#;

        let schema = parse_schema_str(yaml).expect("expression length should parse");
        assert!(matches!(
            schema.fields[1].length,
            Some(LengthSpec::Expr {
                expr: IntExpr::Binary {
                    op: IntExprOp::Sub,
                    ..
                }
            })
        ));
    }

    #[test]
    fn parse_schema_accepts_dynamic_offset() {
        let yaml = r#"
schema_name: "DynamicOffset"
schema_version: 1
endianness: little
fields:
  - name: "data_offset"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - name: "payload"
    type: bytes
    offset:
      kind: FieldRef
      value: "data_offset"
    length: 4
"#;

        let schema = parse_schema_str(yaml).expect("dynamic offset should parse");
        assert!(matches!(
            schema.fields[1].offset,
            OffsetKind::FieldRef(ref field) if field == "data_offset"
        ));
    }

    #[test]
    fn parse_schema_accepts_expression_offset() {
        let yaml = r#"
schema_name: "ExprOffset"
schema_version: 1
endianness: little
fields:
  - name: "data_offset"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - name: "payload"
    type: bytes
    offset:
      kind: Expr
      value:
        op: add
        left:
          field: "data_offset"
        right:
          const: 4
    length: 4
"#;

        let schema = parse_schema_str(yaml).expect("expression offset should parse");
        assert!(matches!(
            schema.fields[1].offset,
            OffsetKind::Expr(IntExpr::Binary {
                op: IntExprOp::Add,
                ..
            })
        ));
    }

    #[test]
    fn parse_schema_rejects_dynamic_length_on_repeated_field() {
        let yaml = r#"
schema_name: "DynamicRepeated"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length:
      field: "block_len"
    repeat:
      count: 2
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_expression_length_on_repeated_field() {
        let yaml = r#"
schema_name: "DynamicRepeated"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length:
      expr:
        op: sub
        left:
          field: "block_len"
        right:
          const: 1
    repeat:
      count: 2
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_empty_dynamic_offset_reference() {
        let yaml = r#"
schema_name: "DynamicOffset"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: FieldRef
      value: ""
    length: 4
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_dynamic_offset_on_repeated_field() {
        let yaml = r#"
schema_name: "DynamicOffsetRepeat"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: FieldRef
      value: "data_offset"
    length: 4
    repeat:
      count: 2
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_expression_offset_on_repeated_field() {
        let yaml = r#"
schema_name: "DynamicOffsetRepeat"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Expr
      value:
        op: add
        left:
          field: "data_offset"
        right:
          const: 4
    length: 4
    repeat:
      count: 2
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_string_expression_offset_payload() {
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

        let err = parse_schema_str(yaml).unwrap_err();
        assert!(matches!(err, SchemaError::Yaml(_)));
    }

    #[test]
    fn parse_schema_rejects_expression_with_empty_field_ref() {
        let yaml = r#"
schema_name: "ExprOffset"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length:
      expr:
        op: add
        left:
          field: ""
        right:
          const: 4
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_expression_with_unsupported_op() {
        let yaml = r#"
schema_name: "ExprOffset"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length:
      expr:
        op: mul
        left:
          const: 2
        right:
          const: 4
"#;

        let err = parse_schema_str(yaml).unwrap_err();
        assert!(matches!(err, SchemaError::Yaml(_)));
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
    fn parse_schema_accepts_repeat_usage() {
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

        let schema = parse_schema_str(yaml).expect("repeat should be accepted");
        assert_eq!(schema.fields[0].repeat.as_ref().map(|r| r.count), Some(2));
    }

    #[test]
    fn parse_schema_rejects_zero_repeat_count() {
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
      count: 0
"#;

        expect_validation_error(yaml);
    }

    #[test]
    fn parse_schema_rejects_repeat_count_over_max() {
        let yaml = format!(
            r#"
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
      count: {}
"#,
            MAX_REPEAT_COUNT + 1
        );

        expect_validation_error(&yaml);
    }

    #[test]
    fn parse_schema_file_preserves_include_field_order() {
        let dir = temp_test_dir("include_order");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - name: "magic"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - include: "common.yaml"
  - name: "tail"
    type: bytes
    offset:
      kind: Absolute
      value: 16
    length: 4
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "version"
    type: u16
    offset:
      kind: Absolute
      value: 4
  - name: "flags"
    type: u16
    offset:
      kind: Absolute
      value: 6
"#,
        );

        let schema = parse_schema_file(dir.join("root.yaml")).expect("include should resolve");
        let names = schema
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["magic", "version", "flags", "tail"]);

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_supports_nested_relative_includes() {
        let dir = temp_test_dir("nested_include");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "parts/common.yaml"
  - name: "tail"
    type: bytes
    offset:
      kind: Absolute
      value: 12
    length: 2
"#,
        );
        write_file(
            dir.join("parts/common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "magic"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - include: "../shared/more.yaml"
"#,
        );
        write_file(
            dir.join("shared/more.yaml"),
            r#"
schema_name: "More"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u32
    offset:
      kind: Absolute
      value: 4
"#,
        );

        let schema = parse_schema_file(dir.join("root.yaml")).expect("nested include should work");
        let names = schema
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, vec!["magic", "value", "tail"]);

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_missing_include_file() {
        let dir = temp_test_dir("missing_include");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "missing.yaml"
"#,
        );

        let err = parse_schema_file(dir.join("root.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::Io { .. }));
        assert!(err.to_string().contains("missing.yaml"));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_include_cycles() {
        let dir = temp_test_dir("include_cycle");
        write_file(
            dir.join("a.yaml"),
            r#"
schema_name: "A"
schema_version: 1
endianness: little
fields:
  - include: "b.yaml"
"#,
        );
        write_file(
            dir.join("b.yaml"),
            r#"
schema_name: "B"
schema_version: 1
endianness: little
fields:
  - include: "a.yaml"
"#,
        );

        let err = parse_schema_file(dir.join("a.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::IncludeCycle { .. }));
        assert!(err.to_string().contains("cycle"));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_include_items_with_extra_keys() {
        let dir = temp_test_dir("include_extra_keys");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
    offset: 10
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u8
    offset:
      kind: Absolute
      value: 0
"#,
        );

        let err = parse_schema_file(dir.join("root.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::Validation(_)));
        assert!(err.to_string().contains("exactly one key"));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_accepts_valid_repeat_in_included_fields() {
        let dir = temp_test_dir("include_repeat_valid");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "values"
    type: u8
    offset:
      kind: Absolute
      value: 0
    repeat:
      count: 3
"#,
        );

        let schema = parse_schema_file(dir.join("root.yaml")).expect("repeat should be valid");
        assert_eq!(
            schema.fields[0].repeat.as_ref().map(|repeat| repeat.count),
            Some(3)
        );

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_invalid_repeat_in_included_fields() {
        let dir = temp_test_dir("include_repeat_invalid");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "values"
    type: u8
    offset:
      kind: Absolute
      value: 0
    repeat:
      count: 0
"#,
        );

        let err = parse_schema_file(dir.join("root.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::Validation(_)));
        assert!(err.to_string().contains("repeat count 0"));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_string_expression_offsets_in_included_fields() {
        let dir = temp_test_dir("include_expr_offset");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u16
    offset:
      kind: Expr
      value: "a+b"
"#,
        );

        let err = parse_schema_file(dir.join("root.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::Yaml(_)));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_rejects_conflicting_include_endianness() {
        let dir = temp_test_dir("include_endianness_conflict");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: big
fields:
  - name: "value"
    type: u16
    offset:
      kind: Absolute
      value: 0
"#,
        );

        let err = parse_schema_file(dir.join("root.yaml")).unwrap_err();
        assert!(matches!(err, SchemaError::Validation(_)));
        let message = err.to_string();
        assert!(message.contains("common.yaml"));
        assert!(message.contains("Big"));
        assert!(message.contains("Little"));
        assert!(message.contains("must match for includes in v1"));

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_accepts_matching_include_endianness() {
        let dir = temp_test_dir("include_endianness_match");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u16
    offset:
      kind: Absolute
      value: 0
"#,
        );

        let schema =
            parse_schema_file(dir.join("root.yaml")).expect("matching include should work");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "value");

        cleanup_test_dir(dir);
    }

    #[test]
    fn parse_schema_file_keeps_field_level_endianness_in_includes() {
        let dir = temp_test_dir("include_field_endianness");
        write_file(
            dir.join("root.yaml"),
            r#"
schema_name: "Root"
schema_version: 1
fields:
  - include: "common.yaml"
"#,
        );
        write_file(
            dir.join("common.yaml"),
            r#"
schema_name: "Common"
schema_version: 1
endianness: big
fields:
  - name: "value"
    type: u16
    offset:
      kind: Absolute
      value: 0
    endianness: little
"#,
        );

        let schema = parse_schema_file(dir.join("root.yaml"))
            .expect("include without root default should work");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].endianness, Some(Endianness::Little));

        cleanup_test_dir(dir);
    }

    fn temp_test_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "binocular_schema_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&dir).expect("failed to create temp dir");
        dir
    }

    fn write_file(path: PathBuf, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("failed to create parent directory");
        }
        fs::write(path, contents).expect("failed to write test file");
    }

    fn cleanup_test_dir(dir: PathBuf) {
        let _ = fs::remove_dir_all(dir);
    }

    fn arb_endianness() -> impl Strategy<Value = Endianness> {
        prop_oneof![Just(Endianness::Little), Just(Endianness::Big),]
    }

    fn arb_field_type() -> impl Strategy<Value = FieldType> {
        prop_oneof![
            Just(FieldType::U8),
            Just(FieldType::U16),
            Just(FieldType::U32),
            Just(FieldType::U64),
            Just(FieldType::I32),
            Just(FieldType::F32),
            Just(FieldType::Bytes),
            Just(FieldType::Ascii),
        ]
    }

    fn arb_int_expr() -> impl Strategy<Value = IntExpr> {
        let leaf = prop_oneof![
            any::<i64>().prop_map(|value| IntExpr::Const { value }),
            any::<String>().prop_map(|field| IntExpr::FieldRef { field }),
        ];

        leaf.prop_recursive(4, 16, 2, |inner| {
            prop_oneof![
                (inner.clone(), inner.clone()).prop_map(|(left, right)| IntExpr::Binary {
                    op: IntExprOp::Add,
                    left: Box::new(left),
                    right: Box::new(right),
                }),
                (inner.clone(), inner).prop_map(|(left, right)| IntExpr::Binary {
                    op: IntExprOp::Sub,
                    left: Box::new(left),
                    right: Box::new(right),
                }),
            ]
        })
    }

    fn arb_offset_kind() -> impl Strategy<Value = OffsetKind> {
        prop_oneof![
            any::<u64>().prop_map(OffsetKind::Absolute),
            any::<String>().prop_map(OffsetKind::FieldRef),
            arb_int_expr().prop_map(OffsetKind::Expr),
        ]
    }

    fn arb_length_spec() -> impl Strategy<Value = LengthSpec> {
        prop_oneof![
            any::<u64>().prop_map(LengthSpec::Literal),
            any::<String>().prop_map(|field| LengthSpec::FieldRef { field }),
            arb_int_expr().prop_map(|expr| LengthSpec::Expr { expr }),
        ]
    }

    fn arb_repeat_info() -> impl Strategy<Value = crate::ast::RepeatInfo> {
        any::<u64>().prop_map(|count| crate::ast::RepeatInfo { count })
    }

    fn arb_field_def() -> impl Strategy<Value = FieldDef> {
        (
            any::<String>(),
            arb_field_type(),
            arb_offset_kind(),
            proptest::option::of(arb_length_spec()),
            proptest::option::of(arb_endianness()),
            proptest::option::of(any::<String>()),
            proptest::option::of(arb_repeat_info()),
        )
            .prop_map(
                |(name, ty, offset, length, endianness, description, repeat)| FieldDef {
                    name,
                    ty,
                    offset,
                    length,
                    endianness,
                    description,
                    repeat,
                },
            )
    }

    fn arb_schema() -> impl Strategy<Value = Schema> {
        (
            any::<String>(),
            any::<u32>(),
            proptest::option::of(arb_endianness()),
            prop::collection::vec(arb_field_def(), 0..16),
        )
            .prop_map(|(schema_name, schema_version, endianness, fields)| Schema {
                schema_name,
                schema_version,
                endianness,
                fields,
            })
    }

    proptest! {
        #[test]
        fn parse_schema_str_is_panic_safe_for_arbitrary_yaml(input in any::<String>()) {
            let caught = catch_unwind(AssertUnwindSafe(|| parse_schema_str(&input)));
            prop_assert!(caught.is_ok(), "parse_schema_str panicked for input: {:?}", input);

            match caught.expect("already checked is_ok") {
                Ok(_) | Err(SchemaError::Yaml(_)) | Err(SchemaError::Validation(_)) => {}
                Err(other) => prop_assert!(false, "unexpected error variant: {:?}", other),
            }
        }

        #[test]
        fn validate_schema_is_panic_safe_for_arbitrary_schema(schema in arb_schema()) {
            let caught = catch_unwind(AssertUnwindSafe(|| validate_schema(&schema)));
            prop_assert!(caught.is_ok(), "validate_schema panicked for schema: {:?}", schema);

            match caught.expect("already checked is_ok") {
                Ok(()) | Err(SchemaError::Validation(_)) => {}
                Err(other) => prop_assert!(false, "unexpected error variant: {:?}", other),
            }
        }
    }
}
