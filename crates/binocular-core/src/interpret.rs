use binocular_schema::ast::{Endianness, FieldDef, FieldType, OffsetKind, Schema};
use crate::buffer::FileBuffer;
use crate::error::InterpretError;

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
