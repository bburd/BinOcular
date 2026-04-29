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
use binocular_schema::ast::{FieldDef, FieldType};
use binocular_schema::parser::parse_schema_file;
use clap::Parser;
use serde_json::json;

const MAX_DISPLAY_BYTES: usize = 256;
const BANNER: &str = include_str!("../assets/ascii/banner.txt");
const BADGE: &str = include_str!("../assets/ascii/badge.txt");
const TAGLINE: &str = "BinOcular â€” Know your bytes. Donâ€™t guess them.";

#[derive(Parser)]
#[command(name = "binocular-cli")]
#[command(about = "BinOcular â€” Know your bytes. Donâ€™t guess them.", long_about = None)]
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

    /// Emit full bytes/ascii values instead of capped previews.
    #[arg(long)]
    full_bytes: bool,

    /// Prepend branding banner and version before human-readable output.
    #[arg(long)]
    branding: bool,
}

fn print_banner() {
    print!("{BANNER}");
    if !BANNER.ends_with('\n') {
        println!();
    }
    println!("{TAGLINE}");
}

fn print_badge() {
    eprint!("{BADGE}");
    if !BADGE.ends_with('\n') {
        eprintln!();
    }
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
    let schema = match parse_schema_file(schema) {
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
        .map(|eval| FieldRecord {
            name: eval.display_name,
            offset: Some(eval.resolved_offset),
            offset_hex: Some(format!("0x{:08X}", eval.resolved_offset)),
            field_type: render_field_type(&eval.field, eval.byte_len),
            byte_len: eval.byte_len,
            value: eval.value,
            error: eval.error,
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
                    "value": render_json_value(record.value, record.byte_len, cli.full_bytes),
                    "error": record.error,
                })
            })
            .collect();

        println!("{}", serde_json::Value::Array(json_records));

        // Early return to avoid executing the branding branch below when
        // `--json` is set. Branding is meant for human-readable output only.
        return Ok(());
    }

    print_badge();

    if cli.branding {
        print_banner();
        println!("v{}", env!("CARGO_PKG_VERSION"));
    }

    println!("NAME|OFFSET|TYPE|VALUE|ERROR");
    for record in records {
        let offset = render_offset(record.offset);
        let value = render_table_value(record.value.as_ref(), record.byte_len, cli.full_bytes);
        let error = record.error.unwrap_or_else(|| "-".to_string());

        println!(
            "{}|{}|{}|{}|{}",
            record.name, offset, record.field_type, value, error
        );
    }

    Ok(())
}

#[derive(Debug)]
struct FieldRecord {
    name: String,
    offset: Option<u64>,
    offset_hex: Option<String>,
    field_type: String,
    byte_len: usize,
    value: Option<FieldValue>,
    error: Option<String>,
}

fn render_offset(offset: Option<u64>) -> String {
    match offset {
        Some(value) => format!("{} (0x{value:08X})", value),
        None => "-".to_string(),
    }
}

fn render_field_type(field: &FieldDef, byte_len: usize) -> String {
    match field.ty {
        FieldType::U8 => "u8".to_string(),
        FieldType::U16 => "u16".to_string(),
        FieldType::U32 => "u32".to_string(),
        FieldType::U64 => "u64".to_string(),
        FieldType::I32 => "i32".to_string(),
        FieldType::F32 => "f32".to_string(),
        FieldType::Bytes => format!("bytes[{byte_len}]"),
        FieldType::Ascii => format!("ascii[{byte_len}]"),
    }
}

fn render_table_value(value: Option<&FieldValue>, byte_len: usize, full_bytes: bool) -> String {
    value
        .map(|value| render_field_value(value, byte_len, full_bytes, true))
        .unwrap_or_else(|| "-".to_string())
}

fn render_field_value(
    value: &FieldValue,
    byte_len: usize,
    full_bytes: bool,
    quote_ascii: bool,
) -> String {
    match value {
        FieldValue::UInt(v) => format!("{} (0x{:X})", v, v),
        FieldValue::Int(v) => format!("{} (0x{:X})", v, v),
        FieldValue::Float(v) => format!("{v}"),
        FieldValue::Bytes(bytes) => render_bytes(bytes, byte_len, full_bytes),
        FieldValue::Ascii(text) => render_ascii(text, byte_len, full_bytes, quote_ascii),
    }
}

fn render_json_value(
    value: Option<FieldValue>,
    byte_len: usize,
    full_bytes: bool,
) -> serde_json::Value {
    match value {
        Some(FieldValue::UInt(v)) => json!(v),
        Some(FieldValue::Int(v)) => json!(v),
        Some(FieldValue::Float(v)) => json!(v),
        Some(value @ FieldValue::Bytes(_)) | Some(value @ FieldValue::Ascii(_)) => {
            json!(render_field_value(&value, byte_len, full_bytes, false))
        }
        None => serde_json::Value::Null,
    }
}

fn render_bytes(bytes: &[u8], byte_len: usize, full_bytes: bool) -> String {
    let display_len = if full_bytes || byte_len <= MAX_DISPLAY_BYTES {
        bytes.len()
    } else {
        bytes.len().min(MAX_DISPLAY_BYTES)
    };
    let mut rendered: Vec<String> = bytes[..display_len]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();

    if !full_bytes && byte_len > MAX_DISPLAY_BYTES {
        rendered.push("...".to_string());
    }

    let rendered = rendered.join(" ");

    if !full_bytes && byte_len > MAX_DISPLAY_BYTES {
        format!("{rendered} ({byte_len} bytes)")
    } else {
        rendered
    }
}

fn render_ascii(text: &str, byte_len: usize, full_bytes: bool, quote_ascii: bool) -> String {
    let rendered = if full_bytes || byte_len <= MAX_DISPLAY_BYTES {
        render_ascii_preview(text, usize::MAX)
    } else {
        format!("{}... ({byte_len} bytes)", render_ascii_preview(text, MAX_DISPLAY_BYTES))
    };

    maybe_quote(rendered, quote_ascii)
}

fn render_ascii_preview(text: &str, max_bytes: usize) -> String {
    let mut escaped = String::new();
    let mut used_bytes = 0;

    for ch in text.chars() {
        let ch_bytes = ch.len_utf8();
        if used_bytes + ch_bytes > max_bytes {
            break;
        }
        escaped.extend(ch.escape_default());
        used_bytes += ch_bytes;
    }

    escaped
}

fn maybe_quote(rendered: String, quote_ascii: bool) -> String {
    if quote_ascii {
        format!("\"{rendered}\"")
    } else {
        rendered
    }
}
