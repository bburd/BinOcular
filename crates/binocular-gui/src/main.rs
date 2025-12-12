use std::{fs, path::PathBuf};

use binocular_core::buffer::{FileBuffer, MemoryBuffer};
use binocular_core::interpret::{interpret_schema, FieldEval};
use binocular_schema::ast::Schema;
use binocular_schema::parser::parse_schema_str;
use eframe::egui;

const MAX_HEX_BYTES: usize = 256;

struct Document {
    path: PathBuf,
    name: String,
    size: u64,
    buffer: MemoryBuffer,
    schema: Option<Schema>,
    field_evaluations: Option<Vec<FieldEval>>,
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

        if let Some(path) = rfd::FileDialog::new()
            .add_filter("YAML", &["yaml", "yml"])
            .pick_file()
        {
            let schema_str = match fs::read_to_string(&path) {
                Ok(contents) => contents,
                Err(err) => {
                    eprintln!("Failed to read schema file: {err}");
                    return;
                }
            };

            let schema = match parse_schema_str(&schema_str) {
                Ok(schema) => schema,
                Err(err) => {
                    eprintln!("Failed to parse schema: {err}");
                    return;
                }
            };

            if let Some(doc) = self.documents.get_mut(doc_index) {
                let evaluations = interpret_schema(&doc.buffer, &schema);
                doc.schema = Some(schema);
                doc.field_evaluations = Some(evaluations);
            }
        }
    }

    fn load_document_from_path(path: PathBuf) -> Result<Document, String> {
        let metadata = fs::metadata(&path).map_err(|err| err.to_string())?;
        if !metadata.is_file() {
            return Err("Selected path is not a file".to_string());
        }

        let size = metadata.len();
        let data = fs::read(&path).map_err(|err| err.to_string())?;
        let buffer = MemoryBuffer::from_vec(data);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.display().to_string());

        Ok(Document {
            path,
            name,
            size,
            buffer,
            schema: None,
            field_evaluations: None,
        })
    }
}

impl Document {
    fn read_bytes(&self, offset: u64, len: usize) -> Option<&[u8]> {
        self.buffer.read_bytes(offset, len).ok()
    }
}

fn draw_hex_view(ui: &mut egui::Ui, doc: &Document) {
    const BYTES_PER_ROW: usize = 16;
    let to_show = doc.size.min(MAX_HEX_BYTES as u64) as usize;

    if to_show == 0 {
        ui.label("File is empty.");
        return;
    }

    let Some(bytes) = doc.read_bytes(0, to_show) else {
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
            row_start, hex_column, ascii_column
        ));
    }
}

impl eframe::App for BinOcularApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
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
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(index) = self.current_doc {
                if let Some(doc) = self.documents.get(index) {
                    ui.heading(&doc.name);
                    ui.label(format!("Size: {}", format_size(doc.size)));
                    ui.separator();
                    draw_hex_view(ui, doc);
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
