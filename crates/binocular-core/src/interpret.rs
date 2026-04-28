use crate::buffer::FileBuffer;
use crate::error::InterpretError;
use binocular_schema::ast::{Endianness, FieldDef, FieldType, OffsetKind, Schema};

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
    pub byte_len: usize,
    pub value: Option<FieldValue>,
    pub error: Option<String>,
}

pub fn interpret_field<B: FileBuffer + ?Sized>(
    buffer: &B,
    field: &FieldDef,
    schema: Option<&Schema>,
) -> Result<FieldValue, InterpretError> {
    let len = field_byte_len(field);
    let offset = resolve_base_offset(field)?;
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

    for field in &schema.fields {
        let byte_len = field_byte_len(field);
        let count = field.repeat.as_ref().map_or(1, |repeat| repeat.count);

        for index in 0..count {
            let display_name = if field.repeat.is_some() {
                format!("{}[{index}]", field.name)
            } else {
                field.name.clone()
            };

            let resolved = resolve_repeated_offset(field, index, byte_len);
            let resolved_offset = match resolved {
                Ok(offset) => offset,
                Err(err) => {
                    rows.push(FieldEval {
                        field: field.clone(),
                        display_name,
                        resolved_offset: 0,
                        byte_len,
                        value: None,
                        error: Some(err.to_string()),
                    });
                    continue;
                }
            };

            let value = interpret_field_at(buffer, field, Some(schema), resolved_offset, byte_len);
            match value {
                Ok(value) => rows.push(FieldEval {
                    field: field.clone(),
                    display_name,
                    resolved_offset,
                    byte_len,
                    value: Some(value),
                    error: None,
                }),
                Err(err) => rows.push(FieldEval {
                    field: field.clone(),
                    display_name,
                    resolved_offset,
                    byte_len,
                    value: None,
                    error: Some(err.to_string()),
                }),
            }
        }
    }

    rows
}

fn field_byte_len(field: &FieldDef) -> usize {
    match field.ty {
        FieldType::U8 => 1,
        FieldType::U16 => 2,
        FieldType::U32 | FieldType::I32 | FieldType::F32 => 4,
        FieldType::U64 => 8,
        FieldType::Bytes | FieldType::Ascii => field.length.unwrap_or(0) as usize,
    }
}

fn resolve_base_offset(field: &FieldDef) -> Result<u64, InterpretError> {
    match &field.offset {
        OffsetKind::Absolute(offset) => Ok(*offset),
        OffsetKind::Expr(_) => Err(InterpretError::Unsupported),
    }
}

fn resolve_repeated_offset(
    field: &FieldDef,
    index: u64,
    byte_len: usize,
) -> Result<u64, InterpretError> {
    let base_offset = resolve_base_offset(field)?;
    let stride = u64::try_from(byte_len).map_err(|_| InterpretError::OffsetOverflow)?;
    let repeated_bytes = index
        .checked_mul(stride)
        .ok_or(InterpretError::OffsetOverflow)?;

    base_offset
        .checked_add(repeated_bytes)
        .ok_or(InterpretError::OffsetOverflow)
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
    use binocular_schema::ast::{Endianness, FieldDef, FieldType, OffsetKind, Schema};

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
        assert_eq!(first.byte_len, 1);
        assert!(first.error.is_none());
        assert_uint(first.value.clone().unwrap(), 0xDE);

        let second = &results[1];
        assert_eq!(second.field.name, "u32_fail");
        assert_eq!(second.display_name, "u32_fail");
        assert_eq!(second.resolved_offset, 1);
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
    fn expr_offset_is_unsupported() {
        let buffer = MemoryBuffer::from_vec(vec![0x00, 0x01, 0x02, 0x03]);
        let field = FieldDef {
            name: "expr".to_string(),
            ty: FieldType::U8,
            offset: OffsetKind::Expr("1 + 1".to_string()),
            length: None,
            endianness: None,
            description: None,
            repeat: None,
        };

        let err = interpret_field(&buffer, &field, None).unwrap_err();
        assert!(matches!(err, InterpretError::Unsupported));
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
        assert_eq!(results[0].byte_len, 1);
        assert_uint(results[0].value.clone().unwrap(), 0x10);

        assert_eq!(results[1].display_name, "byte[1]");
        assert_eq!(results[1].resolved_offset, 1);
        assert_eq!(results[1].byte_len, 1);
        assert_uint(results[1].value.clone().unwrap(), 0x20);

        assert_eq!(results[2].display_name, "byte[2]");
        assert_eq!(results[2].resolved_offset, 2);
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
                length: Some(4),
                endianness: None,
                description: None,
                repeat: Some(binocular_schema::ast::RepeatInfo { count: 2 }),
            }],
        };

        let results = interpret_schema(&buffer, &schema);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].resolved_offset, 0);
        assert_eq!(results[0].byte_len, 4);
        assert_eq!(results[1].resolved_offset, 4);
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
        assert!(results[0].value.is_none());
        assert!(results[0]
            .error
            .as_ref()
            .expect("expected error")
            .contains("buffer error"));

        assert_eq!(results[1].display_name, "overflow[1]");
        assert_eq!(results[1].resolved_offset, 0);
        assert!(results[1].value.is_none());
        assert_eq!(
            results[1].error.as_deref(),
            Some("resolved offset overflowed during repeat expansion")
        );
    }
}
