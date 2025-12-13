use assert_cmd::cargo::*;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Record {
    name: String,
    value: Value,
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
    let records: Vec<Record> = serde_json::from_str(&stdout)?;

    assert_eq!(records.len(), 4);

    assert_eq!(records[0].name, "magic");
    assert_eq!(records[0].value.as_u64(), Some(0x12345678));

    assert_eq!(records[1].name, "answer");
    assert_eq!(records[1].value.as_i64(), Some(42));

    assert_eq!(records[2].name, "value");
    let value = records[2].value.as_f64().ok_or("Expected float value")?;
    assert!((value - 1.0).abs() < f64::EPSILON);

    assert_eq!(records[3].name, "status");
    assert_eq!(records[3].value.as_str(), Some("OK!!"));

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
