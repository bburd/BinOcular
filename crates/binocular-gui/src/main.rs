use std::{fs, path::PathBuf, sync::Arc};

use binocular_core::buffer::{FileBuffer, MemoryBuffer, MmapBuffer};
use binocular_core::interpret::{interpret_schema, FieldEval, FieldValue};
use binocular_schema::ast::Schema;
use binocular_schema::parser::parse_schema_str;
use eframe::egui;

const HEX_PAGE_SIZE: usize = 1024;
const MMAP_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;

struct Document {
    _path: PathBuf,
    name: String,
    size: u64,
    buffer: Arc<dyn FileBuffer>,
    schema: Option<Schema>,
    field_evaluations: Option<Vec<FieldEval>>,
    last_error: Option<String>,
    last_error_is_offset: bool,
    schema_path: Option<PathBuf>,
    hex_start_offset: u64,
    hex_offset_input: String,
}

struct BinOcularApp {
    documents: Vec<Document>,
    current_doc: Option<usize>,
}

impl BinOcularApp {
    fn new() -> Self {
        Self {
            documents: Vec::new(),
            current_doc: None,
        }
    }

    fn open_document_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match Self::load_document_from_path(path) {
                Ok(document) => {
                    self.current_doc = Some(self.documents.len());
                    self.documents.push(document);
                }
                Err(err) => {
                    eprintln!("Failed to open file: {err}");
                }
            }
        }
    }

    fn load_schema_for_current_document(&mut self) {
        let Some(doc_index) = self.current_doc else {
            return;
        };

        let Some(path) = rfd::FileDialog::new()
            .add_filter("YAML", &["yaml", "yml"])
            .pick_file()
        else {
            return;
        };

        let Some(doc) = self.documents.get_mut(doc_index) else {
            return;
        };

        let schema_str = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                doc.last_error = Some(format!("Failed to read schema file: {err}"));
                doc.last_error_is_offset = false;
                return;
            }
        };

        let schema = match parse_schema_str(&schema_str) {
            Ok(schema) => schema,
            Err(err) => {
                doc.last_error = Some(format!("Failed to parse or validate schema: {err}"));
                doc.last_error_is_offset = false;
                return;
            }
        };

        let evaluations = interpret_schema(doc.buffer.as_ref(), &schema);
        doc.schema = Some(schema);
        doc.field_evaluations = Some(evaluations);
        doc.schema_path = Some(path);
        doc.last_error = None;
        doc.last_error_is_offset = false;
    }

    fn load_document_from_path(path: PathBuf) -> Result<Document, String> {
        let metadata = fs::metadata(&path).map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            return Err("Selected path is not a file".to_string());
        }

        let size = metadata.len();
        let buffer: Arc<dyn FileBuffer> = if size < MMAP_THRESHOLD_BYTES {
            let data = fs::read(&path).map_err(|err| err.to_string())?;
            Arc::new(MemoryBuffer::from_vec(data))
        } else {
            let mmap = MmapBuffer::open(&path).map_err(|err| err.to_string())?;
            Arc::new(mmap)
        };
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        Ok(Document {
            _path: path,
            name,
            size,
            buffer,
            schema: None,
            field_evaluations: None,
            last_error: None,
            last_error_is_offset: false,
            schema_path: None,
            hex_start_offset: 0,
            hex_offset_input: "0x0".to_string(),
        })
    }

    fn reload_schema_for_current_document(&mut self) {
        let Some(doc_index) = self.current_doc else {
            return;
        };

        let Some(doc) = self.documents.get_mut(doc_index) else {
            return;
        };

        let Some(schema_path) = doc.schema_path.clone() else {
            doc.last_error = Some("No schema loaded to reload".to_string());
            doc.last_error_is_offset = false;
            return;
        };

        let schema_str = match fs::read_to_string(&schema_path) {
            Ok(contents) => contents,
            Err(err) => {
                doc.last_error = Some(format!("Failed to read schema file: {err}"));
                doc.last_error_is_offset = false;
                return;
            }
        };

        let schema = match parse_schema_str(&schema_str) {
            Ok(schema) => schema,
            Err(err) => {
                doc.last_error = Some(format!("Failed to parse or validate schema: {err}"));
                doc.last_error_is_offset = false;
                return;
            }
        };

        let evaluations = interpret_schema(doc.buffer.as_ref(), &schema);
        doc.schema = Some(schema);
        doc.field_evaluations = Some(evaluations);
        doc.last_error = None;
        doc.last_error_is_offset = false;
    }
}

impl Document {
    fn read_bytes(&self, offset: u64, len: usize) -> Option<&[u8]> {
        self.buffer.read_bytes(offset, len).ok()
    }
}

fn draw_hex_view(ui: &mut egui::Ui, doc: &Document) {
    const BYTES_PER_ROW: usize = 16;
    let remaining = doc.size.saturating_sub(doc.hex_start_offset);
    let to_show = remaining.min(HEX_PAGE_SIZE as u64) as usize;

    if to_show == 0 {
        ui.label("File is empty.");
        return;
    }

    let Some(bytes) = doc.read_bytes(doc.hex_start_offset, to_show) else {
        ui.label("Failed to read file data.");
        return;
    };

    for row_start in (0..bytes.len()).step_by(BYTES_PER_ROW) {
        let row_end = (row_start + BYTES_PER_ROW).min(bytes.len());
        let row = &bytes[row_start..row_end];

        let mut hex_column = String::new();
        let mut ascii_column = String::new();

        for i in 0..BYTES_PER_ROW {
            if let Some(byte) = row.get(i) {
                hex_column.push_str(&format!("{:02X} ", byte));
                let ch = if (0x20..=0x7E).contains(byte) {
                    *byte as char
                } else {
                    '.'
                };
                ascii_column.push(ch);
            } else {
                hex_column.push_str("   ");
            }
        }

        ui.monospace(format!(
            "{:08X}: {} {}",
            doc.hex_start_offset + row_start as u64,
            hex_column,
            ascii_column
        ));
    }
}

fn format_offset(offset: &binocular_schema::ast::OffsetKind) -> String {
    match offset {
        binocular_schema::ast::OffsetKind::Absolute(value) => {
            format!("0x{value:X} ({value})")
        }
        binocular_schema::ast::OffsetKind::Expr(expr) => expr.clone(),
    }
}

fn format_value(value: &FieldValue) -> String {
    match value {
        FieldValue::UInt(v) => format!("{v} (0x{v:X})"),
        FieldValue::Int(v) => format!("{v} (0x{v:X})"),
        FieldValue::Float(v) => format!("{v}"),
        FieldValue::Bytes(bytes) => bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" "),
        FieldValue::Ascii(text) => text.clone(),
    }
}

fn draw_field_table(ui: &mut egui::Ui, evaluations: &[FieldEval]) {
    egui::Grid::new("field_evaluations")
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Offset");
            ui.strong("Type");
            ui.strong("Value");
            ui.strong("Error");
            ui.end_row();

            for eval in evaluations {
                ui.label(&eval.field.name);
                ui.monospace(format_offset(&eval.field.offset));
                ui.label(format!("{:?}", eval.field.ty));
                if let Some(value) = &eval.value {
                    ui.label(format_value(value));
                } else {
                    ui.label("-");
                }
                if let Some(error) = &eval.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                } else {
                    ui.label("-");
                }
                ui.end_row();
            }
        });
}

impl eframe::App for BinOcularApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                let schema_reload_enabled = self
                    .current_doc
                    .and_then(|index| self.documents.get(index))
                    .is_some_and(|doc| doc.schema_path.is_some());

                ui.menu_button("File", |ui| {
                    if ui.button("Open...").clicked() {
                        ui.close_menu();
                        self.open_document_dialog();
                    }
                });
                ui.menu_button("Schema", |ui| {
                    if ui.button("Load...").clicked() {
                        ui.close_menu();
                        self.load_schema_for_current_document();
                    }

                    if ui
                        .add_enabled(schema_reload_enabled, egui::Button::new("Reload"))
                        .clicked()
                    {
                        ui.close_menu();
                        self.reload_schema_for_current_document();
                    }
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(index) = self.current_doc {
                if let Some(doc) = self.documents.get_mut(index) {
                    ui.heading(&doc.name);
                    ui.label(format!("Size: {}", format_size(doc.size)));

                    if let Some(error) = doc.last_error.as_deref() {
                        let error_text = error.to_owned();
                        ui.horizontal(|ui| {
                            ui.colored_label(
                                ui.visuals().error_fg_color,
                                egui::RichText::new(&error_text).strong(),
                            );
                            if ui.button("Dismiss").clicked() {
                                doc.last_error = None;
                                doc.last_error_is_offset = false;
                            }
                        });
                        ui.add_space(4.0);
                    }

                    ui.horizontal(|ui| {
                        ui.label("Go to offset:");
                        let _ = ui.text_edit_singleline(&mut doc.hex_offset_input);

                        if ui.button("Go").clicked() {
                            let input = doc.hex_offset_input.trim();
                            let parsed_offset = if let Some(hex) = input
                                .strip_prefix("0x")
                                .or_else(|| input.strip_prefix("0X"))
                            {
                                u64::from_str_radix(hex, 16)
                            } else {
                                input.parse::<u64>()
                            };

                            match parsed_offset {
                                Ok(offset) => {
                                    let max_start = doc.size.saturating_sub(HEX_PAGE_SIZE as u64);
                                    let clamped = offset.min(max_start);
                                    doc.hex_start_offset = clamped;
                                    doc.hex_offset_input = format!("0x{:X}", clamped);
                                    if doc.last_error_is_offset {
                                        doc.last_error = None;
                                        doc.last_error_is_offset = false;
                                    }
                                }
                                Err(_) => {
                                    doc.last_error = Some(format!(
                                        "Invalid offset: {}",
                                        doc.hex_offset_input.trim()
                                    ));
                                    doc.last_error_is_offset = true;
                                }
                            }
                        }

                        let max_start = doc.size.saturating_sub(HEX_PAGE_SIZE as u64);
                        if ui
                            .add_enabled(
                                doc.hex_start_offset > 0,
                                egui::Button::new("Previous page"),
                            )
                            .clicked()
                        {
                            let new_offset =
                                doc.hex_start_offset.saturating_sub(HEX_PAGE_SIZE as u64);
                            doc.hex_start_offset = new_offset;
                            doc.hex_offset_input = format!("0x{:X}", new_offset);
                            if doc.last_error_is_offset {
                                doc.last_error = None;
                                doc.last_error_is_offset = false;
                            }
                        }

                        if ui
                            .add_enabled(
                                doc.hex_start_offset < max_start,
                                egui::Button::new("Next page"),
                            )
                            .clicked()
                        {
                            let new_offset = doc
                                .hex_start_offset
                                .saturating_add(HEX_PAGE_SIZE as u64)
                                .min(max_start);
                            doc.hex_start_offset = new_offset;
                            doc.hex_offset_input = format!("0x{:X}", new_offset);
                            if doc.last_error_is_offset {
                                doc.last_error = None;
                                doc.last_error_is_offset = false;
                            }
                        }
                    });

                    ui.separator();
                    draw_hex_view(ui, doc);

                    if doc.schema.is_some() {
                        ui.separator();
                        ui.heading("Interpreted Fields");

                        if let Some(schema) = doc.schema.as_ref() {
                            let mut schema_label = format!(
                                "Schema: {} (v{})",
                                schema.schema_name, schema.schema_version
                            );

                            if let Some(schema_path) = doc.schema_path.as_ref() {
                                if let Some(file_name) = schema_path.file_name() {
                                    schema_label
                                        .push_str(&format!(" — {}", file_name.to_string_lossy()));
                                }
                            }

                            ui.label(schema_label);
                        }

                        if let Some(evaluations) = doc.field_evaluations.as_ref() {
                            if !evaluations.is_empty() {
                                draw_field_table(ui, evaluations);
                            }
                        }
                    }
                    return;
                }
            }

            ui.heading("Welcome to BinOcular");
            ui.label("Open a file to get started.");
        });

        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            let status_left = self
                .current_doc
                .and_then(|index| self.documents.get(index))
                .map(|doc| doc.name.clone())
                .unwrap_or_else(|| "No file open".to_string());

            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true),
                |ui| {
                    ui.label(status_left);
                    ui.label("BinOcular pre-alpha");
                },
            );
        });
    }
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;

    if bytes >= MB as u64 {
        format!("{:.2} MB", bytes as f64 / MB)
    } else if bytes >= KB as u64 {
        format!("{:.2} KB", bytes as f64 / KB)
    } else {
        format!("{bytes} bytes")
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "BinOcular",
        options,
        Box::new(|_cc| Box::new(BinOcularApp::new())),
    )
}
