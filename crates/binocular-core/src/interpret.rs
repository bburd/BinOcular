use std::collections::HashMap;

use crate::buffer::FileBuffer;
use crate::error::InterpretError;
use binocular_schema::ast::{
    Endianness, FieldDef, FieldType, IntExpr, LengthSpec, OffsetKind, Schema,
};

#[derive(Debug, Clone)]
pub enum FieldValue {
    UInt(u64),
    Int(i64),
    Float(f32),
    Bytes(Vec<u8>),
    Ascii(String),
}

#[derive(Debug, Clone)]
pub struct FieldEval {
    pub field: FieldDef,
    pub display_name: String,
    pub resolved_offset: u64,
    pub offset_valid: bool,
    pub byte_len: usize,
    pub value: Option<FieldValue>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum NumericContextValue {
    Unsigned(u64),
    Signed(i64),
}

#[derive(Debug, Clone, Copy)]
enum ContextValue {
    Numeric(NumericContextValue),
    NonNumeric,
}

type FieldContext = HashMap<String, ContextValue>;

pub fn interpret_field<B: FileBuffer + ?Sized>(
    buffer: &B,
    field: &FieldDef,
    schema: Option<&Schema>,
) -> Result<FieldValue, InterpretError> {
    let context = FieldContext::new();
    let len = resolve_field_byte_len(field, &context)?;
    let offset = resolve_base_offset(field, &context)?;
    interpret_field_at(buffer, field, schema, offset, len)
}

fn interpret_field_at<B: FileBuffer + ?Sized>(
    buffer: &B,
    field: &FieldDef,
    schema: Option<&Schema>,
    offset: u64,
    len: usize,
) -> Result<FieldValue, InterpretError> {
    let bytes = buffer.read_bytes(offset, len)?;

    let endianness = field
        .endianness
        .or_else(|| schema.and_then(|s| s.endianness))
        .unwrap_or(Endianness::Little);

    let value = match field.ty {
        FieldType::U8 => FieldValue::UInt(bytes[0] as u64),
        FieldType::U16 => {
            let v = read_u16(bytes, endianness)?;
            FieldValue::UInt(v as u64)
        }
        FieldType::U32 => {
            let v = read_u32(bytes, endianness)?;
            FieldValue::UInt(v as u64)
        }
        FieldType::U64 => {
            let v = read_u64(bytes, endianness)?;
            FieldValue::UInt(v)
        }
        FieldType::I32 => {
            let v = read_i32(bytes, endianness)?;
            FieldValue::Int(v as i64)
        }
        FieldType::F32 => {
            let v = read_f32(bytes, endianness)?;
            FieldValue::Float(v)
        }
        FieldType::Bytes => FieldValue::Bytes(bytes.to_vec()),
        FieldType::Ascii => {
            let s = String::from_utf8_lossy(bytes).to_string();
            FieldValue::Ascii(s)
        }
    };

    Ok(value)
}

pub fn interpret_schema<B: FileBuffer + ?Sized>(buffer: &B, schema: &Schema) -> Vec<FieldEval> {
    let mut rows = Vec::new();
    let mut context = FieldContext::new();

    for field in &schema.fields {
        let count = field.repeat.as_ref().map_or(1, |repeat| repeat.count);

        for index in 0..count {
            let display_name = if field.repeat.is_some() {
                format!("{}[{index}]", field.name)
            } else {
                field.name.clone()
            };

            let resolved = resolve_repeated_offset(field, index, &context);
            let resolved_offset = match resolved {
                Ok(offset) => offset,
                Err(err) => {
                    rows.push(FieldEval {
                        field: field.clone(),
                        display_name,
                        resolved_offset: 0,
                        offset_valid: false,
                        byte_len: 0,
                        value: None,
                        error: Some(err.to_string()),
                    });
                    continue;
                }
            };

            let byte_len = match resolve_field_byte_len(field, &context) {
                Ok(byte_len) => byte_len,
                Err(err) => {
                    rows.push(FieldEval {
                        field: field.clone(),
                        display_name,
                        resolved_offset,
                        offset_valid: true,
                        byte_len: 0,
                        value: None,
                        error: Some(err.to_string()),
                    });
                    continue;
                }
            };

            let value = interpret_field_at(buffer, field, Some(schema), resolved_offset, byte_len);
            match value {
                Ok(value) => {
                    record_context_value(&mut context, &display_name, &value);
                    rows.push(FieldEval {
                        field: field.clone(),
                        display_name,
                        resolved_offset,
                        offset_valid: true,
                        byte_len,
                        value: Some(value),
                        error: None,
                    })
                }
                Err(err) => rows.push(FieldEval {
                    field: field.clone(),
                    display_name,
                    resolved_offset,
                    offset_valid: true,
                    byte_len,
                    value: None,
                    error: Some(err.to_string()),
                }),
            }
        }
    }

    rows
}

fn fixed_field_byte_len(field: &FieldDef) -> Result<usize, InterpretError> {
    match field.ty {
        FieldType::U8 => Ok(1),
        FieldType::U16 => Ok(2),
        FieldType::U32 | FieldType::I32 | FieldType::F32 => Ok(4),
        FieldType::U64 => Ok(8),
        FieldType::Bytes | FieldType::Ascii => match &field.length {
            Some(LengthSpec::Literal(length)) => {
                usize::try_from(*length).map_err(|_| InterpretError::LengthOverflow {
                    field: field.name.clone(),
                })
            }
            Some(LengthSpec::FieldRef { .. } | LengthSpec::Expr { .. }) => {
                Err(InterpretError::Unsupported)
            }
            None => Ok(0),
        },
    }
}

fn resolve_base_offset(field: &FieldDef, context: &FieldContext) -> Result<u64, InterpretError> {
    match &field.offset {
        OffsetKind::Absolute(offset) => Ok(*offset),
        OffsetKind::FieldRef(referenced) => resolve_dynamic_offset(referenced, context),
        OffsetKind::Expr(expr) => resolve_offset_expr(expr, context),
    }
}

fn resolve_field_byte_len(
    field: &FieldDef,
    context: &FieldContext,
) -> Result<usize, InterpretError> {
    match field.ty {
        FieldType::Bytes | FieldType::Ascii => match &field.length {
            Some(LengthSpec::Literal(length)) => {
                usize::try_from(*length).map_err(|_| InterpretError::LengthOverflow {
                    field: field.name.clone(),
                })
            }
            Some(LengthSpec::FieldRef { field: referenced }) => {
                resolve_dynamic_length(referenced, context)
            }
            Some(LengthSpec::Expr { expr }) => resolve_length_expr(expr, context),
            None => Ok(0),
        },
        _ => fixed_field_byte_len(field),
    }
}

fn resolve_length_expr(expr: &IntExpr, context: &FieldContext) -> Result<usize, InterpretError> {
    let value = eval_int_expr(expr, context)?;
    if value < 0 {
        return Err(InterpretError::NegativeExpressionLength);
    }
    if value == 0 {
        return Err(InterpretError::ZeroExpressionLength);
    }

    usize::try_from(value as u64).map_err(|_| InterpretError::ExpressionLengthOverflow)
}

fn resolve_offset_expr(expr: &IntExpr, context: &FieldContext) -> Result<u64, InterpretError> {
    let value = eval_int_expr(expr, context)?;
    if value < 0 {
        return Err(InterpretError::NegativeExpressionOffset);
    }

    Ok(value as u64)
}

fn eval_int_expr(expr: &IntExpr, context: &FieldContext) -> Result<i64, InterpretError> {
    match expr {
        IntExpr::Const { value } => Ok(*value),
        IntExpr::FieldRef { field } => resolve_expression_reference(field, context),
        IntExpr::Binary { op, left, right } => {
            let left = eval_int_expr(left, context)?;
            let right = eval_int_expr(right, context)?;

            match op {
                binocular_schema::ast::IntExprOp::Add => left
                    .checked_add(right)
                    .ok_or(InterpretError::ExpressionOverflow),
                binocular_schema::ast::IntExprOp::Sub => left
                    .checked_sub(right)
                    .ok_or(InterpretError::ExpressionOverflow),
            }
        }
    }
}

fn resolve_expression_reference(
    referenced: &str,
    context: &FieldContext,
) -> Result<i64, InterpretError> {
    let Some(value) = context.get(referenced).copied() else {
        return Err(InterpretError::MissingExpressionReference {
            field: referenced.to_string(),
        });
    };

    match value {
        ContextValue::NonNumeric => Err(InterpretError::InvalidExpressionReferenceType {
            field: referenced.to_string(),
        }),
        ContextValue::Numeric(NumericContextValue::Unsigned(value)) => i64::try_from(value)
            .map_err(|_| InterpretError::ExpressionReferenceOverflow {
                field: referenced.to_string(),
            }),
        ContextValue::Numeric(NumericContextValue::Signed(value)) => Ok(value),
    }
}

fn resolve_dynamic_length(
    referenced: &str,
    context: &FieldContext,
) -> Result<usize, InterpretError> {
    let Some(value) = context.get(referenced).copied() else {
        return Err(InterpretError::MissingLengthReference {
            field: referenced.to_string(),
        });
    };

    match value {
        ContextValue::NonNumeric => Err(InterpretError::InvalidLengthReferenceType {
            field: referenced.to_string(),
        }),
        ContextValue::Numeric(NumericContextValue::Unsigned(value)) => usize::try_from(value)
            .map_err(|_| InterpretError::LengthOverflow {
                field: referenced.to_string(),
            }),
        ContextValue::Numeric(NumericContextValue::Signed(value)) => {
            if value < 0 {
                return Err(InterpretError::NegativeLengthReference {
                    field: referenced.to_string(),
                });
            }

            usize::try_from(value as u64).map_err(|_| InterpretError::LengthOverflow {
                field: referenced.to_string(),
            })
        }
    }
}

fn resolve_dynamic_offset(referenced: &str, context: &FieldContext) -> Result<u64, InterpretError> {
    let Some(value) = context.get(referenced).copied() else {
        return Err(InterpretError::MissingOffsetReference {
            field: referenced.to_string(),
        });
    };

    match value {
        ContextValue::NonNumeric => Err(InterpretError::InvalidOffsetReferenceType {
            field: referenced.to_string(),
        }),
        ContextValue::Numeric(NumericContextValue::Unsigned(value)) => Ok(value),
        ContextValue::Numeric(NumericContextValue::Signed(value)) => {
            if value < 0 {
                return Err(InterpretError::NegativeOffsetReference {
                    field: referenced.to_string(),
                });
            }

            Ok(value as u64)
        }
    }
}

fn resolve_repeated_offset(
    field: &FieldDef,
    index: u64,
    context: &FieldContext,
) -> Result<u64, InterpretError> {
    let base_offset = resolve_base_offset(field, context)?;
    if index == 0 {
        return Ok(base_offset);
    }

    let stride =
        u64::try_from(fixed_field_byte_len(field)?).map_err(|_| InterpretError::OffsetOverflow)?;
    let repeated_bytes = index
        .checked_mul(stride)
        .ok_or(InterpretError::OffsetOverflow)?;

    base_offset
        .checked_add(repeated_bytes)
        .ok_or(InterpretError::OffsetOverflow)
}

fn record_context_value(context: &mut FieldContext, display_name: &str, value: &FieldValue) {
    let context_value = match value {
        FieldValue::UInt(value) => ContextValue::Numeric(NumericContextValue::Unsigned(*value)),
        FieldValue::Int(value) => ContextValue::Numeric(NumericContextValue::Signed(*value)),
        FieldValue::Float(_) | FieldValue::Bytes(_) | FieldValue::Ascii(_) => {
            ContextValue::NonNumeric
        }
    };
    context.insert(display_name.to_string(), context_value);
}

fn read_u16(bytes: &[u8], endianness: Endianness) -> Result<u16, InterpretError> {
    let bytes: [u8; 2] = bytes
        .try_into()
        .map_err(|_| InterpretError::InvalidNumericByteWidth {
            expected: 2,
            actual: bytes.len(),
        })?;

    match endianness {
        Endianness::Little => Ok(u16::from_le_bytes(bytes)),
        Endianness::Big => Ok(u16::from_be_bytes(bytes)),
    }
}

fn read_u32(bytes: &[u8], endianness: Endianness) -> Result<u32, InterpretError> {
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| InterpretError::InvalidNumericByteWidth {
            expected: 4,
            actual: bytes.len(),
        })?;

    match endianness {
        Endianness::Little => Ok(u32::from_le_bytes(bytes)),
        Endianness::Big => Ok(u32::from_be_bytes(bytes)),
    }
}

fn read_u64(bytes: &[u8], endianness: Endianness) -> Result<u64, InterpretError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| InterpretError::InvalidNumericByteWidth {
            expected: 8,
            actual: bytes.len(),
        })?;

    match endianness {
        Endianness::Little => Ok(u64::from_le_bytes(bytes)),
        Endianness::Big => Ok(u64::from_be_bytes(bytes)),
    }
}

fn read_i32(bytes: &[u8], endianness: Endianness) -> Result<i32, InterpretError> {
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| InterpretError::InvalidNumericByteWidth {
            expected: 4,
            actual: bytes.len(),
        })?;

    match endianness {
        Endianness::Little => Ok(i32::from_le_bytes(bytes)),
        Endianness::Big => Ok(i32::from_be_bytes(bytes)),
    }
}

fn read_f32(bytes: &[u8], endianness: Endianness) -> Result<f32, InterpretError> {
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| InterpretError::InvalidNumericByteWidth {
            expected: 4,
            actual: bytes.len(),
        })?;

    match endianness {
        Endianness::Little => Ok(f32::from_le_bytes(bytes)),
        Endianness::Big => Ok(f32::from_be_bytes(bytes)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::MemoryBuffer;
    use binocular_schema::ast::{
        Endianness, FieldDef, FieldType, IntExpr, IntExprOp, LengthSpec, OffsetKind, Schema,
    };

    fn make_field(
        name: &str,
        ty: FieldType,
        offset: u64,
        endianness: Option<Endianness>,
    ) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            offset: OffsetKind::Absolute(offset),
            length: None,
            endianness,
            description: None,
            repeat: None,
        }
    }

    fn make_sized_field(name: &str, ty: FieldType, offset: u64, length: LengthSpec) -> FieldDef {
        FieldDef {
            name: name.to_string(),
            ty,
            offset: OffsetKind::Absolute(offset),
            length: Some(length),
            endianness: None,
            description: None,
            repeat: None,
        }
    }

    fn expr_const(value: i64) -> IntExpr {
        IntExpr::Const { value }
    }

    fn expr_field(field: &str) -> IntExpr {
        IntExpr::FieldRef {
            field: field.to_string(),
        }
    }

    fn expr_add(left: IntExpr, right: IntExpr) -> IntExpr {
        IntExpr::Binary {
            op: IntExprOp::Add,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn expr_sub(left: IntExpr, right: IntExpr) -> IntExpr {
        IntExpr::Binary {
            op: IntExprOp::Sub,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    fn assert_uint(value: FieldValue, expected: u64) {
        match value {
            FieldValue::UInt(v) => assert_eq!(v, expected),
            other => panic!("expected UInt, got {other:?}"),
        }
    }

    fn assert_int(value: FieldValue, expected: i64) {
        match value {
            FieldValue::Int(v) => assert_eq!(v, expected),
            other => panic!("expected Int, got {other:?}"),
        }
    }

    fn assert_float(value: FieldValue, expected: f32) {
        match value {
            FieldValue::Float(v) => assert_eq!(v, expected),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn interpret_schema_collects_success_and_errors() {
        let buffer = MemoryBuffer::from_vec(vec![0xDE, 0xAD, 0xBE]);

        let schema = Schema {
            schema_name: "mixed".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![
                make_field("byte", FieldType::U8, 0, None),
                make_field("u32_fail", FieldType::U32, 1, None),
            ],
        };

        let results = interpret_schema(&buffer, &schema);

        assert_eq!(results.len(), 2);

        let first = &results[0];
        assert_eq!(first.field.name, "byte");
        assert_eq!(first.display_name, "byte");
        assert_eq!(first.resolved_offset, 0);
        assert!(first.offset_valid);
        assert_eq!(first.byte_len, 1);
        assert!(first.error.is_none());
        assert_uint(first.value.clone().unwrap(), 0xDE);

        let second = &results[1];
        assert_eq!(second.field.name, "u32_fail");
        assert_eq!(second.display_name, "u32_fail");
        assert_eq!(second.resolved_offset, 1);
        assert!(second.offset_valid);
        assert_eq!(second.byte_len, 4);
        assert!(second.value.is_none());
        let err = second.error.clone().expect("expected an error");
        assert!(err.contains("buffer error"));
    }

    #[test]
    fn interprets_with_schema_endianness() {
        let buffer = MemoryBuffer::from_vec(vec![
            0xAA, // U8
            0x01, 0x23, // U16
            0x45, 0x67, 0x89, 0xAB, // U32
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // U64
            0xFF, 0xFE, 0xFD, 0xFC, // I32
            0x3F, 0x80, 0x00, 0x00, // F32
        ]);

        let schema = Schema {
            schema_name: "test".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Big),
            fields: vec![],
        };

        let u8_value = interpret_field(
            &buffer,
            &make_field("u8", FieldType::U8, 0, None),
            Some(&schema),
        )
        .unwrap();
        let u16_value = interpret_field(
            &buffer,
            &make_field("u16", FieldType::U16, 1, None),
            Some(&schema),
        )
        .unwrap();
        let u32_value = interpret_field(
            &buffer,
            &make_field("u32", FieldType::U32, 3, None),
            Some(&schema),
        )
        .unwrap();
        let u64_value = interpret_field(
            &buffer,
            &make_field("u64", FieldType::U64, 7, None),
            Some(&schema),
        )
        .unwrap();
        let i32_value = interpret_field(
            &buffer,
            &make_field("i32", FieldType::I32, 15, None),
            Some(&schema),
        )
        .unwrap();
        let f32_value = interpret_field(
            &buffer,
            &make_field("f32", FieldType::F32, 19, None),
            Some(&schema),
        )
        .unwrap();

        assert_uint(u8_value, 0xAA);
        assert_uint(u16_value, 0x0123);
        assert_uint(u32_value, 0x456789AB);
        assert_uint(u64_value, 0x0102_0304_0506_0708);
        assert_int(
            i32_value,
            i32::from_be_bytes([0xFF, 0xFE, 0xFD, 0xFC]) as i64,
        );
        assert_float(f32_value, f32::from_be_bytes([0x3F, 0x80, 0x00, 0x00]));
    }

    #[test]
    fn interprets_with_field_endianness_only() {
        let buffer = MemoryBuffer::from_vec(vec![
            0xAA, // U8
            0x01, 0x23, // U16
            0x45, 0x67, 0x89, 0xAB, // U32
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, // U64
            0xFF, 0xFE, 0xFD, 0xFC, // I32
            0x3F, 0x80, 0x00, 0x00, // F32
        ]);

        let endianness = Some(Endianness::Big);
        let u8_value = interpret_field(
            &buffer,
            &make_field("u8", FieldType::U8, 0, endianness),
            None,
        )
        .unwrap();
        let u16_value = interpret_field(
            &buffer,
            &make_field("u16", FieldType::U16, 1, endianness),
            None,
        )
        .unwrap();
        let u32_value = interpret_field(
            &buffer,
            &make_field("u32", FieldType::U32, 3, endianness),
            None,
        )
        .unwrap();
        let u64_value = interpret_field(
            &buffer,
            &make_field("u64", FieldType::U64, 7, endianness),
            None,
        )
        .unwrap();
        let i32_value = interpret_field(
            &buffer,
            &make_field("i32", FieldType::I32, 15, endianness),
            None,
        )
        .unwrap();
        let f32_value = interpret_field(
            &buffer,
            &make_field("f32", FieldType::F32, 19, endianness),
            None,
        )
        .unwrap();

        assert_uint(u8_value, 0xAA);
        assert_uint(u16_value, 0x0123);
        assert_uint(u32_value, 0x456789AB);
        assert_uint(u64_value, 0x0102_0304_0506_0708);
        assert_int(
            i32_value,
            i32::from_be_bytes([0xFF, 0xFE, 0xFD, 0xFC]) as i64,
        );
        assert_float(f32_value, f32::from_be_bytes([0x3F, 0x80, 0x00, 0x00]));
    }

    #[test]
    fn field_endianness_overrides_schema_endianness() {
        let buffer = MemoryBuffer::from_vec(vec![0x01, 0x02]);

        let schema = Schema {
            schema_name: "precedence".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Big),
            fields: vec![],
        };

        let field = make_field("u16", FieldType::U16, 0, Some(Endianness::Little));

        let value = interpret_field(&buffer, &field, Some(&schema)).unwrap();

        assert_uint(value, 0x0201);
    }

    #[test]
    fn interprets_with_default_little_endian() {
        let buffer = MemoryBuffer::from_vec(vec![
            0x10, // U8
            0x22, 0x11, // U16
            0x66, 0x55, 0x44, 0x33, // U32
            0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11, // U64
            0xEB, 0x32, 0xA4, 0xF8, // I32 (-123456789)
            0x00, 0x00, 0x50, 0x40, // F32 (3.25)
        ]);

        let u8_value =
            interpret_field(&buffer, &make_field("u8", FieldType::U8, 0, None), None).unwrap();
        let u16_value =
            interpret_field(&buffer, &make_field("u16", FieldType::U16, 1, None), None).unwrap();
        let u32_value =
            interpret_field(&buffer, &make_field("u32", FieldType::U32, 3, None), None).unwrap();
        let u64_value =
            interpret_field(&buffer, &make_field("u64", FieldType::U64, 7, None), None).unwrap();
        let i32_value =
            interpret_field(&buffer, &make_field("i32", FieldType::I32, 15, None), None).unwrap();
        let f32_value =
            interpret_field(&buffer, &make_field("f32", FieldType::F32, 19, None), None).unwrap();

        assert_uint(u8_value, 0x10);
        assert_uint(u16_value, 0x1122);
        assert_uint(u32_value, 0x33445566);
        assert_uint(u64_value, 0x1122_3344_5566_7788);
        assert_int(
            i32_value,
            i32::from_le_bytes([0xEB, 0x32, 0xA4, 0xF8]) as i64,
        );
        assert_float(f32_value, f32::from_le_bytes([0x00, 0x00, 0x50, 0x40]));
    }

    #[test]
    fn interpret_field_supports_const_only_expression_offset() {
        let buffer = MemoryBuffer::from_vec(vec![0x00, 0x01, 0x02, 0x03]);
        let field = FieldDef {
            name: "expr".to_string(),
            ty: FieldType::U8,
            offset: OffsetKind::Expr(expr_add(expr_const(1), expr_const(1))),
            length: None,
            endianness: None,
            description: None,
            repeat: None,
        };

        let value = interpret_field(&buffer, &field, None).unwrap();
        assert_uint(value, 0x02);
    }

    #[test]
    fn interpret_field_rejects_expression_field_refs_without_context() {
        let buffer = MemoryBuffer::from_vec(vec![0x00, 0x01, 0x02, 0x03]);
        let field = FieldDef {
            name: "expr".to_string(),
            ty: FieldType::U8,
            offset: OffsetKind::Expr(expr_field("base")),
            length: None,
            endianness: None,
            description: None,
            repeat: None,
        };

        let err = interpret_field(&buffer, &field, None).unwrap_err();
        assert_eq!(err.to_string(), "missing expression reference `base`");
    }

    #[test]
    fn repeated_numeric_fields_expand_with_incremented_offsets() {
        let buffer = MemoryBuffer::from_vec(vec![0x10, 0x20, 0x30]);
        let schema = Schema {
            schema_name: "repeat".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "byte".to_string(),
                ty: FieldType::U8,
                offset: OffsetKind::Absolute(0),
                length: None,
                endianness: None,
                description: None,
                repeat: Some(binocular_schema::ast::RepeatInfo { count: 3 }),
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 3);

        assert_eq!(results[0].display_name, "byte[0]");
        assert_eq!(results[0].resolved_offset, 0);
        assert!(results[0].offset_valid);
        assert_eq!(results[0].byte_len, 1);
        assert_uint(results[0].value.clone().unwrap(), 0x10);

        assert_eq!(results[1].display_name, "byte[1]");
        assert_eq!(results[1].resolved_offset, 1);
        assert!(results[1].offset_valid);
        assert_eq!(results[1].byte_len, 1);
        assert_uint(results[1].value.clone().unwrap(), 0x20);

        assert_eq!(results[2].display_name, "byte[2]");
        assert_eq!(results[2].resolved_offset, 2);
        assert!(results[2].offset_valid);
        assert_eq!(results[2].byte_len, 1);
        assert_uint(results[2].value.clone().unwrap(), 0x30);
    }

    #[test]
    fn repeated_ascii_and_bytes_use_declared_length_as_stride() {
        let buffer = MemoryBuffer::from_vec(b"ABCDWXYZ".to_vec());
        let schema = Schema {
            schema_name: "repeat".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "chunk".to_string(),
                ty: FieldType::Ascii,
                offset: OffsetKind::Absolute(0),
                length: Some(LengthSpec::Literal(4)),
                endianness: None,
                description: None,
                repeat: Some(binocular_schema::ast::RepeatInfo { count: 2 }),
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].resolved_offset, 0);
        assert!(results[0].offset_valid);
        assert_eq!(results[0].byte_len, 4);
        assert_eq!(results[1].resolved_offset, 4);
        assert!(results[1].offset_valid);
        assert_eq!(results[1].byte_len, 4);

        match results[0].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "ABCD"),
            other => panic!("expected Ascii, got {other:?}"),
        }
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "WXYZ"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn repeated_field_partial_failure_only_marks_failing_rows() {
        let buffer = MemoryBuffer::from_vec(vec![0xAA, 0xBB]);
        let schema = Schema {
            schema_name: "repeat".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "pair".to_string(),
                ty: FieldType::U8,
                offset: OffsetKind::Absolute(1),
                length: None,
                endianness: None,
                description: None,
                repeat: Some(binocular_schema::ast::RepeatInfo { count: 3 }),
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 3);

        assert!(results[0].error.is_none());
        assert_uint(results[0].value.clone().unwrap(), 0xBB);

        assert!(results[1].value.is_none());
        assert!(results[1]
            .error
            .as_ref()
            .expect("expected error")
            .contains("buffer error"));

        assert!(results[2].value.is_none());
        assert!(results[2]
            .error
            .as_ref()
            .expect("expected error")
            .contains("buffer error"));
    }

    #[test]
    fn repeated_field_overflow_only_marks_overflowing_row() {
        let buffer = MemoryBuffer::from_vec(vec![0xAB]);
        let schema = Schema {
            schema_name: "repeat".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "overflow".to_string(),
                ty: FieldType::U16,
                offset: OffsetKind::Absolute(u64::MAX - 1),
                length: None,
                endianness: None,
                description: None,
                repeat: Some(binocular_schema::ast::RepeatInfo { count: 2 }),
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 2);

        assert_eq!(results[0].display_name, "overflow[0]");
        assert_eq!(results[0].resolved_offset, u64::MAX - 1);
        assert!(results[0].offset_valid);
        assert!(results[0].value.is_none());
        assert!(results[0]
            .error
            .as_ref()
            .expect("expected error")
            .contains("buffer error"));

        assert_eq!(results[1].display_name, "overflow[1]");
        assert_eq!(results[1].resolved_offset, 0);
        assert!(!results[1].offset_valid);
        assert!(results[1].value.is_none());
        assert_eq!(
            results[1].error.as_deref(),
            Some("resolved offset overflowed during repeat expansion")
        );
    }

    #[test]
    fn dynamic_length_resolves_from_prior_unsigned_field() {
        let buffer = MemoryBuffer::from_vec(vec![3, 0, b'C', b'A', b'T']);
        let schema = Schema {
            schema_name: "dynamic".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("block_len", FieldType::U16, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::FieldRef {
                        field: "block_len".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 2);
        assert_eq!(results[1].byte_len, 3);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "CAT"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_length_resolves_from_non_negative_i32_field() {
        let buffer = MemoryBuffer::from_vec(vec![4, 0, 0, 0, b'T', b'E', b'S', b'T']);
        let schema = Schema {
            schema_name: "dynamic_i32".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("block_len", FieldType::I32, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    4,
                    LengthSpec::FieldRef {
                        field: "block_len".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[1].byte_len, 4);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "TEST"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn expression_length_resolves_from_previous_field_minus_constant() {
        let buffer = MemoryBuffer::from_vec(vec![7, 0, b'C', b'A', b'T']);
        let schema = Schema {
            schema_name: "expr_length".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("block_len", FieldType::U16, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::Expr {
                        expr: expr_sub(expr_field("block_len"), expr_const(4)),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[1].byte_len, 3);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "CAT"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_length_can_reference_repeated_numeric_display_name() {
        let buffer = MemoryBuffer::from_vec(vec![2, 0, 3, 0, b'H', b'E', b'Y']);
        let schema = Schema {
            schema_name: "dynamic_repeat_source".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                FieldDef {
                    name: "sizes".to_string(),
                    ty: FieldType::U16,
                    offset: OffsetKind::Absolute(0),
                    length: None,
                    endianness: None,
                    description: None,
                    repeat: Some(binocular_schema::ast::RepeatInfo { count: 2 }),
                },
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    4,
                    LengthSpec::FieldRef {
                        field: "sizes[1]".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[2].byte_len, 3);
        match results[2].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "HEY"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_length_missing_reference_becomes_row_error() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K']);
        let schema = Schema {
            schema_name: "missing_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![make_sized_field(
                "payload",
                FieldType::Ascii,
                0,
                LengthSpec::FieldRef {
                    field: "missing".to_string(),
                },
            )],
        };

        let results = interpret_schema(&buffer, &schema);
        assert!(results[0].value.is_none());
        assert_eq!(results[0].byte_len, 0);
        assert_eq!(
            results[0].error.as_deref(),
            Some("missing dynamic length reference `missing`")
        );
    }

    #[test]
    fn expression_length_missing_reference_becomes_row_error() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K']);
        let schema = Schema {
            schema_name: "missing_expr_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![make_sized_field(
                "payload",
                FieldType::Ascii,
                0,
                LengthSpec::Expr {
                    expr: expr_sub(expr_field("missing"), expr_const(1)),
                },
            )],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[0].error.as_deref(),
            Some("missing expression reference `missing`")
        );
    }

    #[test]
    fn dynamic_length_rejects_non_numeric_reference_source() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K', b'A', b'Y']);
        let schema = Schema {
            schema_name: "non_numeric_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::FieldRef {
                        field: "label".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as a dynamic length source")
        );
    }

    #[test]
    fn expression_length_rejects_non_numeric_reference_source() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K', b'A', b'Y']);
        let schema = Schema {
            schema_name: "expr_non_numeric_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::Expr {
                        expr: expr_add(expr_field("label"), expr_const(1)),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as an expression source")
        );
    }

    #[test]
    fn dynamic_length_rejects_float_reference_source() {
        let buffer = MemoryBuffer::from_vec(vec![0, 0, 128, 63, b'O']);
        let schema = Schema {
            schema_name: "float_ref".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("len", FieldType::F32, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    4,
                    LengthSpec::FieldRef {
                        field: "len".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `len` cannot be used as a dynamic length source")
        );
    }

    #[test]
    fn dynamic_length_rejects_negative_i32_reference() {
        let buffer = MemoryBuffer::from_vec(vec![255, 255, 255, 255, b'O']);
        let schema = Schema {
            schema_name: "negative_ref".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("len", FieldType::I32, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    4,
                    LengthSpec::FieldRef {
                        field: "len".to_string(),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `len` resolved to a negative dynamic length")
        );
    }

    #[test]
    fn expression_length_rejects_negative_result() {
        let buffer = MemoryBuffer::from_vec(vec![3, 0]);
        let schema = Schema {
            schema_name: "expr_negative_length".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("len", FieldType::U16, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::Expr {
                        expr: expr_sub(expr_field("len"), expr_const(4)),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("expression resolved to a negative dynamic length")
        );
    }

    #[test]
    fn expression_length_rejects_zero_result() {
        let buffer = MemoryBuffer::from_vec(vec![4, 0]);
        let schema = Schema {
            schema_name: "expr_zero_length".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("len", FieldType::U16, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Ascii,
                    2,
                    LengthSpec::Expr {
                        expr: expr_sub(expr_field("len"), expr_const(4)),
                    },
                ),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("expression resolved to a zero dynamic length")
        );
    }

    #[test]
    fn expression_length_rejects_arithmetic_overflow() {
        let schema = Schema {
            schema_name: "expr_overflow".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![make_sized_field(
                "payload",
                FieldType::Ascii,
                0,
                LengthSpec::Expr {
                    expr: expr_add(expr_const(i64::MAX), expr_const(1)),
                },
            )],
        };

        let results = interpret_schema(&MemoryBuffer::from_vec(vec![b'O']), &schema);
        assert_eq!(
            results[0].error.as_deref(),
            Some("expression arithmetic overflowed")
        );
    }

    #[test]
    fn dynamic_length_failure_does_not_block_later_fields() {
        let buffer = MemoryBuffer::from_vec(vec![b'A', b'B', 0x34, 0x12]);
        let schema = Schema {
            schema_name: "continue_after_error".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                make_sized_field(
                    "payload",
                    FieldType::Bytes,
                    0,
                    LengthSpec::FieldRef {
                        field: "label".to_string(),
                    },
                ),
                make_field("tail", FieldType::U16, 2, None),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as a dynamic length source")
        );
        assert_uint(results[2].value.clone().unwrap(), 0x1234);
    }

    #[test]
    fn expression_length_failure_does_not_block_later_fields() {
        let buffer = MemoryBuffer::from_vec(vec![2, 0, 0x34, 0x12]);
        let schema = Schema {
            schema_name: "continue_after_expr_error".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("len", FieldType::U16, 0, None),
                make_sized_field(
                    "payload",
                    FieldType::Bytes,
                    0,
                    LengthSpec::Expr {
                        expr: expr_sub(expr_field("len"), expr_const(4)),
                    },
                ),
                make_field("tail", FieldType::U16, 2, None),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("expression resolved to a negative dynamic length")
        );
        assert_uint(results[2].value.clone().unwrap(), 0x1234);
    }

    #[test]
    fn dynamic_offset_resolves_from_prior_unsigned_field() {
        let buffer = MemoryBuffer::from_vec(vec![4, 0, 0, 0, b'J', b'U', b'M', b'P']);
        let schema = Schema {
            schema_name: "dynamic_offset".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("data_offset", FieldType::U32, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::FieldRef("data_offset".to_string()),
                    length: Some(LengthSpec::Literal(4)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[1].resolved_offset, 4);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "JUMP"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_offset_resolves_from_non_negative_i32_field() {
        let buffer = MemoryBuffer::from_vec(vec![4, 0, 0, 0, b'D', b'A', b'T', b'A']);
        let schema = Schema {
            schema_name: "dynamic_offset_i32".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("data_offset", FieldType::I32, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::FieldRef("data_offset".to_string()),
                    length: Some(LengthSpec::Literal(4)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[1].resolved_offset, 4);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "DATA"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn expression_offset_resolves_from_previous_field_plus_constant() {
        let buffer = MemoryBuffer::from_vec(vec![4, 0, 0, 0, 0xAA, 0xBB, b'D', b'A', b'T', b'A']);
        let schema = Schema {
            schema_name: "expr_offset".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("data_offset", FieldType::U32, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::Expr(expr_add(expr_field("data_offset"), expr_const(2))),
                    length: Some(LengthSpec::Literal(4)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[1].resolved_offset, 6);
        match results[1].value.as_ref().unwrap() {
            FieldValue::Ascii(value) => assert_eq!(value, "DATA"),
            other => panic!("expected Ascii, got {other:?}"),
        }
    }

    #[test]
    fn dynamic_offset_missing_reference_becomes_row_error() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K']);
        let schema = Schema {
            schema_name: "missing_offset_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: FieldType::Ascii,
                offset: OffsetKind::FieldRef("missing".to_string()),
                length: Some(LengthSpec::Literal(2)),
                endianness: None,
                description: None,
                repeat: None,
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert!(results[0].value.is_none());
        assert_eq!(results[0].byte_len, 0);
        assert_eq!(results[0].resolved_offset, 0);
        assert!(!results[0].offset_valid);
        assert_eq!(
            results[0].error.as_deref(),
            Some("missing dynamic offset reference `missing`")
        );
    }

    #[test]
    fn expression_offset_missing_reference_becomes_row_error() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K']);
        let schema = Schema {
            schema_name: "missing_expr_offset_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: FieldType::Ascii,
                offset: OffsetKind::Expr(expr_add(expr_field("missing"), expr_const(1))),
                length: Some(LengthSpec::Literal(2)),
                endianness: None,
                description: None,
                repeat: None,
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[0].resolved_offset, 0);
        assert!(!results[0].offset_valid);
        assert_eq!(
            results[0].error.as_deref(),
            Some("missing expression reference `missing`")
        );
    }

    #[test]
    fn dynamic_offset_rejects_non_numeric_reference_source() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K', b'A', b'Y']);
        let schema = Schema {
            schema_name: "non_numeric_offset_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::FieldRef("label".to_string()),
                    length: Some(LengthSpec::Literal(2)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as a dynamic offset source")
        );
        assert!(!results[1].offset_valid);
    }

    #[test]
    fn expression_offset_rejects_non_numeric_reference_source() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K', b'A', b'Y']);
        let schema = Schema {
            schema_name: "expr_non_numeric_offset_ref".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::Expr(expr_add(expr_field("label"), expr_const(1))),
                    length: Some(LengthSpec::Literal(2)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as an expression source")
        );
        assert!(!results[1].offset_valid);
    }

    #[test]
    fn dynamic_offset_rejects_negative_i32_reference() {
        let buffer = MemoryBuffer::from_vec(vec![255, 255, 255, 255, b'O']);
        let schema = Schema {
            schema_name: "negative_offset_ref".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("data_offset", FieldType::I32, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::FieldRef("data_offset".to_string()),
                    length: Some(LengthSpec::Literal(1)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `data_offset` resolved to a negative dynamic offset")
        );
        assert!(!results[1].offset_valid);
    }

    #[test]
    fn expression_offset_rejects_negative_result() {
        let buffer = MemoryBuffer::from_vec(vec![2, 0, 0, 0, b'O']);
        let schema = Schema {
            schema_name: "negative_expr_offset".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("data_offset", FieldType::I32, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Ascii,
                    offset: OffsetKind::Expr(expr_sub(expr_field("data_offset"), expr_const(4))),
                    length: Some(LengthSpec::Literal(1)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("expression resolved to a negative dynamic offset")
        );
        assert!(!results[1].offset_valid);
    }

    #[test]
    fn expression_offset_rejects_arithmetic_overflow() {
        let schema = Schema {
            schema_name: "expr_offset_overflow".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: FieldType::Ascii,
                offset: OffsetKind::Expr(expr_add(expr_const(i64::MAX), expr_const(1))),
                length: Some(LengthSpec::Literal(1)),
                endianness: None,
                description: None,
                repeat: None,
            }],
        };

        let results = interpret_schema(&MemoryBuffer::from_vec(vec![b'O']), &schema);
        assert_eq!(
            results[0].error.as_deref(),
            Some("expression arithmetic overflowed")
        );
        assert!(!results[0].offset_valid);
    }

    #[test]
    fn dynamic_offset_failure_does_not_block_later_fields() {
        let buffer = MemoryBuffer::from_vec(vec![b'A', b'B', 0x34, 0x12]);
        let schema = Schema {
            schema_name: "continue_after_offset_error".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_sized_field("label", FieldType::Ascii, 0, LengthSpec::Literal(2)),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Bytes,
                    offset: OffsetKind::FieldRef("label".to_string()),
                    length: Some(LengthSpec::Literal(1)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
                make_field("tail", FieldType::U16, 2, None),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("field `label` cannot be used as a dynamic offset source")
        );
        assert!(!results[1].offset_valid);
        assert_uint(results[2].value.clone().unwrap(), 0x1234);
    }

    #[test]
    fn expression_offset_failure_does_not_block_later_fields() {
        let buffer = MemoryBuffer::from_vec(vec![2, 0, 0x34, 0x12]);
        let schema = Schema {
            schema_name: "continue_after_expr_offset_error".to_string(),
            schema_version: 1,
            endianness: Some(Endianness::Little),
            fields: vec![
                make_field("offset_base", FieldType::U16, 0, None),
                FieldDef {
                    name: "payload".to_string(),
                    ty: FieldType::Bytes,
                    offset: OffsetKind::Expr(expr_sub(expr_field("offset_base"), expr_const(4))),
                    length: Some(LengthSpec::Literal(1)),
                    endianness: None,
                    description: None,
                    repeat: None,
                },
                make_field("tail", FieldType::U16, 2, None),
            ],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(
            results[1].error.as_deref(),
            Some("expression resolved to a negative dynamic offset")
        );
        assert!(!results[1].offset_valid);
        assert_uint(results[2].value.clone().unwrap(), 0x1234);
    }

    #[test]
    fn successful_offset_zero_stays_valid() {
        let buffer = MemoryBuffer::from_vec(vec![b'O', b'K']);
        let schema = Schema {
            schema_name: "offset_zero".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: FieldType::Ascii,
                offset: OffsetKind::Absolute(0),
                length: Some(LengthSpec::Literal(2)),
                endianness: None,
                description: None,
                repeat: None,
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[0].resolved_offset, 0);
        assert!(results[0].offset_valid);
        assert!(results[0].error.is_none());
    }

    #[test]
    fn buffer_failures_after_resolving_offset_keep_offset_valid() {
        let buffer = MemoryBuffer::from_vec(vec![0xAA]);
        let schema = Schema {
            schema_name: "resolved_offset_buffer_error".to_string(),
            schema_version: 1,
            endianness: None,
            fields: vec![FieldDef {
                name: "payload".to_string(),
                ty: FieldType::U16,
                offset: OffsetKind::Absolute(0),
                length: None,
                endianness: None,
                description: None,
                repeat: None,
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results[0].resolved_offset, 0);
        assert!(results[0].offset_valid);
        assert!(results[0]
            .error
            .as_deref()
            .is_some_and(|err| err.contains("buffer error")));
    }
}
