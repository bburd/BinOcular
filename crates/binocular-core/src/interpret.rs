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

pub fn interpret_field<B: FileBuffer>(
    buffer: &B,
    field: &FieldDef,
    schema: Option<&Schema>,
) -> Result<FieldValue, InterpretError> {
    let len = match field.ty {
        FieldType::U8 => 1,
        FieldType::U16 => 2,
        FieldType::U32 | FieldType::I32 | FieldType::F32 => 4,
        FieldType::U64 => 8,
        FieldType::Bytes | FieldType::Ascii => field.length.unwrap_or(0) as usize,
    };

    let offset = match &field.offset {
        OffsetKind::Absolute(o) => *o,
        OffsetKind::Expr(_) => return Err(InterpretError::Unsupported),
    };

    let bytes = buffer.read_bytes(offset, len)?;

    let endianness = field
        .endianness
        .or_else(|| schema.and_then(|s| s.endianness))
        .unwrap_or(Endianness::Little);

    let value = match field.ty {
        FieldType::U8 => FieldValue::UInt(bytes[0] as u64),
        FieldType::U16 => {
            let v = read_u16(bytes, endianness);
            FieldValue::UInt(v as u64)
        }
        FieldType::U32 => {
            let v = read_u32(bytes, endianness);
            FieldValue::UInt(v as u64)
        }
        FieldType::U64 => {
            let v = read_u64(bytes, endianness);
            FieldValue::UInt(v)
        }
        FieldType::I32 => {
            let v = read_i32(bytes, endianness);
            FieldValue::Int(v as i64)
        }
        FieldType::F32 => {
            let v = read_f32(bytes, endianness);
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

fn read_u16(bytes: &[u8], endianness: Endianness) -> u16 {
    match endianness {
        Endianness::Little => u16::from_le_bytes(bytes.try_into().unwrap()),
        Endianness::Big => u16::from_be_bytes(bytes.try_into().unwrap()),
    }
}

fn read_u32(bytes: &[u8], endianness: Endianness) -> u32 {
    match endianness {
        Endianness::Little => u32::from_le_bytes(bytes.try_into().unwrap()),
        Endianness::Big => u32::from_be_bytes(bytes.try_into().unwrap()),
    }
}

fn read_u64(bytes: &[u8], endianness: Endianness) -> u64 {
    match endianness {
        Endianness::Little => u64::from_le_bytes(bytes.try_into().unwrap()),
        Endianness::Big => u64::from_be_bytes(bytes.try_into().unwrap()),
    }
}

fn read_i32(bytes: &[u8], endianness: Endianness) -> i32 {
    match endianness {
        Endianness::Little => i32::from_le_bytes(bytes.try_into().unwrap()),
        Endianness::Big => i32::from_be_bytes(bytes.try_into().unwrap()),
    }
}

fn read_f32(bytes: &[u8], endianness: Endianness) -> f32 {
    match endianness {
        Endianness::Little => f32::from_le_bytes(bytes.try_into().unwrap()),
        Endianness::Big => f32::from_be_bytes(bytes.try_into().unwrap()),
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
}
