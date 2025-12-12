# BinOcular  
*A schema-driven binary inspection toolkit for developers, reverse-engineers, and anyone who wants to stop guessing about byte layouts.*

BinOcular is a portable, cross-platform binary analysis toolkit written in Rust.  
It provides a structured, declarative way to explore unknown binary formats, visualize data layouts, and build custom parsers — without guesswork or hex-editor archaeology.

This workspace includes both a CLI and an early GUI shell, with a long-term goal of becoming a fully extensible open-source binary inspection suite.

## Features

- **Portable** — Windows-first, with Linux/macOS support planned  
- **Fast & Safe** — Rust’s safety guarantees without sacrificing performance  
- **Schema-Driven** — Describe binary structures using a clean YAML layout format  
- **Precise Visualization** — Offsets, endian behavior, integers, strings, blobs  
- **Extensible** — Designed for plugins, custom field types, and external tooling  
- **Developer-Friendly** — CLI output (table or JSON) for automation and testing  

## Project Status

Active development.  
The core schema engine, interpreter, and CLI are running; the GUI shell is in its early stages. A growing test suite verifies schema parsing, validation, and real-world use cases.

## Roadmap (High-Level)

- [x] Field interpreter & offset model  
- [x] Schema parser + validation  
- [x] CLI table + JSON output  
- [x] Initial GUI (egui desktop shell)  
- [ ] Full hex viewer + linked structure view  
- [ ] Plugin/interface system  
- [ ] Cross-platform build pipeline  
- [ ] Advanced schema features (conditionals, arrays, computed fields)

## Workspace Layout

- `crates/binocular-core` — core buffer abstractions and field interpreter  
- `crates/binocular-schema` — YAML AST, parser, schema validation logic  
- `crates/binocular-cli` — command-line tool for rendering interpreted fields  
- `crates/binocular-gui` — early-stage egui desktop application  

## Quickstart

1. Install the Rust toolchain (Rust 1.76+ recommended)  
2. Build all crates:

   ```bash
   cargo build
   ```

3. Run the full test suite:

   ```bash
   cargo test
   ```

## Using the CLI

The CLI consumes a **binary file** and a **YAML schema**.  
You must supply:

```
binocular-cli --schema <SCHEMA> <FILE>
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

# Emit JSON instead:
cargo run -p binocular-cli -- --schema packet.yml packet.bin --json
```

### Example output

```
NAME    | OFFSET            | TYPE       | VALUE                          | ERROR
magic   | 0 (0x00000000)    | u32        | 2882343476 (0xABCD1234)        | -
payload | 4 (0x00000004)    | ascii[5]   | "hello"                        | -
```

## GUI Preview

The GUI is a lightweight egui desktop shell that can open files, load schemas, and display early metadata.

```bash
cargo run -p binocular-gui
```

More structured visualizers (hex view, linked fields, schema explorer) are planned.

## Contributing

BinOcular is still solidifying its core architecture.  
Issues, ideas, and design discussions are welcome — especially around schema clarity, new field types, UX, and testing approaches.

## License

MIT License — see [`LICENSE`](LICENSE) for details.
