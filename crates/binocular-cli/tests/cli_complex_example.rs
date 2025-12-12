use assert_cmd::Command;
use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Record {
    name: String,
    value: Value,
    error: Option<String>,
}

#[test]
fn complex_schema_outputs_expected_values() -> Result<(), Box<dyn std::error::Error>> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or("Failed to determine workspace root")?
        .to_path_buf();

    let schema_path = workspace_root.join("examples/complex_schema.yaml");
    let bin_path = workspace_root.join("examples/complex.bin");

    let output = Command::cargo_bin("binocular-cli")?
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

    assert_eq!(records.len(), 5);

    let find_record = |name: &str| -> Result<&Record, Box<dyn std::error::Error>> {
        records
            .iter()
            .find(|record| record.name == name)
            .ok_or_else(|| format!("Missing record: {name}").into())
    };

    let magic16 = find_record("magic16")?;
    assert!(magic16.error.is_none());
    assert_eq!(magic16.value.as_u64(), Some(0xABCD));

    let code32 = find_record("code32")?;
    assert!(code32.error.is_none());
    assert_eq!(code32.value.as_u64(), Some(0x11223344));

    let delta = find_record("delta")?;
    assert!(delta.error.is_none());
    assert_eq!(delta.value.as_i64(), Some(-100));

    let label = find_record("label")?;
    let label_value = label.value.as_str().ok_or("Expected label string")?;
    assert!(label_value.contains("BINOCULAR"));

    let tail = find_record("tail")?;
    let tail_value = tail.value.as_str().ok_or("Expected tail bytes")?;
    assert_eq!(tail_value, "DE AD BE EF 01 02");

    Ok(())
}
