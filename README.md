<p align="center">
  <img src="crates/binocular-cli/assets/images/banner.png" alt="BinOcular banner" width="600" />
</p>
<p align="center"><i>BinOcular — Know your bytes. Don’t guess them.</i></p>

# BinOcular
*A schema-driven binary inspection toolkit for developers, reverse-engineers, and anyone who wants to stop guessing about byte layouts.*

BinOcular is a portable, cross-platform binary analysis toolkit written in Rust.  
It provides a structured, declarative way to explore unknown binary formats, visualize data layouts, and build custom parsers — without guesswork or hex-editor archaeology.

This workspace includes both a CLI and a GUI, with a long-term goal of becoming a fully extensible open-source binary inspection suite.

## Features

- **Portable** — Windows-first, with Linux/macOS support planned  
- **Fast & Safe** — Rust’s safety guarantees without sacrificing performance  
- **Schema-Driven** — Describe binary structures using a clean YAML layout format  
- **Precise Visualization** — Offsets, endian behavior, integers, strings, blobs  
- **Extensible** — Designed for future plugins, custom field types, and tooling  
- **Developer-Friendly** — CLI output (table or JSON) for automation and testing  

## Project Status

Active development (**v0.2.0**).  
The core schema engine, interpreter, CLI, and GUI MVP are functional.  
This release focused on hardening and crash resistance under malformed input.

## v0.2.0 Hardening Summary

**TL;DR:** v0.2.0 made BinOcular crash-resistant.

### What changed

- Removed panic paths in the interpreter (including `unwrap` in numeric decoding)
- Added property tests for schema parsing and validation
- Added a randomized crash harness for the end-to-end pipeline

### What it means

- Arbitrary or malformed input now yields structured errors instead of crashes
- Much higher confidence in stability across parser + interpreter + CLI flow

### What didn't change

- No new features
- No schema expansion
- No UI changes

## Roadmap (High-Level)

- [x] Field interpreter & offset model  
- [x] Schema parser + validation  
- [x] CLI table + JSON output  
- [x] GUI MVP (hex view + interpreted fields)  
- [ ] Paging-backed hex viewer for large files  
- [x] Property tests and fuzzing hardening
- [ ] Plugin/interface system  
- [ ] Advanced schema features (arrays, expressions, nested structures)  

## Workspace Layout

- `crates/binocular-core` — core buffer abstractions and field interpreter  
- `crates/binocular-schema` — YAML AST, parser, and schema validation  
- `crates/binocular-cli` — command-line tool for inspecting binaries  
- `crates/binocular-gui` — egui desktop application  

## Quickstart

1. Install the Rust toolchain (Rust 1.76+ recommended)
2. Build all crates:

```bash
cargo build --workspace
```

3. Run the full test suite:

```bash
cargo test --workspace
```

## Using the CLI

The CLI consumes a **binary file** and a **YAML schema**.

```bash
cargo run -p binocular-cli -- --schema <SCHEMA> <FILE>
```

### Example schema (`packet.yml`)

```yaml
schema_name: "Packet"
schema_version: 1
endianness: little
fields:
  - name: "magic"
    type: u32
    offset: { kind: Absolute, value: 0 }
  - name: "payload"
    type: ascii
    offset: { kind: Absolute, value: 4 }
    length: 5
```

### Example binary & inspection

```bash
# Create sample binary
python - <<'PY'
with open('packet.bin', 'wb') as f:
    f.write((0xABCD1234).to_bytes(4, 'little'))
    f.write(b'hello')
PY

# Render structured table view
cargo run -p binocular-cli -- --schema packet.yml packet.bin

# Emit JSON instead
cargo run -p binocular-cli -- --schema packet.yml packet.bin --json
```

### Example output

```
NAME    | OFFSET            | TYPE       | VALUE                          | ERROR
magic   | 0 (0x00000000)    | u32        | 2882343476 (0xABCD1234)        | -
payload | 4 (0x00000004)    | ascii[5]   | "hello"                        | -
```

## GUI

The GUI is a lightweight egui desktop application that can:

- Open binary files
- Load YAML schemas
- Display a hex preview
- Show interpreted fields

```bash
cargo run -p binocular-gui
```

More advanced visualizations and large-file support are planned.

## Contributing

BinOcular is still evolving.  
Issues, ideas, and design discussions are welcome — especially around schema clarity, new field types, UX, and testing.

## License

MIT License — see [`LICENSE`](LICENSE) for details.
