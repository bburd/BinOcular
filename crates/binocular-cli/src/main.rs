use std::fs;
use std::path::PathBuf;
use std::process;

use binocular_core::buffer::MemoryBuffer;
use binocular_core::interpret::{interpret_field, FieldValue};
use binocular_schema::ast::{FieldDef, FieldType, OffsetKind};
use binocular_schema::parser::parse_schema_str;
use clap::Parser;

#[derive(Parser)]
#[command(name = "binocular-cli")]
#[command(about = "CLI companion for BinOcular", long_about = None)]
struct Cli {
    /// Path to the binary file to inspect.
    file: PathBuf,

    /// Path to the YAML schema describing the file layout.
    #[arg(short, long)]
    schema: PathBuf,

    /// Output as JSON instead of a human-readable table.
    #[arg(long)]
    json: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let file_bytes = fs::read(&cli.file)?;
    let schema_str = fs::read_to_string(&cli.schema)?;
    let schema = match parse_schema_str(&schema_str) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to parse schema: {err}");
            process::exit(1);
        }
    };

    let buffer = MemoryBuffer::from_vec(file_bytes);

    let mut records = Vec::new();
    for field in &schema.fields {
        let offset = match &field.offset {
            OffsetKind::Absolute(o) => Some(*o),
            OffsetKind::Expr(_) => None,
        };

        let result = interpret_field(&buffer, field, Some(&schema));
        let (value, error) = match result {
            Ok(value) => (Some(render_field_value(&value)), None),
            Err(err) => (None, Some(err.to_string())),
        };

        records.push(FieldRecord {
            name: field.name.clone(),
            offset,
            offset_hex: offset.map(|o| format!("0x{o:08X}")),
            field_type: render_field_type(field),
            value,
            error,
        });
    }

    if cli.json {
        use serde_json::json;

        let json_records: Vec<_> = records
            .into_iter()
            .map(|record| {
                json!({
                    "name": record.name,
                    "offset": record.offset,
                    "offset_hex": record.offset_hex,
                    "type": record.field_type,
                    "value": record.value,
                    "error": record.error,
                })
            })
            .collect();

        println!("{}", serde_json::Value::Array(json_records).to_string());
    } else {
        println!("NAME|OFFSET|TYPE|VALUE|ERROR");
        for record in records {
            let offset = render_offset(record.offset);
            let value = record.value.unwrap_or_else(|| "-".to_string());
            let error = record.error.unwrap_or_else(|| "-".to_string());

            println!(
                "{}|{}|{}|{}|{}",
                record.name, offset, record.field_type, value, error
            );
        }
    }

    Ok(())
}

#[derive(Debug)]
struct FieldRecord {
    name: String,
    offset: Option<u64>,
    offset_hex: Option<String>,
    field_type: String,
    value: Option<String>,
    error: Option<String>,
}

fn render_offset(offset: Option<u64>) -> String {
    match offset {
        Some(value) => format!("{} (0x{value:08X})", value),
        None => "-".to_string(),
    }
}

fn render_field_type(field: &FieldDef) -> String {
    match field.ty {
        FieldType::U8 => "u8".to_string(),
        FieldType::U16 => "u16".to_string(),
        FieldType::U32 => "u32".to_string(),
        FieldType::U64 => "u64".to_string(),
        FieldType::I32 => "i32".to_string(),
        FieldType::F32 => "f32".to_string(),
        FieldType::Bytes => match field.length {
            Some(length) => format!("bytes[{length}]"),
            None => "bytes".to_string(),
        },
        FieldType::Ascii => match field.length {
            Some(length) => format!("ascii[{length}]"),
            None => "ascii".to_string(),
        },
    }
}

fn render_field_value(value: &FieldValue) -> String {
    match value {
        FieldValue::UInt(v) => format!("{} (0x{:X})", v, v),
        FieldValue::Int(v) => format!("{} (0x{:X})", v, v),
        FieldValue::Float(v) => format!("{}", v),
        FieldValue::Bytes(bytes) => render_bytes(bytes),
        FieldValue::Ascii(text) => render_ascii(text),
    }
}

fn render_bytes(bytes: &[u8]) -> String {
    const MAX_BYTES: usize = 16;

    let display_len = bytes.len().min(MAX_BYTES);
    let mut rendered: Vec<String> = bytes[..display_len]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();

    if bytes.len() > MAX_BYTES {
        rendered.push("…".to_string());
    }

    let rendered = rendered.join(" ");

    if bytes.len() > MAX_BYTES {
        format!("{rendered} (len={})", bytes.len())
    } else {
        rendered
    }
}

fn render_ascii(text: &str) -> String {
    const MAX_CHARS: usize = 32;
    let mut escaped = text
        .chars()
        .take(MAX_CHARS)
        .flat_map(|c| c.escape_default())
        .collect::<String>();

    if text.chars().count() > MAX_CHARS {
        escaped.push('…');
    }

    format!("\"{}\"", escaped)
}
