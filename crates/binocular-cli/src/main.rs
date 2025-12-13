//! BinOcular CLI entrypoint.
//!
//! # Output contract
//! The `--json` flag is meant for automation and must emit **only valid JSON to
//! stdout**. No banners, badges, version strings, or other branding are allowed
//! in this mode. CI and integration tests parse stdout as JSON directly, so any
//! additional text will cause failures.

use std::fs;
use std::path::PathBuf;
use std::process;

use anyhow::Context;
use binocular_core::buffer::MemoryBuffer;
use binocular_core::interpret::{interpret_schema, FieldValue};
use binocular_schema::ast::{FieldDef, FieldType, OffsetKind};
use binocular_schema::parser::parse_schema_str;
use clap::Parser;
use serde_json::json;

const BANNER: &str = include_str!("../assets/ascii/banner.txt");
const BADGE: &str = include_str!("../assets/ascii/badge.txt");
const TAGLINE: &str = "A schema-driven binary inspection toolkit for developers, reverse-engineers, and anyone who wants to stop guessing about byte layouts.";

#[derive(Parser)]
#[command(name = "binocular-cli")]
#[command(about = "CLI companion for BinOcular", long_about = None)]
struct Cli {
    /// Path to the binary file to inspect.
    #[arg(required_unless_present = "branding")]
    file: Option<PathBuf>,

    /// Path to the YAML schema describing the file layout.
    #[arg(short, long, required_unless_present = "branding")]
    schema: Option<PathBuf>,

    /// Output as JSON instead of a human-readable table.
    #[arg(long)]
    json: bool,

    /// Prepend branding banner and version before human-readable output.
    #[arg(long)]
    branding: bool,
}

fn print_banner() {
    print!("{BANNER}");
    println!("{TAGLINE}");
}

fn print_badge() {
    print!("{BADGE}");
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.branding && !cli.json && cli.file.is_none() && cli.schema.is_none() {
        print_banner();
        println!("v{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.json && cli.branding {
        eprintln!(
            "Branding output is disabled when --json is set; branding would break JSON stdout."
        );
        process::exit(2);
    }

    let file = cli
        .file
        .as_ref()
        .context("FILE is required unless --branding is set")?;
    let schema = cli
        .schema
        .as_ref()
        .context("--schema is required unless --branding is set")?;

    let file_bytes = fs::read(file)?;
    let schema_str = fs::read_to_string(schema)?;
    let schema = match parse_schema_str(&schema_str) {
        Ok(schema) => schema,
        Err(err) => {
            eprintln!("Failed to parse schema: {err}");
            process::exit(1);
        }
    };

    let buffer = MemoryBuffer::from_vec(file_bytes);

    let evaluations = interpret_schema(&buffer, &schema);
    let records: Vec<_> = evaluations
        .into_iter()
        .map(|eval| {
            let offset = match &eval.field.offset {
                OffsetKind::Absolute(o) => Some(*o),
                OffsetKind::Expr(_) => None,
            };

            FieldRecord {
                name: eval.field.name.clone(),
                offset,
                offset_hex: offset.map(|o| format!("0x{o:08X}")),
                field_type: render_field_type(&eval.field),
                value: eval.value,
                error: eval.error,
            }
        })
        .collect();

    // CI parses stdout as JSON directly in this branch; keep stdout free of
    // banners, version strings, or other non-JSON content.
    if cli.json {
        let json_records: Vec<_> = records
            .into_iter()
            .map(|record| {
                json!({
                    "name": record.name,
                    "offset": record.offset,
                    "offset_hex": record.offset_hex,
                    "type": record.field_type,
                    "value": render_json_value(record.value),
                    "error": record.error,
                })
            })
            .collect();

        println!("{}", serde_json::Value::Array(json_records));

        // Early return to avoid executing the branding branch below when
        // `--json` is set. Branding is meant for human-readable output only.
        return Ok(());
    } else {
        print_badge();

        if cli.branding {
            print_banner();
            println!("v{}", env!("CARGO_PKG_VERSION"));
        }

        println!("NAME|OFFSET|TYPE|VALUE|ERROR");
        for record in records {
            let offset = render_offset(record.offset);
            let value = render_table_value(record.value.as_ref());
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
    value: Option<FieldValue>,
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

fn render_table_value(value: Option<&FieldValue>) -> String {
    value
        .map(render_field_value)
        .unwrap_or_else(|| "-".to_string())
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

fn render_json_value(value: Option<FieldValue>) -> serde_json::Value {
    match value {
        Some(FieldValue::UInt(v)) => json!(v),
        Some(FieldValue::Int(v)) => json!(v),
        Some(FieldValue::Float(v)) => json!(v),
        Some(FieldValue::Bytes(bytes)) => json!(render_bytes(&bytes)), // fine to keep as hex string
        Some(FieldValue::Ascii(text)) => json!(text),                  // 👈 use raw text
        None => serde_json::Value::Null,
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
