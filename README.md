# BinOcular

BinOcular is a portable, cross-platform binary inspection and analysis toolkit written in Rust.
It provides a structured, schema-driven way to explore unknown binary formats, visualize data layouts, and build custom parsers—without guesswork.

## Features

- **Portable** — Windows-first, with Linux/macOS support planned  
- **Fast & Safe** — Built on Rust’s zero-cost abstractions  
- **Schema-Driven** — Define, load, and validate custom binary structures  
- **Flexible Views** — Visualize offsets, endian patterns, and raw byte layouts  
- **Extensible** — Support for plugins, custom readers, and custom parsers  
- **Developer-Friendly** — Clean API for automation and tool integration

## Project Goals

BinOcular aims to become a reliable open-source utility for anyone who needs clear, practical insight into binary files—from reverse engineers to curious developers.

## Status

Early development.  
Core design, schema format, and foundational Rust modules are being implemented.

## Roadmap (High-Level)

- [ ] Core binary reader engine  
- [ ] Schema definition and validation system  
- [ ] Hex/structure combined viewer  
- [ ] Plugin interface  
- [ ] Cross-platform builds  
- [ ] CLI utilities for parsing and validation  
- [ ] Optional GUI layer (future)

## Workspace Layout

- `crates/binocular-core` — buffer abstractions and the field interpreter.
- `crates/binocular-schema` — YAML schema AST, parser, and validation helpers.
- `crates/binocular-cli` — command-line companion that renders interpreted fields.
- `crates/binocular-gui` — early-stage egui desktop shell.

## Quickstart

1. Install the Rust toolchain (Rust 1.76+ recommended).
2. Build all crates: `cargo build`.
3. Run tests to verify parsers and interpreters: `cargo test`.

## Using the CLI

The CLI consumes a binary file and a YAML schema that describes the layout. The schema format is defined in
`binocular-schema` and validated before use.

Example schema (`packet.yml`):

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

Example binary creation and inspection:

```bash
# Create a tiny sample binary (0xABCD1234 followed by "hello")
python - <<'PY'
with open('packet.bin', 'wb') as f:
    f.write((0xABCD1234).to_bytes(4, 'little'))
    f.write(b'hello')
PY

# Render the table view
cargo run -p binocular-cli -- packet.bin --schema packet.yml

# Emit machine-readable JSON instead
cargo run -p binocular-cli -- packet.bin --schema packet.yml --json
```

Sample table output:

```
NAME|OFFSET|TYPE|VALUE|ERROR
magic|0 (0x00000000)|u32|2882343476 (0xABCD1234)|-
payload|4 (0x00000004)|ascii[5]|"hello"|-
```

## GUI Preview

The `binocular-gui` crate provides an egui-based desktop shell that can open files and display metadata. Launch it with:

```bash
cargo run -p binocular-gui
```

The current build shows file details and scaffolding for a hex view; more visualizers will arrive as the core stabilizes.

## Contributing

Contributions are welcome once the core layout stabilizes.
Feel free to open issues, suggest schema improvements, or discuss design decisions.

## License

MIT License — see [`LICENSE`](LICENSE) for details.
