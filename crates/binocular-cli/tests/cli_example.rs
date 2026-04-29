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
    #[serde(rename = "type")]
    field_type: Option<String>,
    value: Value,
    error: Option<String>,
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

fn remove_if_exists(path: &PathBuf) {
    let _ = fs::remove_file(path);
}

fn run_cli(args: &[&str]) -> Result<std::process::Output, Box<dyn std::error::Error>> {
    Ok(cargo_bin_cmd!("binocular-cli").args(args).output()?)
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

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

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
fn dynamic_offset_schema_outputs_resolved_runtime_offset() -> Result<(), Box<dyn std::error::Error>>
{
    let schema_path = unique_temp_path("dynamic_offset_schema", "yaml");
    let bin_path = unique_temp_path("dynamic_offset_data", "bin");

    let schema = r#"
schema_name: "DynamicOffset"
schema_version: 1
endianness: little
fields:
  - name: "data_offset"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - name: "payload"
    type: ascii
    offset:
      kind: FieldRef
      value: "data_offset"
    length: 4
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, [4_u8, 0, 0, 0, b'T', b'E', b'S', b'T'])?;

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    assert_eq!(records.len(), 2);

    assert_eq!(records[0].name, "data_offset");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(4));

    assert_eq!(records[1].name, "payload");
    assert_eq!(records[1].offset, Some(4));
    assert_eq!(records[1].value.as_str(), Some("TEST"));

    Ok(())
}

#[test]
fn included_schema_outputs_expected_field_order() -> Result<(), Box<dyn std::error::Error>> {
    let root_schema_path = unique_temp_path("include_root_schema", "yaml");
    let common_schema_path = unique_temp_path("include_common_schema", "yaml");
    let bin_path = unique_temp_path("include_data", "bin");

    let root_schema = format!(
        r#"
schema_name: "Root"
schema_version: 1
endianness: little
fields:
  - name: "magic"
    type: u16
    offset:
      kind: Absolute
      value: 0
  - include: "{}"
  - name: "tail"
    type: bytes
    offset:
      kind: Absolute
      value: 6
    length: 2
"#,
        common_schema_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid include file name")?
    );

    let common_schema = r#"
schema_name: "Common"
schema_version: 1
endianness: little
fields:
  - name: "version"
    type: u16
    offset:
      kind: Absolute
      value: 2
  - name: "flags"
    type: u16
    offset:
      kind: Absolute
      value: 4
"#;

    fs::write(&root_schema_path, root_schema)?;
    fs::write(&common_schema_path, common_schema)?;
    fs::write(
        &bin_path,
        [0x34_u8, 0x12, 0x78, 0x56, 0xBC, 0x9A, 0xDE, 0xF0],
    )?;

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            root_schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    remove_if_exists(&root_schema_path);
    remove_if_exists(&common_schema_path);
    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    assert_eq!(records.len(), 4);

    assert_eq!(records[0].name, "magic");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(0x1234));

    assert_eq!(records[1].name, "version");
    assert_eq!(records[1].offset, Some(2));
    assert_eq!(records[1].value.as_u64(), Some(0x5678));

    assert_eq!(records[2].name, "flags");
    assert_eq!(records[2].offset, Some(4));
    assert_eq!(records[2].value.as_u64(), Some(0x9ABC));

    assert_eq!(records[3].name, "tail");
    assert_eq!(records[3].offset, Some(6));
    assert_eq!(records[3].value.as_str(), Some("DE F0"));

    Ok(())
}

#[test]
fn dynamic_length_schema_outputs_expected_payload() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/dynamic_length_schema.yaml");
    let bin_path = unique_temp_path("dynamic_length_data", "bin");
    fs::write(&bin_path, [3_u8, 0, b'C', b'A', b'T'])?;

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    assert_eq!(records.len(), 2);

    assert_eq!(records[0].name, "block_len");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(3));
    assert_eq!(records[0].error, None);

    assert_eq!(records[1].name, "payload");
    assert_eq!(records[1].offset, Some(2));
    assert_eq!(records[1].field_type.as_deref(), Some("ascii[3]"));
    assert_eq!(records[1].value.as_str(), Some("CAT"));
    assert_eq!(records[1].error, None);

    Ok(())
}

#[test]
fn expression_offset_and_length_schema_outputs_resolved_runtime_values(
) -> Result<(), Box<dyn std::error::Error>> {
    let schema_path = unique_temp_path("expression_schema", "yaml");
    let bin_path = unique_temp_path("expression_data", "bin");

    let schema = r#"
schema_name: "ExpressionExample"
schema_version: 1
endianness: little
fields:
  - name: "data_offset"
    type: u32
    offset:
      kind: Absolute
      value: 0
  - name: "block_len"
    type: u16
    offset:
      kind: Absolute
      value: 4
  - name: "payload"
    type: ascii
    offset:
      kind: Expr
      value:
        op: add
        left:
          field: "data_offset"
        right:
          const: 2
    length:
      expr:
        op: sub
        left:
          field: "block_len"
        right:
          const: 4
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, [4_u8, 0, 0, 0, 7, 0, b'C', b'A', b'T'])?;

    let output = cargo_bin_cmd!("binocular-cli")
        .args([
            "--json",
            "--schema",
            schema_path.to_str().ok_or("Invalid schema path")?,
            bin_path.to_str().ok_or("Invalid binary path")?,
        ])
        .output()?;

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    assert_eq!(records.len(), 3);

    assert_eq!(records[0].name, "data_offset");
    assert_eq!(records[0].offset, Some(0));
    assert_eq!(records[0].value.as_u64(), Some(4));

    assert_eq!(records[1].name, "block_len");
    assert_eq!(records[1].offset, Some(4));
    assert_eq!(records[1].value.as_u64(), Some(7));

    assert_eq!(records[2].name, "payload");
    assert_eq!(records[2].offset, Some(6));
    assert_eq!(records[2].field_type.as_deref(), Some("ascii[3]"));
    assert_eq!(records[2].value.as_str(), Some("CAT"));
    assert_eq!(records[2].error, None);

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
        !stdout.contains("Know your bytes"),
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
        stdout.contains("Know your bytes"),
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

#[test]
fn oversized_ascii_json_is_preview_only_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let schema_path = unique_temp_path("oversized_ascii_schema", "yaml");
    let bin_path = unique_temp_path("oversized_ascii_data", "bin");
    let payload = "A".repeat(257);

    let schema = r#"
schema_name: "OversizedAscii"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: ascii
    offset:
      kind: Absolute
      value: 0
    length: 257
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, payload.as_bytes())?;

    let output = run_cli(&[
        "--json",
        "--schema",
        schema_path.to_str().ok_or("Invalid schema path")?,
        bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    let value = records[0].value.as_str().ok_or("Expected string value")?;

    assert!(value.contains("257 bytes"));
    assert!(value.ends_with("... (257 bytes)"));
    assert_ne!(value, payload);

    Ok(())
}

#[test]
fn oversized_bytes_json_is_preview_only_by_default() -> Result<(), Box<dyn std::error::Error>> {
    let schema_path = unique_temp_path("oversized_bytes_schema", "yaml");
    let bin_path = unique_temp_path("oversized_bytes_data", "bin");
    let payload = vec![0xAB_u8; 257];

    let schema = r#"
schema_name: "OversizedBytes"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length: 257
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, &payload)?;

    let output = run_cli(&[
        "--json",
        "--schema",
        schema_path.to_str().ok_or("Invalid schema path")?,
        bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

    assert!(output.status.success(), "CLI did not exit successfully");

    let stdout = String::from_utf8(output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&stdout)?;
    let value = records[0].value.as_str().ok_or("Expected string value")?;

    assert!(value.contains("257 bytes"));
    assert!(value.ends_with("... (257 bytes)"));
    assert!(value.len() < 1024, "preview should stay compact");

    Ok(())
}

#[test]
fn full_bytes_restores_oversized_json_output() -> Result<(), Box<dyn std::error::Error>> {
    let ascii_schema_path = unique_temp_path("full_ascii_schema", "yaml");
    let ascii_bin_path = unique_temp_path("full_ascii_data", "bin");
    let ascii_payload = "B".repeat(257);

    let ascii_schema = r#"
schema_name: "FullAscii"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: ascii
    offset:
      kind: Absolute
      value: 0
    length: 257
"#;

    fs::write(&ascii_schema_path, ascii_schema)?;
    fs::write(&ascii_bin_path, ascii_payload.as_bytes())?;

    let ascii_output = run_cli(&[
        "--json",
        "--full-bytes",
        "--schema",
        ascii_schema_path.to_str().ok_or("Invalid schema path")?,
        ascii_bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    remove_if_exists(&ascii_schema_path);
    remove_if_exists(&ascii_bin_path);

    assert!(
        ascii_output.status.success(),
        "CLI did not exit successfully"
    );

    let ascii_stdout = String::from_utf8(ascii_output.stdout)?;
    let ascii_records: Vec<Record> = serde_json::from_str(&ascii_stdout)?;
    assert_eq!(
        ascii_records[0].value.as_str(),
        Some(ascii_payload.as_str())
    );

    let bytes_schema_path = unique_temp_path("full_bytes_schema", "yaml");
    let bytes_bin_path = unique_temp_path("full_bytes_data", "bin");
    let bytes_payload = vec![0xCD_u8; 257];

    let bytes_schema = r#"
schema_name: "FullBytes"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: bytes
    offset:
      kind: Absolute
      value: 0
    length: 257
"#;

    fs::write(&bytes_schema_path, bytes_schema)?;
    fs::write(&bytes_bin_path, &bytes_payload)?;

    let bytes_output = run_cli(&[
        "--json",
        "--full-bytes",
        "--schema",
        bytes_schema_path.to_str().ok_or("Invalid schema path")?,
        bytes_bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    remove_if_exists(&bytes_schema_path);
    remove_if_exists(&bytes_bin_path);

    assert!(
        bytes_output.status.success(),
        "CLI did not exit successfully"
    );

    let bytes_stdout = String::from_utf8(bytes_output.stdout)?;
    let bytes_records: Vec<Record> = serde_json::from_str(&bytes_stdout)?;
    let value = bytes_records[0]
        .value
        .as_str()
        .ok_or("Expected string value")?;
    let expected = std::iter::repeat_n("CD", 257).collect::<Vec<_>>().join(" ");
    assert_eq!(value, expected);

    Ok(())
}

#[test]
fn display_cap_boundary_is_not_truncated_at_256_bytes() -> Result<(), Box<dyn std::error::Error>> {
    let schema_path = unique_temp_path("boundary_ascii_schema", "yaml");
    let bin_path = unique_temp_path("boundary_ascii_data", "bin");
    let payload = "C".repeat(256);

    let schema = r#"
schema_name: "BoundaryAscii"
schema_version: 1
endianness: little
fields:
  - name: "payload"
    type: ascii
    offset:
      kind: Absolute
      value: 0
    length: 256
"#;

    fs::write(&schema_path, schema)?;
    fs::write(&bin_path, payload.as_bytes())?;

    let json_output = run_cli(&[
        "--json",
        "--schema",
        schema_path.to_str().ok_or("Invalid schema path")?,
        bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    let table_output = run_cli(&[
        "--schema",
        schema_path.to_str().ok_or("Invalid schema path")?,
        bin_path.to_str().ok_or("Invalid binary path")?,
    ])?;

    remove_if_exists(&schema_path);
    remove_if_exists(&bin_path);

    assert!(
        json_output.status.success(),
        "CLI did not exit successfully"
    );
    assert!(
        table_output.status.success(),
        "CLI did not exit successfully"
    );

    let json_stdout = String::from_utf8(json_output.stdout)?;
    let records: Vec<Record> = serde_json::from_str(&json_stdout)?;
    assert_eq!(records[0].value.as_str(), Some(payload.as_str()));

    let table_stdout = String::from_utf8(table_output.stdout)?;
    assert!(table_stdout.contains(&format!("\"{payload}\"")));
    assert!(!table_stdout.contains("256 bytes"));

    Ok(())
}
