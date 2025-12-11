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

## Contributing

Contributions are welcome once the core layout stabilizes.  
Feel free to open issues, suggest schema improvements, or discuss design decisions.

## License

MIT License — see [`LICENSE`](LICENSE) for details.
