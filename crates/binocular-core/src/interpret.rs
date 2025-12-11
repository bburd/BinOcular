use binocular_schema::ast::{FieldDef, FieldType, OffsetKind};
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

    let value = match field.ty {
        FieldType::U8 => FieldValue::UInt(bytes[0] as u64),
        FieldType::U16 => {
            let v = u16::from_be_bytes(bytes.try_into().unwrap());
            FieldValue::UInt(v as u64)
        }
        FieldType::U32 => {
            let v = u32::from_be_bytes(bytes.try_into().unwrap());
            FieldValue::UInt(v as u64)
        }
        FieldType::U64 => {
            let v = u64::from_be_bytes(bytes.try_into().unwrap());
            FieldValue::UInt(v)
        }
        FieldType::I32 => {
            let v = i32::from_be_bytes(bytes.try_into().unwrap());
            FieldValue::Int(v as i64)
        }
        FieldType::F32 => {
            let v = f32::from_be_bytes(bytes.try_into().unwrap());
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
