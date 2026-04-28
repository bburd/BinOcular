use assert_cmd::cargo::*;
use serde::Deserialize;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Deserialize)]
struct Record {
    name: String,
    offset: Option<u64>,
    value: Value,
}

fn unique_temp_path(prefix: &str, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "{prefix}_{}_{}.{}",
        std::process::id(),
        nanos,
        extension
    ))
}

#[test]
fn simple_schema_outputs_expected_values() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/simple_schema.yaml");
    let bin_path = workspace_root.join("examples/simple.bin");

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let trimmed = stdout.trim_start();

    assert!(
        matches!(trimmed.chars().next(), Some('[') | Some('{')),
        "JSON output must start with an array or object delimiter"
    );

    let records: Vec<Record> = serde_json::from_str(&stdout)?;

    assert_eq!(records.len(), 4);

    assert_eq!(records[0].name, "magic");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(0x12345678));

    assert_eq!(records[1].name, "answer");
    assert_eq!(records[1].offset, Some(4));
    assert_eq!(records[1].value.as_i64(), Some(42));

    assert_eq!(records[2].name, "value");
    assert_eq!(records[2].offset, Some(8));
    let value = records[2].value.as_f64().ok_or("Expected float value")?;
    assert!((value - 1.0).abs() < f64::EPSILON);

    assert_eq!(records[3].name, "status");
    assert_eq!(records[3].offset, Some(12));
    assert_eq!(records[3].value.as_str(), Some("OK!!"));

    Ok(())
}

#[test]
fn repeated_fields_emit_multiple_json_records() -> Result<(), Box<dyn std::error::Error>> {
    let schema_path = unique_temp_path("repeat_schema", "yaml");
    let bin_path = unique_temp_path("repeat_data", "bin");

    let schema = r#"
schema_name: "Repeat"
schema_version: 1
endianness: little
fields:
  - name: "value"
    type: u16
    offset:
      kind: Absolute
      value: 0
    repeat:
      count: 3
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, [1_u8, 0, 2, 0, 3, 0])?;

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    let _ = fs::remove_file(&schema_path);
    let _ = fs::remove_file(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    assert_eq!(records.len(), 3);

    assert_eq!(records[0].name, "value[0]");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(1));

    assert_eq!(records[1].name, "value[1]");
    assert_eq!(records[1].offset, Some(2));
    assert_eq!(records[1].value.as_u64(), Some(2));

    assert_eq!(records[2].name, "value[2]");
    assert_eq!(records[2].offset, Some(4));
    assert_eq!(records[2].value.as_u64(), Some(3));

    Ok(())
}

#[test]
fn json_mode_emits_raw_json_without_branding() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/simple_schema.yaml");
    let bin_path = workspace_root.join("examples/simple.bin");

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let trimmed = stdout.trim_start();

    assert!(
        matches!(trimmed.chars().next(), Some('[') | Some('{')),
        "JSON output must start with an array or object delimiter"
    );

    assert!(
        !stdout.contains("BinOcular — Know your bytes. Don’t guess them."),
        "JSON mode must omit branding copy"
    );

    Ok(())
}

#[test]
fn json_mode_rejects_branding_to_preserve_stdout_contract() -> Result<(), Box<dyn std::error::Error>>
{
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/simple_schema.yaml");
    let bin_path = workspace_root.join("examples/simple.bin");

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--branding",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    assert!(
        !output.status.success(),
        "Branding must be rejected in JSON mode"
    );
    assert!(
        output.stdout.is_empty(),
        "JSON mode must keep stdout free of branding even on failure"
    );

    Ok(())
}

#[test]
fn branding_adds_header_to_human_output() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/simple_schema.yaml");
    let bin_path = workspace_root.join("examples/simple.bin");

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--branding",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    assert!(output.status.success(), "Branding mode should succeed");

    let stdout = String::from_utf8(output.stdout)?;

    assert!(
        stdout.contains("BinOcular — Know your bytes. Don’t guess them."),
        "Branding must include the banner and tagline"
    );
    assert!(
        stdout.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
        "Branding must show the current version"
    );
    assert!(
        stdout.contains("NAME|OFFSET|TYPE|VALUE|ERROR"),
        "Branding should not bypass the human-readable table output"
    );

    Ok(())
}
