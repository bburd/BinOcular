use std::fs;
use std::path::PathBuf;

use clap::Parser;
use binocular_core::buffer::MemoryBuffer;
use binocular_core::interpret::{interpret_field, FieldValue};
use binocular_schema::parser::parse_schema_str;

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
    let schema = parse_schema_str(&schema_str)?;

    let buffer = MemoryBuffer::from_vec(file_bytes);

    let mut results = Vec::new();
    for field in &schema.fields {
        let value = interpret_field(&buffer, field, Some(&schema));
        results.push((field.name.clone(), value));
    }

    if cli.json {
        use serde_json::json;
        let mut obj = serde_json::Map::new();
        for (name, value) in results {
            obj.insert(name, json!(format!("{:?}", value)));
        }
        println!("{}", serde_json::Value::Object(obj).to_string());
    } else {
        println!("Field              Value");
        println!("------------------------");
        for (name, value) in results {
            println!("{:<18} {:?}", name, value);
        }
    }

    Ok(())
}
