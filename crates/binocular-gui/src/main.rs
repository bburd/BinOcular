use std::{collections::HashSet, fs, path::PathBuf, sync::Arc};

use binocular_core::buffer::{FileBuffer, MemoryBuffer, MmapBuffer};
use binocular_core::interpret::{interpret_schema, FieldEval, FieldValue};
use binocular_schema::ast::Schema;
use binocular_schema::error::SchemaError;
use binocular_schema::parser::parse_schema_str_with_base_path;
use eframe::egui;

const HEX_PAGE_SIZE: usize = 1024;
const HEX_VIEW_HEIGHT: f32 = 300.0;
const SPLITTER_WIDTH: f32 = 6.0;
const HEX_PANE_MIN_WIDTH: f32 = 320.0;
const FIELD_PANE_MIN_WIDTH: f32 = 220.0;
const SCHEMA_PANE_MIN_WIDTH: f32 = 220.0;
const SIDE_BY_SIDE_MIN_WIDTH: f32 =
    HEX_PANE_MIN_WIDTH + FIELD_PANE_MIN_WIDTH + SCHEMA_PANE_MIN_WIDTH + (SPLITTER_WIDTH * 2.0);
const DEFAULT_LEFT_SPLIT_FRACTION: f32 = 0.50;
const DEFAULT_RIGHT_SPLIT_FRACTION: f32 = 0.75;
const MMAP_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DISPLAY_BYTES: usize = 256;
const SEARCH_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SearchMode {
    Ascii,
    Hex,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DiagnosticCategory {
    SchemaParse,
    SchemaValidation,
    IncludeResolution,
    Interpretation,
    Search,
}

impl DiagnosticCategory {
    fn label(self) -> &'static str {
        match self {
            DiagnosticCategory::SchemaParse => "Schema Parse",
            DiagnosticCategory::SchemaValidation => "Schema Validation",
            DiagnosticCategory::IncludeResolution => "Include",
            DiagnosticCategory::Interpretation => "Interpretation",
            DiagnosticCategory::Search => "Search",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceLocation {
    path: PathBuf,
    line: usize,
    column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DiagnosticTarget {
    Schema {
        location: Option<SourceLocation>,
    },
    Field {
        name: String,
        offset: u64,
        byte_len: usize,
        offset_valid: bool,
    },
    Search,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GuiDiagnostic {
    category: DiagnosticCategory,
    message: String,
    target: DiagnosticTarget,
    snippet: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SchemaMatch {
    line: usize,
    line_start_byte: usize,
    line_end_byte: usize,
    snippet: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SchemaHighlight {
    line: usize,
    start_byte: usize,
    end_byte: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClickedField {
    display_name: String,
    schema_field_name: String,
    offset: u64,
    byte_len: usize,
    offset_valid: bool,
}

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
    schema_editor_text: String,
    schema_editor_dirty: bool,
    schema_editor_error: Option<String>,
    schema_diagnostics: Vec<GuiDiagnostic>,
    active_schema_diagnostic: Option<usize>,
    schema_match: Option<SchemaMatch>,
    schema_match_scroll_pending: bool,
    schema_cursor_name_highlight: Option<SchemaHighlight>,
    schema_editor_cursor_char_index: Option<usize>,
    hex_start_offset: u64,
    hex_offset_input: String,
    selected_field_range: Option<(u64, usize)>,
    selected_field_name: Option<String>,
    selected_field_scroll_pending: bool,
    search_query: String,
    search_mode: SearchMode,
    active_search_query: String,
    active_search_pattern_len: usize,
    search_matches: Vec<u64>,
    current_search_match: Option<usize>,
    search_error: Option<String>,
    field_filter_query: String,
    field_filter_errors_only: bool,
    collapsed_field_groups: HashSet<String>,
}

struct BinOcularApp {
    documents: Vec<Document>,
    current_doc: Option<usize>,
    left_split_fraction: f32,
    right_split_fraction: f32,
}

impl BinOcularApp {
    fn new() -> Self {
        Self {
            documents: Vec::new(),
            current_doc: None,
            left_split_fraction: DEFAULT_LEFT_SPLIT_FRACTION,
            right_split_fraction: DEFAULT_RIGHT_SPLIT_FRACTION,
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

        let schema_source_text = match fs::read_to_string(&path) {
            Ok(source_text) => source_text,
            Err(err) => {
                let error = format!("Failed to read schema source: {err}");
                doc.last_error = Some(error.clone());
                doc.set_schema_diagnostic(GuiDiagnostic {
                    category: DiagnosticCategory::IncludeResolution,
                    message: error,
                    target: DiagnosticTarget::Schema {
                        location: Some(SourceLocation {
                            path: path.clone(),
                            line: 1,
                            column: 1,
                        }),
                    },
                    snippet: None,
                });
                doc.last_error_is_offset = false;
                return;
            }
        };

        let schema = match parse_schema_str_with_base_path(&schema_source_text, &path) {
            Ok(schema) => schema,
            Err(err) => {
                let error = format!("Failed to parse or validate schema: {err}");
                doc.last_error = Some(error);
                let diagnostic =
                    schema_error_diagnostic(&err, Some(&path), Some(&schema_source_text));
                doc.set_schema_diagnostic(diagnostic);
                doc.last_error_is_offset = false;
                return;
            }
        };

        doc.schema_path = Some(path);
        doc.schema_editor_text = schema_source_text;
        doc.commit_schema(schema);
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
            schema_editor_text: String::new(),
            schema_editor_dirty: false,
            schema_editor_error: None,
            schema_diagnostics: Vec::new(),
            active_schema_diagnostic: None,
            schema_match: None,
            schema_match_scroll_pending: false,
            schema_cursor_name_highlight: None,
            schema_editor_cursor_char_index: None,
            hex_start_offset: 0,
            hex_offset_input: "0x0".to_string(),
            selected_field_range: None,
            selected_field_name: None,
            selected_field_scroll_pending: false,
            search_query: String::new(),
            search_mode: SearchMode::Ascii,
            active_search_query: String::new(),
            active_search_pattern_len: 0,
            search_matches: Vec::new(),
            current_search_match: None,
            search_error: None,
            field_filter_query: String::new(),
            field_filter_errors_only: false,
            collapsed_field_groups: HashSet::new(),
        })
    }

    fn reload_schema_for_current_document(&mut self) {
        let Some(doc_index) = self.current_doc else {
            return;
        };

        let Some(doc) = self.documents.get_mut(doc_index) else {
            return;
        };

        doc.reload_schema_from_disk();
    }
}

impl Document {
    fn commit_schema(&mut self, schema: Schema) {
        let evaluations = interpret_schema(self.buffer.as_ref(), &schema);
        self.schema = Some(schema);
        self.field_evaluations = Some(evaluations);
        self.schema_editor_dirty = false;
        self.schema_editor_error = None;
        self.clear_schema_cursor_name_highlight();
        self.schema_editor_cursor_char_index = None;
        self.clear_schema_diagnostics();
        self.last_error = None;
        self.last_error_is_offset = false;
        self.clear_selected_field();
    }

    fn clear_schema_match(&mut self) {
        self.schema_match = None;
        self.schema_match_scroll_pending = false;
    }

    fn clear_schema_cursor_name_highlight(&mut self) {
        self.schema_cursor_name_highlight = None;
    }

    fn clear_schema_diagnostics(&mut self) {
        self.schema_diagnostics.clear();
        self.active_schema_diagnostic = None;
    }

    fn set_schema_diagnostic(&mut self, diagnostic: GuiDiagnostic) {
        self.schema_diagnostics = vec![diagnostic];
        self.active_schema_diagnostic = Some(0);
        self.clear_schema_match();
        self.clear_schema_cursor_name_highlight();
    }

    fn apply_schema_editor(&mut self) {
        let Some(schema_path) = self.schema_path.clone() else {
            let message = "No schema path available for Apply Schema".to_string();
            self.schema_editor_error = Some(message.clone());
            self.set_schema_diagnostic(GuiDiagnostic {
                category: DiagnosticCategory::SchemaValidation,
                message,
                target: DiagnosticTarget::Schema { location: None },
                snippet: None,
            });
            return;
        };

        match parse_schema_str_with_base_path(&self.schema_editor_text, &schema_path) {
            Ok(schema) => self.commit_schema(schema),
            Err(err) => {
                self.schema_editor_error =
                    Some(format!("Failed to parse or validate schema: {err}"));
                let diagnostic = schema_error_diagnostic(
                    &err,
                    Some(&schema_path),
                    Some(&self.schema_editor_text),
                );
                self.set_schema_diagnostic(diagnostic);
                self.schema_editor_dirty = true;
            }
        }
    }

    fn reload_schema_from_disk(&mut self) {
        let Some(schema_path) = self.schema_path.clone() else {
            let message = "No schema loaded to reload".to_string();
            self.schema_editor_error = Some(message.clone());
            self.set_schema_diagnostic(GuiDiagnostic {
                category: DiagnosticCategory::SchemaValidation,
                message,
                target: DiagnosticTarget::Schema { location: None },
                snippet: None,
            });
            return;
        };

        let schema_source_text = match fs::read_to_string(&schema_path) {
            Ok(source_text) => source_text,
            Err(err) => {
                let error = format!("Failed to read schema source: {err}");
                self.schema_editor_error = Some(error.clone());
                self.set_schema_diagnostic(GuiDiagnostic {
                    category: DiagnosticCategory::IncludeResolution,
                    message: error.clone(),
                    target: DiagnosticTarget::Schema {
                        location: Some(SourceLocation {
                            path: schema_path.clone(),
                            line: 1,
                            column: 1,
                        }),
                    },
                    snippet: None,
                });
                self.last_error = Some(error);
                self.last_error_is_offset = false;
                return;
            }
        };

        match parse_schema_str_with_base_path(&schema_source_text, &schema_path) {
            Ok(schema) => {
                self.schema_editor_text = schema_source_text;
                self.commit_schema(schema);
            }
            Err(err) => {
                let error = format!("Failed to parse or validate schema: {err}");
                self.schema_editor_text = schema_source_text;
                self.schema_editor_dirty = false;
                self.schema_editor_error = Some(error.clone());
                self.clear_schema_cursor_name_highlight();
                self.schema_editor_cursor_char_index = None;
                let diagnostic = schema_error_diagnostic(
                    &err,
                    Some(&schema_path),
                    Some(&self.schema_editor_text),
                );
                self.set_schema_diagnostic(diagnostic);
                self.last_error = Some(error);
                self.last_error_is_offset = false;
            }
        }
    }

    fn diagnostics(&self) -> Vec<GuiDiagnostic> {
        let mut diagnostics = self.schema_diagnostics.clone();

        if let Some(evaluations) = self.field_evaluations.as_ref() {
            diagnostics.extend(
                evaluations
                    .iter()
                    .filter_map(runtime_diagnostic_from_field_eval),
            );
        }

        if let Some(error) = self.search_error.as_ref() {
            diagnostics.push(GuiDiagnostic {
                category: DiagnosticCategory::Search,
                message: error.clone(),
                target: DiagnosticTarget::Search,
                snippet: None,
            });
        }

        diagnostics
    }

    fn diagnostic_count(&self) -> usize {
        self.schema_diagnostics.len()
            + self
                .field_evaluations
                .as_ref()
                .map(|evaluations| {
                    evaluations
                        .iter()
                        .filter(|eval| eval.error.is_some())
                        .count()
                })
                .unwrap_or(0)
            + usize::from(self.search_error.is_some())
    }

    fn activate_runtime_diagnostic(
        &mut self,
        name: &str,
        offset: u64,
        byte_len: usize,
        offset_valid: bool,
    ) {
        if let Some((group, _)) = split_field_group(name) {
            self.collapsed_field_groups.remove(group);
        }

        if !field_name_matches_filter(name, &self.field_filter_query) {
            self.field_filter_query.clear();
        }
        self.field_filter_errors_only = false;
        if offset_valid {
            self.select_field(name.to_string(), offset, byte_len);
            self.selected_field_scroll_pending = true;
        } else {
            self.selected_field_name = Some(name.to_string());
            self.selected_field_range = None;
            self.selected_field_scroll_pending = true;
            self.clear_schema_match();
            self.clear_schema_cursor_name_highlight();
        }
    }

    fn activate_schema_diagnostic(&mut self, diagnostic: &GuiDiagnostic) {
        self.active_schema_diagnostic = self
            .schema_diagnostics
            .iter()
            .position(|existing| existing == diagnostic);
    }

    fn read_bytes(&self, offset: u64, len: usize) -> Option<&[u8]> {
        self.buffer.read_bytes(offset, len).ok()
    }

    fn clear_selected_field(&mut self) {
        self.selected_field_range = None;
        self.selected_field_name = None;
        self.selected_field_scroll_pending = false;
        self.clear_schema_match();
        self.clear_schema_cursor_name_highlight();
    }

    fn select_field(&mut self, name: String, offset: u64, byte_len: usize) {
        self.select_field_with_schema_name(name.clone(), name, offset, byte_len);
    }

    fn select_field_with_schema_name(
        &mut self,
        display_name: String,
        schema_field_name: String,
        offset: u64,
        byte_len: usize,
    ) {
        eprintln!(
            "select_field: doc={:p} name={} offset=0x{:X} len={}",
            self, display_name, offset, byte_len
        );
        self.selected_field_name = Some(display_name.clone());
        self.selected_field_range = Some((offset, byte_len));
        self.clear_schema_cursor_name_highlight();
        self.update_schema_match_for_field(&display_name, &schema_field_name);

        if byte_len == 0 {
            return;
        }

        let page_end = self.hex_start_offset.saturating_add(HEX_PAGE_SIZE as u64);
        if offset < self.hex_start_offset || offset >= page_end {
            let max_start = self.size.saturating_sub(HEX_PAGE_SIZE as u64);
            let new_offset = offset.min(max_start);
            self.hex_start_offset = new_offset;
            self.hex_offset_input = format!("0x{:X}", new_offset);
        }
    }

    fn update_schema_match_for_field(&mut self, display_name: &str, schema_field_name: &str) {
        if let Some(schema_match) =
            find_best_schema_name_match(&self.schema_editor_text, display_name, schema_field_name)
        {
            self.schema_match = Some(schema_match);
            self.schema_match_scroll_pending = true;
        } else {
            self.clear_schema_match();
        }
    }

    fn select_field_from_schema(&mut self, field: ClickedField) {
        self.clear_schema_match();

        if let Some((group, _)) = split_field_group(&field.display_name) {
            self.collapsed_field_groups.remove(group);
        }

        if !field_name_matches_filter(&field.display_name, &self.field_filter_query) {
            self.field_filter_query.clear();
        }
        self.field_filter_errors_only = false;

        self.selected_field_name = Some(field.display_name);
        self.selected_field_range = field.offset_valid.then_some((field.offset, field.byte_len));
        self.selected_field_scroll_pending = true;

        if !field.offset_valid || field.byte_len == 0 {
            return;
        }

        let page_end = self.hex_start_offset.saturating_add(HEX_PAGE_SIZE as u64);
        if field.offset < self.hex_start_offset || field.offset >= page_end {
            let max_start = self.size.saturating_sub(HEX_PAGE_SIZE as u64);
            let new_offset = field.offset.min(max_start);
            self.hex_start_offset = new_offset;
            self.hex_offset_input = format!("0x{:X}", new_offset);
        }
    }

    fn activate_schema_editor_cursor_line(&mut self, line_index: usize) {
        let Some(name_entry) =
            schema_field_name_match_at_line(&self.schema_editor_text, line_index)
        else {
            self.clear_schema_cursor_name_highlight();
            self.selected_field_scroll_pending = false;
            return;
        };

        let Some(evaluations) = self.field_evaluations.as_ref() else {
            self.clear_schema_cursor_name_highlight();
            self.selected_field_scroll_pending = false;
            return;
        };

        if let Some(field) = find_best_field_for_schema_name(evaluations, &name_entry.name) {
            self.schema_cursor_name_highlight = Some(name_entry.highlight);
            self.select_field_from_schema(field);
        } else {
            self.clear_schema_cursor_name_highlight();
            self.selected_field_scroll_pending = false;
        }
    }

    fn find_search_matches(&mut self) -> Result<(), String> {
        if self.search_query.trim().is_empty() {
            return Ok(());
        }

        let query = self.search_query.clone();
        let pattern = match self.search_mode {
            SearchMode::Ascii => query.as_bytes().to_vec(),
            SearchMode::Hex => match parse_hex_pattern(&query) {
                Ok(pattern) => pattern,
                Err(err) => {
                    self.clear_active_search();
                    self.search_error = Some(err);
                    return Ok(());
                }
            },
        };
        let matches = find_byte_pattern_matches(self.buffer.as_ref(), self.size, &pattern)?;
        self.active_search_query = query;
        self.active_search_pattern_len = pattern.len();
        self.search_matches = matches;
        self.current_search_match = None;
        self.search_error = None;

        if !self.search_matches.is_empty() {
            self.select_search_match(0);
        }

        Ok(())
    }

    fn clear_search(&mut self) {
        self.search_query.clear();
        self.clear_active_search();
        self.search_error = None;
    }

    fn clear_active_search(&mut self) {
        self.active_search_query.clear();
        self.active_search_pattern_len = 0;
        self.search_matches.clear();
        self.current_search_match = None;
    }

    fn select_search_match(&mut self, index: usize) {
        if self.search_matches.is_empty() {
            self.current_search_match = None;
            return;
        }

        let index = index % self.search_matches.len();
        self.current_search_match = Some(index);

        let offset = self.search_matches[index];
        let page_end = self.hex_start_offset.saturating_add(HEX_PAGE_SIZE as u64);
        if offset < self.hex_start_offset || offset >= page_end {
            let max_start = self.size.saturating_sub(HEX_PAGE_SIZE as u64);
            let new_offset = offset.min(max_start);
            self.hex_start_offset = new_offset;
            self.hex_offset_input = format!("0x{:X}", new_offset);
        }
    }

    fn next_search_match(&mut self) {
        if self.search_matches.is_empty() {
            self.current_search_match = None;
            return;
        }

        let next = self
            .current_search_match
            .map(|index| (index + 1) % self.search_matches.len())
            .unwrap_or(0);
        self.select_search_match(next);
    }

    fn previous_search_match(&mut self) {
        if self.search_matches.is_empty() {
            self.current_search_match = None;
            return;
        }

        let previous = self
            .current_search_match
            .map(|index| {
                if index == 0 {
                    self.search_matches.len() - 1
                } else {
                    index - 1
                }
            })
            .unwrap_or(0);
        self.select_search_match(previous);
    }

    fn search_status(&self) -> Option<String> {
        if self.active_search_query.is_empty() {
            return None;
        }

        if self.search_matches.is_empty() {
            return Some("No matches".to_string());
        }

        let current = self.current_search_match.unwrap_or(0) + 1;
        Some(format!("Match {current} of {}", self.search_matches.len()))
    }
}

const MIN_HEX_BYTES_PER_ROW: usize = 16;
const MAX_HEX_BYTES_PER_ROW: usize = 64;

fn draw_hex_view(ui: &mut egui::Ui, doc: &Document, available_width: f32) {
    const SELECTED_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(255, 196, 0);
    const SELECTED_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(24, 24, 24);
    const SEARCH_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(117, 180, 255);
    const SEARCH_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(16, 24, 32);
    const ACTIVE_SEARCH_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(105, 235, 165);
    const ACTIVE_SEARCH_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(8, 28, 18);
    let bytes_per_row = adaptive_hex_bytes_per_row(ui, available_width);
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

    let visible_end = doc.hex_start_offset.saturating_add(to_show as u64);
    let selected_visible_range = doc.selected_field_range.and_then(|(start, byte_len)| {
        if byte_len == 0 {
            return None;
        }

        let end = start.saturating_add(byte_len as u64);
        let visible_start = start.max(doc.hex_start_offset);
        let visible_end = end.min(visible_end);
        (visible_start < visible_end).then_some((visible_start, visible_end))
    });
    let visible_search_ranges = visible_search_ranges(
        &doc.search_matches,
        doc.current_search_match,
        doc.active_search_pattern_len,
        doc.hex_start_offset,
        visible_end,
    );

    for row_start in (0..bytes.len()).step_by(bytes_per_row) {
        let row_end = (row_start + bytes_per_row).min(bytes.len());
        let row = &bytes[row_start..row_end];
        let mut ascii_column = String::new();

        for i in 0..bytes_per_row {
            if let Some(byte) = row.get(i) {
                let ch = if (0x20..=0x7E).contains(byte) {
                    *byte as char
                } else {
                    '.'
                };
                ascii_column.push(ch);
            }
        }

        ui.horizontal(|ui| {
            ui.monospace(format!("{:08X}:", doc.hex_start_offset + row_start as u64));

            for i in 0..bytes_per_row {
                if let Some(byte) = row.get(i) {
                    let absolute_offset = doc.hex_start_offset + row_start as u64 + i as u64;
                    let is_selected = selected_visible_range.is_some_and(|(start, end)| {
                        absolute_offset >= start && absolute_offset < end
                    });
                    let is_active_search =
                        visible_search_ranges.iter().any(|(start, end, is_active)| {
                            *is_active && absolute_offset >= *start && absolute_offset < *end
                        });
                    let is_search_match =
                        visible_search_ranges.iter().any(|(start, end, is_active)| {
                            !*is_active && absolute_offset >= *start && absolute_offset < *end
                        });

                    let mut byte_text = egui::RichText::new(format!("{byte:02X}")).monospace();
                    if is_active_search {
                        byte_text = byte_text
                            .background_color(ACTIVE_SEARCH_BYTE_BG)
                            .color(ACTIVE_SEARCH_BYTE_FG);
                    } else if is_selected {
                        byte_text = byte_text
                            .background_color(SELECTED_BYTE_BG)
                            .color(SELECTED_BYTE_FG);
                    } else if is_search_match {
                        byte_text = byte_text
                            .background_color(SEARCH_BYTE_BG)
                            .color(SEARCH_BYTE_FG);
                    }
                    ui.label(byte_text);
                } else {
                    ui.monospace("  ");
                }
            }

            ui.monospace(ascii_column);
        });
    }
}

fn adaptive_hex_bytes_per_row(ui: &egui::Ui, available_width: f32) -> usize {
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let zero_width = ui.fonts(|fonts| fonts.glyph_width(&font_id, '0'));
    let spacing = ui.spacing().item_spacing.x;
    let offset_column_width = zero_width * 9.0 + spacing;
    let per_byte_width = (zero_width * 3.0) + spacing;
    let available_for_bytes = (available_width - offset_column_width).max(0.0);
    let bytes_per_row = (available_for_bytes / per_byte_width).floor() as usize;

    bytes_per_row.clamp(MIN_HEX_BYTES_PER_ROW, MAX_HEX_BYTES_PER_ROW)
}

fn responsive_text_edit_width(ui: &egui::Ui, preferred: f32, minimum: f32) -> f32 {
    let available = ui.available_width();
    available.min(preferred).max(minimum.min(available))
}

fn clicked_outside_regions(
    ui: &egui::Ui,
    panel_rect: egui::Rect,
    protected_rects: &[egui::Rect],
) -> bool {
    let Some(click_pos) = ui.ctx().input(|input| {
        input
            .pointer
            .primary_clicked()
            .then(|| input.pointer.interact_pos())
            .flatten()
    }) else {
        return false;
    };

    panel_rect.contains(click_pos) && !protected_rects.iter().any(|rect| rect.contains(click_pos))
}

fn parse_hex_pattern(input: &str) -> Result<Vec<u8>, String> {
    let mut digits = String::new();

    for ch in input.trim().chars() {
        if ch.is_ascii_hexdigit() {
            digits.push(ch);
        } else if !ch.is_whitespace() {
            return Err(format!("Invalid hex pattern: unexpected character '{ch}'"));
        }
    }

    if digits.is_empty() {
        return Ok(Vec::new());
    }

    if !digits.len().is_multiple_of(2) {
        return Err("Invalid hex pattern: expected an even number of hex digits".to_string());
    }

    let mut bytes = Vec::with_capacity(digits.len() / 2);
    for index in (0..digits.len()).step_by(2) {
        let byte = u8::from_str_radix(&digits[index..index + 2], 16)
            .map_err(|_| "Invalid hex pattern".to_string())?;
        bytes.push(byte);
    }

    Ok(bytes)
}

fn find_byte_pattern_matches(
    buffer: &dyn FileBuffer,
    file_size: u64,
    pattern: &[u8],
) -> Result<Vec<u64>, String> {
    if pattern.is_empty() || u64::try_from(pattern.len()).unwrap_or(u64::MAX) > file_size {
        return Ok(Vec::new());
    }

    let overlap = pattern.len().saturating_sub(1);
    let read_budget = SEARCH_CHUNK_SIZE
        .checked_add(overlap)
        .ok_or_else(|| "Search query is too large".to_string())?;
    let mut matches = Vec::new();
    let mut offset = 0_u64;

    while offset < file_size {
        let remaining = file_size.saturating_sub(offset);
        let read_len = remaining.min(read_budget as u64) as usize;
        let bytes = buffer
            .read_bytes(offset, read_len)
            .map_err(|err| err.to_string())?;

        let is_last_chunk = offset.saturating_add(read_len as u64) >= file_size;
        let scan_len = if is_last_chunk {
            bytes.len()
        } else {
            bytes.len().saturating_sub(overlap)
        };
        let scan_end = scan_len.min(bytes.len().saturating_sub(pattern.len()).saturating_add(1));

        for start in 0..scan_end {
            if &bytes[start..start + pattern.len()] == pattern {
                matches.push(offset + start as u64);
            }
        }

        offset = offset.saturating_add(SEARCH_CHUNK_SIZE as u64);
    }

    Ok(matches)
}

fn visible_search_ranges(
    matches: &[u64],
    current_match: Option<usize>,
    query_len: usize,
    visible_start: u64,
    visible_end: u64,
) -> Vec<(u64, u64, bool)> {
    if query_len == 0 {
        return Vec::new();
    }

    let query_len = query_len as u64;
    let mut ranges = Vec::new();

    for (index, start) in matches.iter().copied().enumerate() {
        let end = start.saturating_add(query_len);
        if end <= visible_start {
            continue;
        }
        if start >= visible_end {
            break;
        }

        ranges.push((
            start.max(visible_start),
            end.min(visible_end),
            current_match == Some(index),
        ));
    }

    ranges
}

fn format_resolved_offset(offset: u64, offset_valid: bool) -> String {
    if offset_valid {
        format!("0x{offset:X} ({offset})")
    } else {
        "<invalid>".to_string()
    }
}

fn format_value(value: &FieldValue, byte_len: usize) -> String {
    match value {
        FieldValue::UInt(v) => format!("{v} (0x{v:X})"),
        FieldValue::Int(v) => format!("{v} (0x{v:X})"),
        FieldValue::Float(v) => format!("{v}"),
        FieldValue::Bytes(bytes) => format_bytes_preview(bytes, byte_len),
        FieldValue::Ascii(text) => format_ascii_preview(text, byte_len),
    }
}

fn format_bytes_preview(bytes: &[u8], byte_len: usize) -> String {
    let display_len = if byte_len <= MAX_DISPLAY_BYTES {
        bytes.len()
    } else {
        bytes.len().min(MAX_DISPLAY_BYTES)
    };
    let mut rendered: Vec<String> = bytes[..display_len]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect();

    if byte_len > MAX_DISPLAY_BYTES {
        rendered.push("...".to_string());
    }

    let rendered = rendered.join(" ");
    if byte_len > MAX_DISPLAY_BYTES {
        format!("{rendered} ({byte_len} bytes)")
    } else {
        rendered
    }
}

fn format_ascii_preview(text: &str, byte_len: usize) -> String {
    if byte_len <= MAX_DISPLAY_BYTES {
        return text.to_string();
    }

    let mut preview = String::new();
    let mut used_bytes = 0;
    for ch in text.chars() {
        let ch_bytes = ch.len_utf8();
        if used_bytes + ch_bytes > MAX_DISPLAY_BYTES {
            break;
        }
        preview.push(ch);
        used_bytes += ch_bytes;
    }

    format!("{preview}... ({byte_len} bytes)")
}

fn schema_error_diagnostic(
    error: &SchemaError,
    fallback_path: Option<&PathBuf>,
    fallback_source: Option<&str>,
) -> GuiDiagnostic {
    match error {
        SchemaError::Yaml { message, location } => {
            let source_location = source_location(fallback_path.cloned(), location.as_ref());
            let snippet = source_location
                .as_ref()
                .and_then(|location| schema_snippet(location, fallback_path, fallback_source));
            GuiDiagnostic {
                category: DiagnosticCategory::SchemaParse,
                message: message.clone(),
                target: DiagnosticTarget::Schema {
                    location: source_location,
                },
                snippet,
            }
        }
        SchemaError::Validation {
            message,
            path,
            location,
        } => {
            let source_location = source_location(
                path.clone().or_else(|| fallback_path.cloned()),
                location.as_ref(),
            );
            let snippet = source_location
                .as_ref()
                .and_then(|location| schema_snippet(location, fallback_path, fallback_source));
            GuiDiagnostic {
                category: DiagnosticCategory::SchemaValidation,
                message: message.clone(),
                target: DiagnosticTarget::Schema {
                    location: source_location,
                },
                snippet,
            }
        }
        SchemaError::Io { path, .. } => GuiDiagnostic {
            category: DiagnosticCategory::IncludeResolution,
            message: error.to_string(),
            target: DiagnosticTarget::Schema {
                location: Some(SourceLocation {
                    path: path.clone(),
                    line: 1,
                    column: 1,
                }),
            },
            snippet: None,
        },
        SchemaError::IncludeCycle { .. } => GuiDiagnostic {
            category: DiagnosticCategory::IncludeResolution,
            message: error.to_string(),
            target: DiagnosticTarget::Schema { location: None },
            snippet: None,
        },
    }
}

fn source_location(
    path: Option<PathBuf>,
    location: Option<&binocular_schema::error::SchemaLocation>,
) -> Option<SourceLocation> {
    let path = path?;
    let location = location?;
    Some(SourceLocation {
        path,
        line: location.line,
        column: location.column,
    })
}

fn schema_snippet(
    location: &SourceLocation,
    fallback_path: Option<&PathBuf>,
    fallback_source: Option<&str>,
) -> Option<String> {
    let source = if fallback_path.is_some_and(|path| path == &location.path) {
        fallback_source.map(str::to_string)
    } else {
        fs::read_to_string(&location.path).ok()
    }?;

    source_snippet(&source, location.line)
}

fn source_snippet(source: &str, line: usize) -> Option<String> {
    if line == 0 {
        return None;
    }

    let lines = source.lines().collect::<Vec<_>>();
    if lines.is_empty() || line > lines.len() {
        return None;
    }

    let start = line.saturating_sub(2).max(1);
    let end = (line + 2).min(lines.len());
    let mut snippet = Vec::new();

    for current_line in start..=end {
        let marker = if current_line == line { ">" } else { " " };
        let text = lines[current_line - 1];
        snippet.push(format!("{marker} {current_line:>4} | {text}"));
    }

    Some(snippet.join("\n"))
}

fn find_best_schema_name_match(
    source: &str,
    display_name: &str,
    schema_field_name: &str,
) -> Option<SchemaMatch> {
    let candidates = schema_match_candidates(display_name, schema_field_name);

    for candidate in candidates {
        if let Some(schema_match) = find_schema_name_entry(source, &candidate) {
            return Some(schema_match);
        }
    }

    None
}

fn schema_match_candidates(display_name: &str, schema_field_name: &str) -> Vec<String> {
    let normalized_display_name = strip_repeat_indexes(display_name);
    let normalized_schema_field_name = strip_repeat_indexes(schema_field_name);
    let leaf_name = normalized_display_name
        .rsplit_once('.')
        .map_or(normalized_display_name.as_str(), |(_, leaf)| leaf)
        .to_string();

    let mut candidates = Vec::new();
    for candidate in [
        normalized_display_name,
        normalized_schema_field_name,
        leaf_name,
    ] {
        if !candidate.is_empty() && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }

    candidates
}

fn strip_repeat_indexes(name: &str) -> String {
    let chars = name.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '[' {
            let index_start = index;
            index += 1;
            let digit_start = index;
            while index < chars.len() && chars[index].is_ascii_digit() {
                index += 1;
            }

            if index < chars.len() && chars[index] == ']' && index > digit_start {
                index += 1;
                continue;
            }

            output.extend(chars[index_start..index].iter());
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }

    output
}

fn find_schema_name_entry(source: &str, target_name: &str) -> Option<SchemaMatch> {
    let mut line_start_byte = 0;

    for (line_index, segment) in source.split_inclusive('\n').enumerate() {
        let line_without_lf = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line_without_lf
            .strip_suffix('\r')
            .unwrap_or(line_without_lf);

        if let Some(value) = parse_schema_name_entry(line) {
            if value.name == target_name {
                return Some(SchemaMatch {
                    line: line_index + 1,
                    line_start_byte,
                    line_end_byte: line_start_byte + line.len(),
                    snippet: line.trim().to_string(),
                });
            }
        }

        line_start_byte += segment.len();
    }

    None
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaNameEntry {
    name: String,
    name_start_byte: usize,
    name_end_byte: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct SchemaNameLineMatch {
    name: String,
    highlight: SchemaHighlight,
}

fn schema_field_name_match_at_line(source: &str, line_index: usize) -> Option<SchemaNameLineMatch> {
    let mut line_start_byte = 0;

    for (current_line_index, segment) in source.split_inclusive('\n').enumerate() {
        let line_without_lf = segment.strip_suffix('\n').unwrap_or(segment);
        let line = line_without_lf
            .strip_suffix('\r')
            .unwrap_or(line_without_lf);

        if current_line_index == line_index {
            let entry = parse_schema_name_entry(line)?;
            return Some(SchemaNameLineMatch {
                name: entry.name,
                highlight: SchemaHighlight {
                    line: line_index + 1,
                    start_byte: line_start_byte + entry.name_start_byte,
                    end_byte: line_start_byte + entry.name_end_byte,
                },
            });
        }

        line_start_byte += segment.len();
    }

    None
}

fn line_index_for_char_index(source: &str, char_index: usize) -> usize {
    let mut line_index = 0;

    for (index, ch) in source.chars().enumerate() {
        if index >= char_index {
            break;
        }

        if ch == '\n' {
            line_index += 1;
        }
    }

    line_index
}

fn parse_schema_name_entry(line: &str) -> Option<SchemaNameEntry> {
    let mut rest = line.trim_start();
    let mut rest_start_byte = line.len() - rest.len();

    if let Some(after_dash) = rest.strip_prefix('-') {
        if !after_dash.chars().next().is_some_and(char::is_whitespace) {
            return None;
        }
        let trimmed_after_dash = after_dash.trim_start();
        rest_start_byte += 1 + after_dash.len() - trimmed_after_dash.len();
        rest = trimmed_after_dash;
    }

    let after_key = rest.strip_prefix("name:")?;
    let value = after_key.trim_start();
    let value_start_byte = rest_start_byte + "name:".len() + after_key.len() - value.len();
    let (name, value_name_start, value_name_end) = parse_schema_name_value_span(value)?;
    let name = strip_repeat_indexes(&name);
    (!name.is_empty()).then_some(SchemaNameEntry {
        name,
        name_start_byte: value_start_byte + value_name_start,
        name_end_byte: value_start_byte + value_name_end,
    })
}

fn parse_schema_name_value_span(value: &str) -> Option<(String, usize, usize)> {
    let first_char = value.chars().next()?;

    if first_char == '"' || first_char == '\'' {
        let quote = first_char;
        let mut escaped = false;
        let mut name = String::new();
        let name_start = first_char.len_utf8();
        let mut name_end = value.len();

        for (offset, ch) in value[name_start..].char_indices() {
            let absolute_offset = name_start + offset;
            if escaped {
                name.push(ch);
                escaped = false;
            } else if quote == '"' && ch == '\\' {
                escaped = true;
            } else if ch == quote {
                name_end = absolute_offset;
                break;
            } else {
                name.push(ch);
            }
        }

        Some((name, name_start, name_end))
    } else {
        let name_end = value
            .split_once('#')
            .map_or(value, |(before_comment, _)| before_comment)
            .trim_end()
            .len();
        let name = value[..name_end].to_string();
        Some((name, 0, name_end))
    }
}

fn find_best_field_for_schema_name(
    evaluations: &[FieldEval],
    schema_name: &str,
) -> Option<ClickedField> {
    let normalized_schema_name = strip_repeat_indexes(schema_name);
    if normalized_schema_name.is_empty() {
        return None;
    }

    evaluations
        .iter()
        .find(|eval| normalized_leaf_name(&eval.field.name) == normalized_schema_name)
        .or_else(|| {
            evaluations
                .iter()
                .find(|eval| strip_repeat_indexes(&eval.display_name) == normalized_schema_name)
        })
        .or_else(|| {
            evaluations
                .iter()
                .find(|eval| normalized_leaf_name(&eval.display_name) == normalized_schema_name)
        })
        .map(clicked_field_from_eval)
}

fn clicked_field_from_eval(eval: &FieldEval) -> ClickedField {
    ClickedField {
        display_name: eval.display_name.clone(),
        schema_field_name: eval.field.name.clone(),
        offset: eval.resolved_offset,
        byte_len: eval.byte_len,
        offset_valid: eval.offset_valid,
    }
}

fn normalized_leaf_name(name: &str) -> String {
    let normalized = strip_repeat_indexes(name);
    normalized
        .rsplit_once('.')
        .map_or(normalized.as_str(), |(_, leaf)| leaf)
        .to_string()
}

fn runtime_diagnostic_from_field_eval(eval: &FieldEval) -> Option<GuiDiagnostic> {
    let error = eval.error.as_ref()?;
    Some(GuiDiagnostic {
        category: DiagnosticCategory::Interpretation,
        message: error.clone(),
        target: DiagnosticTarget::Field {
            name: eval.display_name.clone(),
            offset: eval.resolved_offset,
            byte_len: eval.byte_len,
            offset_valid: eval.offset_valid,
        },
        snippet: None,
    })
}

fn field_name_matches_filter(display_name: &str, query: &str) -> bool {
    let query = query.trim();
    query.is_empty() || display_name.to_lowercase().contains(&query.to_lowercase())
}

fn field_matches_filter(eval: &FieldEval, query: &str, errors_only: bool) -> bool {
    if errors_only && eval.error.is_none() {
        return false;
    }

    field_name_matches_filter(&eval.display_name, query)
}

struct FieldTableWidths {
    name: f32,
    offset: f32,
    ty: f32,
    value: f32,
    error: f32,
}

struct FieldTableChild<'a> {
    name: String,
    eval: &'a FieldEval,
}

enum FieldTableItem<'a> {
    Ungrouped(&'a FieldEval),
    Group {
        name: String,
        children: Vec<FieldTableChild<'a>>,
    },
}

impl FieldTableItem<'_> {
    fn field_count(&self) -> usize {
        match self {
            FieldTableItem::Ungrouped(_) => 1,
            FieldTableItem::Group { children, .. } => children.len(),
        }
    }
}

fn split_field_group(display_name: &str) -> Option<(&str, &str)> {
    let (group, child) = display_name.rsplit_once('.')?;
    (!group.is_empty() && !child.is_empty()).then_some((group, child))
}

fn filtered_field_items<'a>(
    evaluations: &'a [FieldEval],
    filter_query: &str,
    filter_errors_only: bool,
) -> Vec<FieldTableItem<'a>> {
    let mut items = Vec::new();

    for eval in evaluations
        .iter()
        .filter(|eval| field_matches_filter(eval, filter_query, filter_errors_only))
    {
        if let Some((group_name, child_name)) = split_field_group(&eval.display_name) {
            if let Some(FieldTableItem::Group { children, .. }) = items.iter_mut().find(|item| {
                matches!(
                    item,
                    FieldTableItem::Group { name, .. } if name == group_name
                )
            }) {
                children.push(FieldTableChild {
                    name: child_name.to_string(),
                    eval,
                });
            } else {
                items.push(FieldTableItem::Group {
                    name: group_name.to_string(),
                    children: vec![FieldTableChild {
                        name: child_name.to_string(),
                        eval,
                    }],
                });
            }
        } else {
            items.push(FieldTableItem::Ungrouped(eval));
        }
    }

    items
}

fn field_item_count(items: &[FieldTableItem<'_>]) -> usize {
    items.iter().map(FieldTableItem::field_count).sum()
}

fn draw_field_table(
    ui: &mut egui::Ui,
    items: &[FieldTableItem<'_>],
    collapsed_groups: &mut HashSet<String>,
    selected_name: Option<&str>,
    selected_range: Option<(u64, usize)>,
    scroll_selected_row: bool,
    available_width: f32,
) -> (Option<ClickedField>, Vec<egui::Rect>, bool) {
    let mut clicked_field = None;
    let mut row_rects = Vec::new();
    let mut did_scroll_to_selected_row = false;
    let spacing_x = ui.spacing().item_spacing.x;
    let name_column_width = (available_width * 0.22).clamp(120.0, 220.0);
    let offset_column_width = 104.0;
    let type_column_width = 76.0;
    let flexible_width = (available_width
        - name_column_width
        - offset_column_width
        - type_column_width
        - (spacing_x * 4.0))
        .max(320.0);
    let value_column_width = (flexible_width * 0.45).max(160.0);
    let widths = FieldTableWidths {
        name: name_column_width,
        offset: offset_column_width,
        ty: type_column_width,
        value: value_column_width,
        error: (flexible_width - value_column_width).max(160.0),
    };

    egui::Grid::new("field_evaluations")
        .striped(true)
        .show(ui, |ui| {
            ui.add_sized(
                [widths.name, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new("Name").strong()),
            );
            ui.add_sized(
                [widths.offset, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new("Offset").strong()),
            );
            ui.add_sized(
                [widths.ty, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new("Type").strong()),
            );
            ui.add_sized(
                [widths.value, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new("Value").strong()),
            );
            ui.add_sized(
                [widths.error, ui.spacing().interact_size.y],
                egui::Label::new(egui::RichText::new("Error").strong()),
            );
            ui.end_row();

            for item in items {
                match item {
                    FieldTableItem::Ungrouped(eval) => {
                        let (field, row_rect, did_scroll) = draw_field_eval_row(
                            ui,
                            eval,
                            &eval.display_name,
                            selected_name,
                            selected_range,
                            scroll_selected_row,
                            &widths,
                        );
                        if let Some(field) = field {
                            clicked_field = Some(field);
                        }
                        did_scroll_to_selected_row |= did_scroll;
                        row_rects.push(row_rect);
                    }
                    FieldTableItem::Group { name, children } => {
                        let collapsed = collapsed_groups.contains(name);
                        let (group_clicked, row_rect) =
                            draw_field_group_row(ui, name, collapsed, &widths);
                        row_rects.push(row_rect);

                        let collapsed = if group_clicked {
                            if !collapsed_groups.insert(name.clone()) {
                                collapsed_groups.remove(name);
                                false
                            } else {
                                true
                            }
                        } else {
                            collapsed
                        };

                        if !collapsed {
                            for child in children {
                                let (field, row_rect, did_scroll) = draw_field_eval_row(
                                    ui,
                                    child.eval,
                                    &format!("    {}", child.name),
                                    selected_name,
                                    selected_range,
                                    scroll_selected_row,
                                    &widths,
                                );
                                if let Some(field) = field {
                                    clicked_field = Some(field);
                                }
                                did_scroll_to_selected_row |= did_scroll;
                                row_rects.push(row_rect);
                            }
                        }
                    }
                }
            }
        });

    (clicked_field, row_rects, did_scroll_to_selected_row)
}

fn draw_field_group_row(
    ui: &mut egui::Ui,
    name: &str,
    collapsed: bool,
    widths: &FieldTableWidths,
) -> (bool, egui::Rect) {
    let marker = if collapsed { "▶" } else { "▼" };
    let mut row_clicked = false;
    let response = ui.add_sized(
        [widths.name, ui.spacing().interact_size.y],
        egui::SelectableLabel::new(
            false,
            egui::RichText::new(format!("{marker} {name}")).strong(),
        ),
    );
    row_clicked |= response.clicked();
    let mut row_rect = response.rect;

    for width in [widths.offset, widths.ty, widths.value, widths.error] {
        let response = ui.add_sized(
            [width, ui.spacing().interact_size.y],
            egui::SelectableLabel::new(false, ""),
        );
        row_clicked |= response.clicked();
        row_rect = row_rect.union(response.rect);
    }

    ui.end_row();
    (
        row_clicked,
        row_rect.expand2(egui::vec2(ui.spacing().item_spacing.x, 0.0)),
    )
}

fn draw_field_eval_row(
    ui: &mut egui::Ui,
    eval: &FieldEval,
    display_name: &str,
    selected_name: Option<&str>,
    selected_range: Option<(u64, usize)>,
    scroll_selected_row: bool,
    widths: &FieldTableWidths,
) -> (Option<ClickedField>, egui::Rect, bool) {
    let is_selected_by_range =
        selected_range.is_some_and(|selected| selected == (eval.resolved_offset, eval.byte_len));
    let is_selected_by_name =
        selected_name.is_some_and(|selected| selected == eval.display_name.as_str());
    let is_selected = is_selected_by_range || is_selected_by_name;
    let mut row_clicked = false;

    let response = ui.add_sized(
        [widths.name, ui.spacing().interact_size.y],
        egui::SelectableLabel::new(is_selected, display_name),
    );
    row_clicked |= response.clicked();
    let mut row_rect = response.rect;

    let response = ui.add_sized(
        [widths.offset, ui.spacing().interact_size.y],
        egui::SelectableLabel::new(
            is_selected,
            egui::RichText::new(format_resolved_offset(
                eval.resolved_offset,
                eval.offset_valid,
            ))
            .monospace(),
        ),
    );
    row_clicked |= response.clicked();
    row_rect = row_rect.union(response.rect);

    let response = ui.add_sized(
        [widths.ty, ui.spacing().interact_size.y],
        egui::SelectableLabel::new(is_selected, format!("{:?}", eval.field.ty)),
    );
    row_clicked |= response.clicked();
    row_rect = row_rect.union(response.rect);

    if let Some(value) = &eval.value {
        let response = ui.add_sized(
            [widths.value, ui.spacing().interact_size.y],
            egui::SelectableLabel::new(is_selected, format_value(value, eval.byte_len)),
        );
        row_clicked |= response.clicked();
        row_rect = row_rect.union(response.rect);
    } else {
        let response = ui.add_sized(
            [widths.value, ui.spacing().interact_size.y],
            egui::SelectableLabel::new(is_selected, "-"),
        );
        row_clicked |= response.clicked();
        row_rect = row_rect.union(response.rect);
    }

    if let Some(error) = &eval.error {
        let response = ui.add_sized(
            [widths.error, ui.spacing().interact_size.y],
            egui::SelectableLabel::new(
                is_selected,
                egui::RichText::new(error).color(ui.visuals().error_fg_color),
            ),
        );
        row_clicked |= response.clicked();
        row_rect = row_rect.union(response.rect);
    } else {
        let response = ui.add_sized(
            [widths.error, ui.spacing().interact_size.y],
            egui::SelectableLabel::new(is_selected, "-"),
        );
        row_clicked |= response.clicked();
        row_rect = row_rect.union(response.rect);
    }

    ui.end_row();
    let row_rect = row_rect.expand2(egui::vec2(ui.spacing().item_spacing.x, 0.0));
    let did_scroll_to_selected_row = scroll_selected_row && is_selected_by_name;
    if did_scroll_to_selected_row {
        ui.scroll_to_rect(row_rect, Some(egui::Align::Center));
    }

    if row_clicked && eval.offset_valid {
        eprintln!(
            "field row clicked: name={} offset=0x{:X} len={}",
            eval.display_name, eval.resolved_offset, eval.byte_len
        );
        (
            Some(ClickedField {
                display_name: eval.display_name.clone(),
                schema_field_name: eval.field.name.clone(),
                offset: eval.resolved_offset,
                byte_len: eval.byte_len,
                offset_valid: eval.offset_valid,
            }),
            row_rect,
            did_scroll_to_selected_row,
        )
    } else {
        (None, row_rect, did_scroll_to_selected_row)
    }
}

fn draw_field_filter_controls(ui: &mut egui::Ui, doc: &mut Document) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        ui.label("Filter:");
        let width = responsive_text_edit_width(ui, 180.0, 80.0);
        let _ = ui.add_sized(
            [width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut doc.field_filter_query),
        );
        ui.checkbox(&mut doc.field_filter_errors_only, "Errors Only");
    })
    .response
}

fn draw_document_view(
    ui: &mut egui::Ui,
    doc: &mut Document,
    left_split_fraction: &mut f32,
    right_split_fraction: &mut f32,
) {
    let panel_rect = ui.max_rect();
    let mut protected_rects = Vec::new();

    if ui.available_width() >= SIDE_BY_SIDE_MIN_WIDTH {
        draw_side_by_side_document_view(
            ui,
            doc,
            &mut protected_rects,
            left_split_fraction,
            right_split_fraction,
        );
    } else {
        draw_stacked_document_view(ui, doc, &mut protected_rects);
    }

    if clicked_outside_regions(ui, panel_rect, &protected_rects) {
        doc.clear_selected_field();
    }
}

fn draw_side_by_side_document_view(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
    left_split_fraction: &mut f32,
    right_split_fraction: &mut f32,
) {
    let available_width = ui.available_width();
    let pane_height = ui.available_height();
    let usable_width = (available_width - (SPLITTER_WIDTH * 2.0)).max(0.0);
    let (hex_width, fields_width, schema_width) =
        pane_widths(usable_width, left_split_fraction, right_split_fraction);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut hex_view_top = None;

        ui.allocate_ui_with_layout(
            egui::vec2(hex_width, pane_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(hex_width);
                hex_view_top = Some(draw_hex_pane(ui, doc, protected_rects, true).top());
            },
        );

        draw_splitter(
            ui,
            protected_rects,
            "hex_fields_splitter",
            usable_width,
            left_split_fraction,
            HEX_PANE_MIN_WIDTH,
            usable_width - FIELD_PANE_MIN_WIDTH - SCHEMA_PANE_MIN_WIDTH,
        );

        ui.allocate_ui_with_layout(
            egui::vec2(fields_width, pane_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(fields_width);
                draw_fields_pane(ui, doc, protected_rects, hex_view_top);
            },
        );

        draw_splitter(
            ui,
            protected_rects,
            "fields_schema_splitter",
            usable_width,
            right_split_fraction,
            HEX_PANE_MIN_WIDTH + FIELD_PANE_MIN_WIDTH,
            usable_width - SCHEMA_PANE_MIN_WIDTH,
        );

        ui.allocate_ui_with_layout(
            egui::vec2(schema_width, pane_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(schema_width);
                draw_schema_pane(ui, doc, protected_rects, true);
            },
        );
    });
}

fn pane_widths(
    usable_width: f32,
    left_split_fraction: &mut f32,
    right_split_fraction: &mut f32,
) -> (f32, f32, f32) {
    let min_left = HEX_PANE_MIN_WIDTH;
    let max_left = usable_width - FIELD_PANE_MIN_WIDTH - SCHEMA_PANE_MIN_WIDTH;
    let min_right = HEX_PANE_MIN_WIDTH + FIELD_PANE_MIN_WIDTH;
    let max_right = usable_width - SCHEMA_PANE_MIN_WIDTH;

    let left_split = (*left_split_fraction * usable_width).clamp(min_left, max_left);
    let right_split = (*right_split_fraction * usable_width)
        .clamp(left_split + FIELD_PANE_MIN_WIDTH, max_right)
        .clamp(min_right, max_right);

    *left_split_fraction = split_to_fraction(left_split, usable_width);
    *right_split_fraction = split_to_fraction(right_split, usable_width);

    (
        left_split,
        right_split - left_split,
        usable_width - right_split,
    )
}

fn split_to_fraction(split: f32, usable_width: f32) -> f32 {
    if usable_width > 0.0 {
        split / usable_width
    } else {
        0.0
    }
}

fn draw_splitter(
    ui: &mut egui::Ui,
    protected_rects: &mut Vec<egui::Rect>,
    _id_source: &'static str,
    usable_width: f32,
    split_fraction_value: &mut f32,
    min_split: f32,
    max_split: f32,
) {
    let height = ui.available_height();
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(SPLITTER_WIDTH, height),
        egui::Sense::click_and_drag(),
    );
    protected_rects.push(rect.expand(2.0));

    let response = response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);

    if response.dragged() && usable_width > 0.0 {
        let pointer_delta_x = ui.ctx().input(|input| input.pointer.delta().x);
        let split =
            (*split_fraction_value * usable_width + pointer_delta_x).clamp(min_split, max_split);
        *split_fraction_value = split_to_fraction(split, usable_width);
    }

    let visuals = ui.style().interact(&response);
    let center_x = rect.center().x;
    let line_top = rect.top() + 4.0;
    let line_bottom = rect.bottom() - 4.0;
    ui.painter().line_segment(
        [
            egui::pos2(center_x, line_top),
            egui::pos2(center_x, line_bottom),
        ],
        egui::Stroke::new(1.0, visuals.fg_stroke.color),
    );
}

fn draw_stacked_document_view(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
) {
    draw_hex_pane(ui, doc, protected_rects, false);

    if doc.schema.is_some() {
        ui.separator();
        draw_fields_pane(ui, doc, protected_rects, None);
        ui.separator();
        draw_schema_pane(ui, doc, protected_rects, false);
    }
}

fn draw_hex_pane(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
    fill_available_height: bool,
) -> egui::Rect {
    protected_rects.push(ui.heading(&doc.name).rect);
    protected_rects.push(ui.label(format!("Size: {}", format_size(doc.size))).rect);

    if let Some(error) = doc.last_error.as_deref() {
        let error_text = error.to_owned();
        let response = ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                ui.visuals().error_fg_color,
                egui::RichText::new(&error_text).strong(),
            );
            if ui.button("Dismiss").clicked() {
                doc.last_error = None;
                doc.last_error_is_offset = false;
            }
        });
        protected_rects.push(response.response.rect);
        ui.add_space(4.0);
    }

    protected_rects.push(draw_offset_controls(ui, doc).rect);
    protected_rects.push(draw_search_controls(ui, doc).rect);

    ui.separator();
    if let (Some(name), Some((offset, len))) =
        (doc.selected_field_name.as_deref(), doc.selected_field_range)
    {
        protected_rects.push(
            ui.label(format!("Selected: {name} @ 0x{offset:08X} (len {len})"))
                .rect,
        );
    }

    let max_height = if fill_available_height {
        ui.available_height().max(HEX_VIEW_HEIGHT)
    } else {
        HEX_VIEW_HEIGHT
    };

    let hex_view_width = ui.available_width();
    let hex_output = egui::ScrollArea::both()
        .id_source("hex_view_scroll_area")
        .max_width(hex_view_width)
        .max_height(max_height)
        .min_scrolled_width(hex_view_width)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(hex_view_width);
            draw_hex_view(ui, doc, hex_view_width);
        });
    protected_rects.push(hex_output.inner_rect.expand(4.0));
    hex_output.inner_rect
}

fn draw_offset_controls(ui: &mut egui::Ui, doc: &mut Document) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        ui.label("Go to offset:");
        let width = responsive_text_edit_width(ui, 160.0, 72.0);
        let _ = ui.add_sized(
            [width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut doc.hex_offset_input),
        );

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
                    doc.last_error =
                        Some(format!("Invalid offset: {}", doc.hex_offset_input.trim()));
                    doc.last_error_is_offset = true;
                }
            }
        }

        let max_start = doc.size.saturating_sub(HEX_PAGE_SIZE as u64);
        if ui
            .add_enabled(doc.hex_start_offset > 0, egui::Button::new("Previous page"))
            .clicked()
        {
            let new_offset = doc.hex_start_offset.saturating_sub(HEX_PAGE_SIZE as u64);
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
    })
    .response
}

fn draw_search_controls(ui: &mut egui::Ui, doc: &mut Document) -> egui::Response {
    ui.horizontal_wrapped(|ui| {
        ui.label("Search");
        ui.radio_value(&mut doc.search_mode, SearchMode::Ascii, "ASCII");
        ui.radio_value(&mut doc.search_mode, SearchMode::Hex, "Hex");

        let width = responsive_text_edit_width(ui, 240.0, 96.0);
        let _ = ui.add_sized(
            [width, ui.spacing().interact_size.y],
            egui::TextEdit::singleline(&mut doc.search_query),
        );

        if ui.button("Find").clicked() {
            match doc.find_search_matches() {
                Ok(()) => {
                    if doc.last_error_is_offset {
                        doc.last_error = None;
                        doc.last_error_is_offset = false;
                    }
                }
                Err(err) => {
                    let error = format!("Search failed: {err}");
                    doc.search_error = Some(error.clone());
                    doc.last_error = Some(error);
                    doc.last_error_is_offset = false;
                }
            }
        }

        let has_matches = !doc.search_matches.is_empty();
        if ui
            .add_enabled(has_matches, egui::Button::new("Prev"))
            .clicked()
        {
            doc.previous_search_match();
        }
        if ui
            .add_enabled(has_matches, egui::Button::new("Next"))
            .clicked()
        {
            doc.next_search_match();
        }

        let has_search_state = !doc.search_query.is_empty()
            || !doc.active_search_query.is_empty()
            || !doc.search_matches.is_empty()
            || doc.search_error.is_some();
        if ui
            .add_enabled(has_search_state, egui::Button::new("Clear"))
            .clicked()
        {
            doc.clear_search();
        }

        if let Some(status) = doc.search_status() {
            ui.label(status);
        }
        if let Some(error) = doc.search_error.as_deref() {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    })
    .response
}

fn align_cursor_to(ui: &mut egui::Ui, top: Option<f32>) {
    if let Some(top) = top {
        let spacer_height = top - ui.cursor().top();
        if spacer_height > 0.0 {
            ui.add_space(spacer_height);
        }
    }
}

fn draw_fields_pane(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
    table_top: Option<f32>,
) {
    if doc.schema.is_none() {
        return;
    }

    protected_rects.push(ui.heading("Interpreted Fields").rect);

    if let Some(schema) = doc.schema.as_ref() {
        let mut schema_label = format!(
            "Schema: {} (v{})",
            schema.schema_name, schema.schema_version
        );

        if let Some(schema_path) = doc.schema_path.as_ref() {
            if let Some(file_name) = schema_path.file_name() {
                schema_label.push_str(&format!(" - {}", file_name.to_string_lossy()));
            }
        }

        protected_rects.push(ui.label(schema_label).rect);
    }

    protected_rects.push(draw_field_filter_controls(ui, doc).rect);

    let clicked_field = if let Some(evaluations) = doc.field_evaluations.as_ref() {
        if evaluations.is_empty() {
            None
        } else {
            let field_items = filtered_field_items(
                evaluations,
                &doc.field_filter_query,
                doc.field_filter_errors_only,
            );
            let visible_count = field_item_count(&field_items);
            protected_rects.push(
                ui.label(format!(
                    "Showing {visible_count} / {} fields",
                    evaluations.len()
                ))
                .rect,
            );

            if visible_count == 0 {
                align_cursor_to(ui, table_top);
                protected_rects.push(ui.label("No matching fields").rect);
                return;
            }

            align_cursor_to(ui, table_top);
            let max_height = ui.available_height().max(120.0);
            let table_width = ui.available_width();
            let selected_name = doc.selected_field_name.clone();
            let scroll_selected_row = doc.selected_field_scroll_pending;
            let table_output = egui::ScrollArea::both()
                .id_source("field_table_scroll_area")
                .max_width(table_width)
                .max_height(max_height)
                .min_scrolled_width(table_width)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(table_width);
                    draw_field_table(
                        ui,
                        &field_items,
                        &mut doc.collapsed_field_groups,
                        selected_name.as_deref(),
                        doc.selected_field_range,
                        scroll_selected_row,
                        table_width,
                    )
                });
            protected_rects.push(table_output.inner_rect.expand(4.0));
            protected_rects.extend(table_output.inner.1);
            if table_output.inner.2 {
                doc.selected_field_scroll_pending = false;
            }
            table_output.inner.0
        }
    } else {
        None
    };

    if let Some(field) = clicked_field {
        doc.select_field_with_schema_name(
            field.display_name,
            field.schema_field_name,
            field.offset,
            field.byte_len,
        );
    }
}

fn draw_diagnostics_panel(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
) {
    let diagnostics = doc.diagnostics();
    if diagnostics.is_empty() {
        return;
    }

    ui.separator();
    let header = ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Diagnostics").strong());
        ui.label(diagnostic_summary(&diagnostics));
    });
    protected_rects.push(header.response.rect);

    let mut clicked_diagnostic = None;
    let panel_width = ui.available_width();
    let output = egui::ScrollArea::vertical()
        .id_source("diagnostics_scroll_area")
        .max_height(150.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.set_width(panel_width);
            for diagnostic in &diagnostics {
                let text = format!(
                    "{} - {}{}",
                    diagnostic.category.label(),
                    diagnostic.message,
                    diagnostic_target_suffix(diagnostic)
                );
                let response = ui.add_sized(
                    [panel_width, ui.spacing().interact_size.y],
                    egui::SelectableLabel::new(false, text),
                );
                if response.clicked() {
                    clicked_diagnostic = Some(diagnostic.clone());
                }
            }
        });
    protected_rects.push(output.inner_rect.expand(4.0));

    if let Some(diagnostic) = clicked_diagnostic {
        match &diagnostic.target {
            DiagnosticTarget::Field {
                name,
                offset,
                byte_len,
                offset_valid,
            } => doc.activate_runtime_diagnostic(name, *offset, *byte_len, *offset_valid),
            DiagnosticTarget::Schema { .. } => doc.activate_schema_diagnostic(&diagnostic),
            DiagnosticTarget::Search => {}
        }
    }

    if let Some(active_index) = doc.active_schema_diagnostic {
        if let Some(diagnostic) = doc.schema_diagnostics.get(active_index) {
            let active = ui.group(|ui| {
                if let DiagnosticTarget::Schema {
                    location: Some(location),
                } = &diagnostic.target
                {
                    ui.label(format!(
                        "{}:{}:{}",
                        location.path.display(),
                        location.line,
                        location.column
                    ));
                }
                if let Some(snippet) = diagnostic.snippet.as_ref() {
                    ui.monospace(snippet);
                } else {
                    ui.label("No source snippet available.");
                }
            });
            protected_rects.push(active.response.rect);
        }
    }
}

fn diagnostic_summary(diagnostics: &[GuiDiagnostic]) -> String {
    let mut parts = Vec::new();
    for category in [
        DiagnosticCategory::SchemaParse,
        DiagnosticCategory::SchemaValidation,
        DiagnosticCategory::IncludeResolution,
        DiagnosticCategory::Interpretation,
        DiagnosticCategory::Search,
    ] {
        let count = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.category == category)
            .count();
        if count > 0 {
            parts.push(format!("{} {count}", category.label()));
        }
    }

    format!("{} errors ({})", diagnostics.len(), parts.join(", "))
}

fn diagnostic_target_suffix(diagnostic: &GuiDiagnostic) -> String {
    match &diagnostic.target {
        DiagnosticTarget::Schema {
            location: Some(location),
        } => format!(
            " [{}:{}:{}]",
            location.path.display(),
            location.line,
            location.column
        ),
        DiagnosticTarget::Schema { location: None } => String::new(),
        DiagnosticTarget::Field { name, .. } => format!(" [{name}]"),
        DiagnosticTarget::Search => String::new(),
    }
}

fn schema_editor_layout_job(
    ui: &egui::Ui,
    text: &str,
    wrap_width: f32,
    highlighted_range: Option<(usize, usize)>,
) -> Arc<egui::Galley> {
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    let base_format = egui::TextFormat {
        font_id,
        color: text_color,
        ..Default::default()
    };
    let mut highlight_format = base_format.clone();
    highlight_format.background = egui::Color32::from_rgba_unmultiplied(255, 216, 77, 60);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;

    if let Some((highlight_start, highlight_end)) = highlighted_range {
        if highlight_start <= highlight_end
            && highlight_end <= text.len()
            && text.is_char_boundary(highlight_start)
            && text.is_char_boundary(highlight_end)
        {
            job.append(&text[..highlight_start], 0.0, base_format.clone());
            job.append(&text[highlight_start..highlight_end], 0.0, highlight_format);
            job.append(&text[highlight_end..], 0.0, base_format);
        } else {
            job.append(text, 0.0, base_format);
        }
    } else {
        job.append(text, 0.0, base_format);
    }

    ui.fonts(|fonts| fonts.layout_job(job))
}

fn draw_schema_pane(
    ui: &mut egui::Ui,
    doc: &mut Document,
    protected_rects: &mut Vec<egui::Rect>,
    fill_available_height: bool,
) {
    if fill_available_height {
        protected_rects.push(ui.max_rect());
    }

    protected_rects.push(ui.heading("Schema").rect);

    if let Some(schema_path) = doc.schema_path.as_ref() {
        let label = if let Some(file_name) = schema_path.file_name() {
            format!(
                "{} - {}",
                file_name.to_string_lossy(),
                schema_path.display()
            )
        } else {
            schema_path.display().to_string()
        };
        protected_rects.push(ui.label(label).rect);
    }

    let controls_response = ui.horizontal_wrapped(|ui| {
        let can_apply = doc.schema_path.is_some() && !doc.schema_editor_text.trim().is_empty();
        if ui
            .add_enabled(can_apply, egui::Button::new("Apply Schema"))
            .clicked()
        {
            doc.apply_schema_editor();
        }

        if ui
            .add_enabled(
                doc.schema_path.is_some(),
                egui::Button::new("Reload from Disk"),
            )
            .clicked()
        {
            doc.reload_schema_from_disk();
        }

        if doc.schema_editor_dirty {
            ui.label("modified");
        }
    });
    protected_rects.push(controls_response.response.rect);

    if let Some(error) = doc.schema_editor_error.as_deref() {
        protected_rects.push(
            ui.colored_label(
                ui.visuals().error_fg_color,
                egui::RichText::new(error).strong(),
            )
            .rect,
        );
    }

    if !doc.schema_editor_text.is_empty() || doc.schema_path.is_some() {
        if let Some(schema_match) = doc.schema_match.as_ref() {
            let status = ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("Best schema match: line {}", schema_match.line))
                        .strong(),
                );
                ui.monospace(&schema_match.snippet);
            });
            protected_rects.push(status.response.rect);
        }

        let diagnostics_reserved_height = if doc.diagnostic_count() > 0 {
            190.0
        } else {
            0.0
        };
        let max_height = if fill_available_height {
            (ui.available_height() - diagnostics_reserved_height).max(120.0)
        } else {
            HEX_VIEW_HEIGHT
        };
        let source_width = ui.available_width();
        let schema_match_highlight = doc.schema_match.as_ref().map(|schema_match| {
            (
                schema_match.line,
                schema_match.line_start_byte,
                schema_match.line_end_byte,
            )
        });
        let highlighted_range = doc
            .schema_cursor_name_highlight
            .as_ref()
            .map(|highlight| (highlight.start_byte, highlight.end_byte))
            .or_else(|| {
                schema_match_highlight.map(|(_, start_byte, end_byte)| (start_byte, end_byte))
            });
        let mut did_scroll_to_schema_match = false;
        let scroll_to_schema_match = doc.schema_match_scroll_pending;

        let source_output = egui::ScrollArea::both()
            .id_source("schema_source_scroll_area")
            .max_width(source_width)
            .max_height(max_height)
            .min_scrolled_width(source_width)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(source_width);
                if let (true, Some((line, _, _))) = (scroll_to_schema_match, schema_match_highlight)
                {
                    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
                    let content_origin = ui.cursor().min;
                    let line_top = content_origin.y + row_height * line.saturating_sub(1) as f32;
                    let target_rect = egui::Rect::from_min_size(
                        egui::pos2(content_origin.x, line_top),
                        egui::vec2(source_width, row_height),
                    );
                    ui.scroll_to_rect(target_rect, Some(egui::Align::Center));
                    did_scroll_to_schema_match = true;
                }

                let mut layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    schema_editor_layout_job(ui, text, wrap_width, highlighted_range)
                };
                let desired_rows = doc.schema_editor_text.lines().count().max(4);
                let output = egui::TextEdit::multiline(&mut doc.schema_editor_text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(source_width)
                    .desired_rows(desired_rows)
                    .min_size(egui::vec2(source_width, max_height))
                    .layouter(&mut layouter)
                    .show(ui);
                let cursor_char_index = output
                    .cursor_range
                    .map(|cursor_range| cursor_range.primary.ccursor.index);
                let cursor_moved = match (doc.schema_editor_cursor_char_index, cursor_char_index) {
                    (Some(previous), Some(current)) => previous != current,
                    _ => false,
                };
                doc.schema_editor_cursor_char_index = cursor_char_index;

                if output.response.changed() {
                    doc.schema_editor_dirty = true;
                    doc.clear_schema_match();
                    doc.clear_schema_cursor_name_highlight();
                    doc.selected_field_scroll_pending = false;
                } else if output.response.clicked() || (output.response.has_focus() && cursor_moved)
                {
                    if let Some(cursor_char_index) = cursor_char_index {
                        let line_index =
                            line_index_for_char_index(&doc.schema_editor_text, cursor_char_index);
                        doc.activate_schema_editor_cursor_line(line_index);
                    }
                }
            });
        protected_rects.push(source_output.inner_rect.expand(4.0));
        if did_scroll_to_schema_match {
            doc.schema_match_scroll_pending = false;
        }
    } else {
        protected_rects.push(ui.label("No schema loaded").rect);
    }

    draw_diagnostics_panel(ui, doc, protected_rects);
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
                let mut left_split_fraction = self.left_split_fraction;
                let mut right_split_fraction = self.right_split_fraction;
                if let Some(doc) = self.documents.get_mut(index) {
                    draw_document_view(
                        ui,
                        doc,
                        &mut left_split_fraction,
                        &mut right_split_fraction,
                    );
                    self.left_split_fraction = left_split_fraction;
                    self.right_split_fraction = right_split_fraction;
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
            let diagnostics_status = self
                .current_doc
                .and_then(|index| self.documents.get(index))
                .map(Document::diagnostic_count)
                .filter(|count| *count > 0)
                .map(|count| format!("Diagnostics: {count} errors"));

            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center).with_main_justify(true),
                |ui| {
                    ui.label(status_left);
                    if let Some(status) = diagnostics_status {
                        ui.label(status);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_doc(bytes: &[u8]) -> Document {
        Document {
            _path: PathBuf::new(),
            name: "test.bin".to_string(),
            size: bytes.len() as u64,
            buffer: Arc::new(MemoryBuffer::from_vec(bytes.to_vec())),
            schema: None,
            field_evaluations: None,
            last_error: None,
            last_error_is_offset: false,
            schema_path: None,
            schema_editor_text: String::new(),
            schema_editor_dirty: false,
            schema_editor_error: None,
            schema_diagnostics: Vec::new(),
            active_schema_diagnostic: None,
            schema_match: None,
            schema_match_scroll_pending: false,
            schema_cursor_name_highlight: None,
            schema_editor_cursor_char_index: None,
            hex_start_offset: 0,
            hex_offset_input: "0x0".to_string(),
            selected_field_range: None,
            selected_field_name: None,
            selected_field_scroll_pending: false,
            search_query: String::new(),
            search_mode: SearchMode::Ascii,
            active_search_query: String::new(),
            active_search_pattern_len: 0,
            search_matches: Vec::new(),
            current_search_match: None,
            search_error: None,
            field_filter_query: String::new(),
            field_filter_errors_only: false,
            collapsed_field_groups: HashSet::new(),
        }
    }

    fn field_eval(display_name: &str, error: Option<&str>) -> FieldEval {
        field_eval_with_schema_name(display_name, display_name, 0, error)
    }

    fn field_eval_with_schema_name(
        display_name: &str,
        schema_name: &str,
        offset: u64,
        error: Option<&str>,
    ) -> FieldEval {
        FieldEval {
            field: binocular_schema::ast::FieldDef {
                name: schema_name.to_string(),
                ty: binocular_schema::ast::FieldType::U8,
                offset: binocular_schema::ast::OffsetKind::Absolute(0),
                length: None,
                endianness: None,
                description: None,
                repeat: None,
                when: None,
            },
            display_name: display_name.to_string(),
            resolved_offset: offset,
            offset_valid: true,
            byte_len: 1,
            value: None,
            error: error.map(str::to_string),
        }
    }

    fn schema_yaml(field_name: &str, value_offset: u64) -> String {
        format!(
            r#"
schema_name: "Test"
schema_version: 1
fields:
  - name: "{field_name}"
    type: u8
    offset:
      kind: Absolute
      value: {value_offset}
"#
        )
    }

    fn unique_temp_path(prefix: &str, extension: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "binocular_gui_{prefix}_{}_{}.{}",
            std::process::id(),
            nanos,
            extension
        ))
    }

    fn evaluation_names(doc: &Document) -> Vec<String> {
        doc.field_evaluations
            .as_ref()
            .expect("document should have field evaluations")
            .iter()
            .map(|eval| eval.display_name.clone())
            .collect()
    }

    #[test]
    fn schema_name_matching_handles_quoted_and_unquoted_values() {
        let source = r#"
fields:
  - name: "magic"
  - name: 'length'
  - name: payload # inline comment
"#;

        assert_eq!(
            find_best_schema_name_match(source, "magic", "magic")
                .map(|schema_match| schema_match.line),
            Some(3)
        );
        assert_eq!(
            find_best_schema_name_match(source, "length", "length")
                .map(|schema_match| schema_match.line),
            Some(4)
        );
        assert_eq!(
            find_best_schema_name_match(source, "payload", "payload")
                .map(|schema_match| schema_match.line),
            Some(5)
        );
    }

    #[test]
    fn schema_name_matching_strips_repeat_indexes() {
        let source = r#"
fields:
  - name: field
"#;

        assert_eq!(
            find_best_schema_name_match(source, "field[0]", "field")
                .map(|schema_match| schema_match.line),
            Some(3)
        );
    }

    #[test]
    fn schema_name_matching_uses_nested_leaf_name() {
        let source = r#"
structures:
  - name: Header
    fields:
      - name: magic
"#;

        assert_eq!(
            find_best_schema_name_match(source, "header.magic", "magic")
                .map(|schema_match| schema_match.line),
            Some(5)
        );
    }

    #[test]
    fn schema_name_matching_ignores_schema_name() {
        let source = r#"
schema_name: magic
fields:
  - name: payload
"#;

        assert_eq!(
            find_best_schema_name_match(source, "magic", "magic")
                .map(|schema_match| schema_match.line),
            None
        );
    }

    #[test]
    fn schema_name_matching_chooses_first_equal_match() {
        let source = r#"
fields:
  - name: id
  - name: id
"#;

        assert_eq!(
            find_best_schema_name_match(source, "records.id", "id")
                .map(|schema_match| schema_match.line),
            Some(3)
        );
    }

    #[test]
    fn schema_field_name_detection_extracts_field_lines() {
        assert_eq!(
            parse_schema_name_entry("  - name: payload").map(|entry| entry.name),
            Some("payload".to_string())
        );
        assert_eq!(
            parse_schema_name_entry("name: \"payload\"").map(|entry| entry.name),
            Some("payload".to_string())
        );
        assert_eq!(
            parse_schema_name_entry("name: 'payload'").map(|entry| entry.name),
            Some("payload".to_string())
        );
        assert_eq!(parse_schema_name_entry("schema_name: payload"), None);
        assert_eq!(parse_schema_name_entry("  type: u8"), None);
    }

    #[test]
    fn schema_field_name_detection_uses_cursor_line() {
        let source = "schema_name: Test\nfields:\n  - name: payload\n    type: u8\n";
        let cursor_index = source
            .find("payload")
            .expect("fixture should contain payload");
        let line_index = line_index_for_char_index(source, cursor_index);

        assert_eq!(line_index, 2);
        assert_eq!(
            schema_field_name_match_at_line(source, line_index).map(|entry| entry.name),
            Some("payload".to_string())
        );
    }

    #[test]
    fn schema_field_name_detection_tracks_value_highlight_span() {
        let source = "fields:\n  - name: \"payload\"\n  - name: 'id'\n";

        let payload =
            schema_field_name_match_at_line(source, 1).expect("quoted payload line should parse");
        assert_eq!(payload.name, "payload");
        assert_eq!(
            &source[payload.highlight.start_byte..payload.highlight.end_byte],
            "payload"
        );

        let id =
            schema_field_name_match_at_line(source, 2).expect("single-quoted id line should parse");
        assert_eq!(id.name, "id");
        assert_eq!(
            &source[id.highlight.start_byte..id.highlight.end_byte],
            "id"
        );
    }

    #[test]
    fn schema_field_matching_prefers_schema_field_leaf() {
        let evaluations = vec![
            field_eval_with_schema_name("records[0].id", "id", 4, None),
            field_eval_with_schema_name("header.id", "id", 8, None),
        ];

        let field = find_best_field_for_schema_name(&evaluations, "id")
            .expect("id should match first repeated field");

        assert_eq!(field.display_name, "records[0].id");
        assert_eq!(field.offset, 4);
    }

    #[test]
    fn schema_field_matching_uses_dotted_display_name() {
        let evaluations = vec![
            field_eval_with_schema_name("header.id", "id", 4, None),
            field_eval_with_schema_name("records[0].id", "id", 8, None),
        ];

        let field = find_best_field_for_schema_name(&evaluations, "records.id")
            .expect("dotted schema name should match normalized display name");

        assert_eq!(field.display_name, "records[0].id");
        assert_eq!(field.offset, 8);
    }

    #[test]
    fn schema_field_matching_returns_none_for_missing_name() {
        let evaluations = vec![field_eval_with_schema_name("payload", "payload", 4, None)];

        assert!(find_best_field_for_schema_name(&evaluations, "missing").is_none());
    }

    #[test]
    fn schema_cursor_activation_selects_matching_field_and_reveals_row() {
        let mut doc = memory_doc(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        doc.schema_editor_text = "fields:\n  - name: length\n".to_string();
        doc.field_evaluations = Some(vec![
            field_eval_with_schema_name("header.magic", "magic", 0, None),
            field_eval_with_schema_name("header.length", "length", 3, None),
        ]);
        doc.field_filter_query = "magic".to_string();
        doc.field_filter_errors_only = true;
        doc.collapsed_field_groups.insert("header".to_string());

        doc.activate_schema_editor_cursor_line(1);

        assert_eq!(doc.selected_field_name.as_deref(), Some("header.length"));
        assert_eq!(doc.selected_field_range, Some((3, 1)));
        let highlight = doc
            .schema_cursor_name_highlight
            .as_ref()
            .expect("schema-origin selection should highlight clicked name");
        assert_eq!(
            &doc.schema_editor_text[highlight.start_byte..highlight.end_byte],
            "length"
        );
        assert!(doc.field_filter_query.is_empty());
        assert!(!doc.field_filter_errors_only);
        assert!(!doc.collapsed_field_groups.contains("header"));
        assert!(doc.selected_field_scroll_pending);
    }

    #[test]
    fn schema_cursor_activation_clears_stale_reverse_scroll_on_no_match() {
        let mut doc = memory_doc(&[0xAA]);
        doc.schema_editor_text = "fields:\n  - name: missing\n  type: u8\n".to_string();
        doc.field_evaluations = Some(vec![field_eval_with_schema_name(
            "payload", "payload", 0, None,
        )]);
        doc.selected_field_name = Some("payload".to_string());
        doc.selected_field_range = Some((0, 1));
        doc.selected_field_scroll_pending = true;

        doc.activate_schema_editor_cursor_line(1);

        assert_eq!(doc.selected_field_name.as_deref(), Some("payload"));
        assert_eq!(doc.selected_field_range, Some((0, 1)));
        assert!(doc.schema_cursor_name_highlight.is_none());
        assert!(!doc.selected_field_scroll_pending);

        doc.selected_field_scroll_pending = true;
        doc.activate_schema_editor_cursor_line(2);
        assert_eq!(doc.selected_field_name.as_deref(), Some("payload"));
        assert!(doc.schema_cursor_name_highlight.is_none());
        assert!(!doc.selected_field_scroll_pending);
    }

    #[test]
    fn selecting_field_updates_or_clears_schema_match() {
        let mut doc = memory_doc(&[0x12]);
        doc.schema_editor_text = r#"
schema_name: "Test"
fields:
  - name: magic
    type: u8
"#
        .to_string();

        doc.select_field_with_schema_name("header.magic".to_string(), "magic".to_string(), 0, 1);

        assert_eq!(doc.selected_field_name.as_deref(), Some("header.magic"));
        assert_eq!(doc.selected_field_range, Some((0, 1)));
        assert_eq!(
            doc.schema_match
                .as_ref()
                .map(|schema_match| schema_match.line),
            Some(4)
        );
        assert!(doc.schema_match_scroll_pending);

        doc.select_field_with_schema_name("missing".to_string(), "missing".to_string(), 0, 1);

        assert_eq!(doc.selected_field_name.as_deref(), Some("missing"));
        assert_eq!(doc.selected_field_range, Some((0, 1)));
        assert!(doc.schema_match.is_none());
        assert!(!doc.schema_match_scroll_pending);
    }

    #[test]
    fn apply_schema_editor_success_updates_interpretation_and_clears_editor_state() {
        let mut doc = memory_doc(&[0x12]);
        doc.schema_path = Some(unique_temp_path("apply_success", "yaml"));
        doc.schema_editor_text = schema_yaml("renamed", 0);
        doc.schema_editor_dirty = true;
        doc.schema_editor_error = Some("old error".to_string());
        doc.schema_diagnostics.push(GuiDiagnostic {
            category: DiagnosticCategory::SchemaParse,
            message: "old diagnostic".to_string(),
            target: DiagnosticTarget::Schema { location: None },
            snippet: None,
        });
        doc.active_schema_diagnostic = Some(0);
        doc.selected_field_range = Some((0, 1));
        doc.selected_field_name = Some("old".to_string());

        doc.apply_schema_editor();

        assert_eq!(evaluation_names(&doc), vec!["renamed"]);
        assert!(!doc.schema_editor_dirty);
        assert!(doc.schema_editor_error.is_none());
        assert!(doc.schema_diagnostics.is_empty());
        assert_eq!(doc.active_schema_diagnostic, None);
        assert!(doc.selected_field_range.is_none());
        assert!(doc.selected_field_name.is_none());
    }

    #[test]
    fn apply_schema_editor_failure_preserves_previous_good_interpretation() {
        let mut doc = memory_doc(&[0x12]);
        doc.schema_path = Some(unique_temp_path("apply_failure", "yaml"));
        doc.schema_editor_text = schema_yaml("good", 0);
        doc.apply_schema_editor();
        doc.selected_field_range = Some((0, 1));
        doc.selected_field_name = Some("good".to_string());

        doc.schema_editor_text = "schema_name: [".to_string();
        doc.schema_editor_dirty = true;
        doc.apply_schema_editor();

        assert_eq!(evaluation_names(&doc), vec!["good"]);
        assert!(doc.schema_editor_dirty);
        assert!(doc.schema_editor_error.is_some());
        assert_eq!(doc.selected_field_range, Some((0, 1)));
        assert_eq!(doc.selected_field_name.as_deref(), Some("good"));
        assert_eq!(doc.schema_diagnostics.len(), 1);
        assert_eq!(
            doc.schema_diagnostics[0].category,
            DiagnosticCategory::SchemaParse
        );
        assert!(matches!(
            doc.schema_diagnostics[0].target,
            DiagnosticTarget::Schema { location: Some(_) }
        ));
        assert!(doc.schema_diagnostics[0]
            .snippet
            .as_deref()
            .is_some_and(|snippet| snippet.contains(">    1 | schema_name: [")));
    }

    #[test]
    fn reload_schema_from_disk_replaces_editor_text_and_clears_error_state() {
        let path = unique_temp_path("reload_success", "yaml");
        let disk_schema = schema_yaml("from_disk", 0);
        fs::write(&path, &disk_schema).expect("failed to write schema fixture");

        let mut doc = memory_doc(&[0x12]);
        doc.schema_path = Some(path.clone());
        doc.schema_editor_text = "schema_name: [".to_string();
        doc.schema_editor_dirty = true;
        doc.schema_editor_error = Some("old error".to_string());

        doc.reload_schema_from_disk();

        assert_eq!(doc.schema_editor_text, disk_schema);
        assert_eq!(evaluation_names(&doc), vec!["from_disk"]);
        assert!(!doc.schema_editor_dirty);
        assert!(doc.schema_editor_error.is_none());
        assert!(doc.schema_diagnostics.is_empty());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn reload_schema_from_disk_failure_records_diagnostic_and_preserves_results() {
        let mut doc = memory_doc(&[0x12]);
        doc.schema_path = Some(unique_temp_path("reload_missing", "yaml"));
        doc.schema_editor_text = schema_yaml("good", 0);
        doc.apply_schema_editor();

        doc.reload_schema_from_disk();

        assert_eq!(evaluation_names(&doc), vec!["good"]);
        assert!(doc.schema_editor_error.is_some());
        assert_eq!(doc.schema_diagnostics.len(), 1);
        assert_eq!(
            doc.schema_diagnostics[0].category,
            DiagnosticCategory::IncludeResolution
        );
    }

    #[test]
    fn runtime_field_errors_are_collected_as_diagnostics() {
        let mut doc = memory_doc(&[0x12]);
        doc.field_evaluations = Some(vec![
            field_eval("header.magic", None),
            field_eval("header.length", Some("read out of bounds")),
        ]);

        let diagnostics = doc.diagnostics();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].category, DiagnosticCategory::Interpretation);
        assert!(matches!(
            &diagnostics[0].target,
            DiagnosticTarget::Field { name, .. } if name == "header.length"
        ));
    }

    #[test]
    fn activating_runtime_diagnostic_expands_group_and_clears_hiding_filters() {
        let mut doc = memory_doc(&[0x12]);
        doc.field_filter_query = "payload".to_string();
        doc.field_filter_errors_only = true;
        doc.collapsed_field_groups.insert("header".to_string());

        doc.activate_runtime_diagnostic("header.length", 0, 1, true);

        assert!(!doc.collapsed_field_groups.contains("header"));
        assert!(doc.field_filter_query.is_empty());
        assert!(!doc.field_filter_errors_only);
        assert_eq!(doc.selected_field_name.as_deref(), Some("header.length"));
        assert_eq!(doc.selected_field_range, Some((0, 1)));
    }

    fn matching_names<'a>(
        evaluations: &'a [FieldEval],
        query: &str,
        errors_only: bool,
    ) -> Vec<&'a str> {
        evaluations
            .iter()
            .filter(|eval| field_matches_filter(eval, query, errors_only))
            .map(|eval| eval.display_name.as_str())
            .collect()
    }

    fn field_item_labels(items: &[FieldTableItem<'_>]) -> Vec<String> {
        let mut labels = Vec::new();

        for item in items {
            match item {
                FieldTableItem::Ungrouped(eval) => {
                    labels.push(format!("field:{}", eval.display_name));
                }
                FieldTableItem::Group { name, children } => {
                    labels.push(format!("group:{name}"));
                    labels.extend(children.iter().map(|child| format!("child:{}", child.name)));
                }
            }
        }

        labels
    }

    #[test]
    fn field_filter_empty_query_shows_all_rows() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("packet.header.length", Some("short read")),
            field_eval("payload", None),
        ];

        assert_eq!(
            matching_names(&evaluations, "", false),
            vec!["header.magic", "packet.header.length", "payload"]
        );
    }

    #[test]
    fn field_filter_matches_name_substrings() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("packet.header.length", None),
            field_eval("payload", None),
        ];

        assert_eq!(
            matching_names(&evaluations, "header", false),
            vec!["header.magic", "packet.header.length"]
        );
    }

    #[test]
    fn field_filter_is_case_insensitive() {
        let evaluations = vec![
            field_eval("Header.Magic", None),
            field_eval("packet.HEADER.length", None),
            field_eval("payload", None),
        ];

        assert_eq!(
            matching_names(&evaluations, "header", false),
            vec!["Header.Magic", "packet.HEADER.length"]
        );
    }

    #[test]
    fn field_filter_errors_only_shows_error_rows() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("packet.header.length", Some("short read")),
            field_eval("payload", Some("invalid offset")),
        ];

        assert_eq!(
            matching_names(&evaluations, "", true),
            vec!["packet.header.length", "payload"]
        );
    }

    #[test]
    fn field_filter_query_and_errors_only_both_apply() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("packet.header.length", Some("short read")),
            field_eval("payload", Some("invalid offset")),
        ];

        assert_eq!(
            matching_names(&evaluations, "header", true),
            vec!["packet.header.length"]
        );
    }

    #[test]
    fn field_group_split_uses_final_dot() {
        assert_eq!(split_field_group("header.magic"), Some(("header", "magic")));
        assert_eq!(
            split_field_group("packet.header.magic"),
            Some(("packet.header", "magic"))
        );
        assert_eq!(
            split_field_group("records[0].id"),
            Some(("records[0]", "id"))
        );
        assert_eq!(split_field_group("payload"), None);
    }

    #[test]
    fn filtered_field_items_group_dotted_names() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("header.length", None),
            field_eval("records[0].id", None),
            field_eval("records[0].value", None),
            field_eval("payload", None),
        ];

        let items = filtered_field_items(&evaluations, "", false);

        assert_eq!(
            field_item_labels(&items),
            vec![
                "group:header",
                "child:magic",
                "child:length",
                "group:records[0]",
                "child:id",
                "child:value",
                "field:payload",
            ]
        );
        assert_eq!(field_item_count(&items), 5);
    }

    #[test]
    fn filtered_field_items_filter_by_group_or_child_name() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("header.length", None),
            field_eval("records[0].id", None),
            field_eval("payload", None),
        ];

        let group_items = filtered_field_items(&evaluations, "header", false);
        assert_eq!(
            field_item_labels(&group_items),
            vec!["group:header", "child:magic", "child:length"]
        );
        assert_eq!(field_item_count(&group_items), 2);

        let child_items = filtered_field_items(&evaluations, "magic", false);
        assert_eq!(
            field_item_labels(&child_items),
            vec!["group:header", "child:magic"]
        );
        assert_eq!(field_item_count(&child_items), 1);
    }

    #[test]
    fn filtered_field_items_errors_only_omits_empty_groups() {
        let evaluations = vec![
            field_eval("header.magic", None),
            field_eval("header.length", Some("short read")),
            field_eval("records[0].id", None),
            field_eval("payload", Some("invalid offset")),
        ];

        let items = filtered_field_items(&evaluations, "", true);

        assert_eq!(
            field_item_labels(&items),
            vec!["group:header", "child:length", "field:payload"]
        );
        assert_eq!(field_item_count(&items), 2);
    }

    #[test]
    fn parse_hex_pattern_accepts_spaced_bytes() {
        assert_eq!(
            parse_hex_pattern("DE AD BE EF").unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn parse_hex_pattern_accepts_compact_bytes() {
        assert_eq!(
            parse_hex_pattern("deadbeef").unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn parse_hex_pattern_accepts_mixed_case_and_extra_whitespace() {
        assert_eq!(
            parse_hex_pattern("  De AD  be EF  ").unwrap(),
            vec![0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn parse_hex_pattern_rejects_odd_digit_count() {
        let err = parse_hex_pattern("ABC").unwrap_err();

        assert!(err.contains("even number"));
    }

    #[test]
    fn parse_hex_pattern_rejects_invalid_characters() {
        let err = parse_hex_pattern("DE AD ZQ").unwrap_err();

        assert!(err.contains("unexpected character"));
    }

    #[test]
    fn parse_hex_pattern_empty_input_is_empty() {
        assert_eq!(parse_hex_pattern("  ").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn byte_pattern_search_finds_exact_matches() {
        let buffer = MemoryBuffer::from_vec(b"abc BIN def BIN".to_vec());

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"BIN").unwrap();

        assert_eq!(matches, vec![4, 12]);
    }

    #[test]
    fn byte_pattern_search_finds_overlapping_matches() {
        let buffer = MemoryBuffer::from_vec(b"AAAA".to_vec());

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"AA").unwrap();

        assert_eq!(matches, vec![0, 1, 2]);
    }

    #[test]
    fn byte_pattern_search_reports_no_matches() {
        let buffer = MemoryBuffer::from_vec(b"BINOCULAR".to_vec());

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"HELLO").unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn byte_pattern_search_empty_pattern_is_empty() {
        let buffer = MemoryBuffer::from_vec(b"BINOCULAR".to_vec());

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"").unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn byte_pattern_search_is_case_sensitive_for_ascii_bytes() {
        let buffer = MemoryBuffer::from_vec(b"bin BIN".to_vec());

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"BIN").unwrap();

        assert_eq!(matches, vec![4]);
    }

    #[test]
    fn byte_pattern_search_finds_match_crossing_chunk_boundary() {
        let mut bytes = vec![b'.'; SEARCH_CHUNK_SIZE + 8];
        bytes[SEARCH_CHUNK_SIZE - 2..SEARCH_CHUNK_SIZE + 3].copy_from_slice(b"HELLO");
        let buffer = MemoryBuffer::from_vec(bytes);

        let matches = find_byte_pattern_matches(&buffer, buffer.file_size(), b"HELLO").unwrap();

        assert_eq!(matches, vec![(SEARCH_CHUNK_SIZE - 2) as u64]);
    }

    #[test]
    fn document_find_replaces_previous_results() {
        let mut doc = memory_doc(b"BIN HELLO BIN");
        doc.search_query = "BIN".to_string();
        doc.find_search_matches().unwrap();
        assert_eq!(doc.search_matches, vec![0, 10]);
        assert_eq!(doc.current_search_match, Some(0));

        doc.search_query = "HELLO".to_string();
        doc.find_search_matches().unwrap();

        assert_eq!(doc.search_matches, vec![4]);
        assert_eq!(doc.current_search_match, Some(0));
        assert_eq!(doc.active_search_query, "HELLO");
        assert_eq!(doc.active_search_pattern_len, 5);
        assert_eq!(doc.search_error, None);
    }

    #[test]
    fn document_editing_query_does_not_rescan() {
        let mut doc = memory_doc(b"BIN HELLO BIN");
        doc.search_query = "BIN".to_string();
        doc.find_search_matches().unwrap();

        doc.search_query = "HELLO".to_string();

        assert_eq!(doc.search_matches, vec![0, 10]);
        assert_eq!(doc.active_search_query, "BIN");
        assert_eq!(doc.active_search_pattern_len, 3);
        assert_eq!(doc.search_status().as_deref(), Some("Match 1 of 2"));
    }

    #[test]
    fn document_hex_search_finds_byte_patterns() {
        let mut doc = memory_doc(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0xDE, 0xAD, 0xBE, 0xEF]);
        doc.search_mode = SearchMode::Hex;
        doc.search_query = "DE AD BE EF".to_string();

        doc.find_search_matches().unwrap();

        assert_eq!(doc.search_matches, vec![0, 5]);
        assert_eq!(doc.current_search_match, Some(0));
        assert_eq!(doc.active_search_query, "DE AD BE EF");
        assert_eq!(doc.active_search_pattern_len, 4);
        assert_eq!(doc.search_error, None);
    }

    #[test]
    fn document_hex_search_compact_query_uses_byte_length_for_highlights() {
        let mut doc = memory_doc(&[0xDE, 0xAD, 0xBE, 0xEF]);
        doc.search_mode = SearchMode::Hex;
        doc.search_query = "deadbeef".to_string();

        doc.find_search_matches().unwrap();

        assert_eq!(doc.search_matches, vec![0]);
        assert_eq!(doc.active_search_pattern_len, 4);
        assert_eq!(doc.search_status().as_deref(), Some("Match 1 of 1"));
    }

    #[test]
    fn document_hex_search_reports_no_matches() {
        let mut doc = memory_doc(&[0xDE, 0xAD, 0xBE, 0xEF]);
        doc.search_mode = SearchMode::Hex;
        doc.search_query = "FF".to_string();

        doc.find_search_matches().unwrap();

        assert!(doc.search_matches.is_empty());
        assert_eq!(doc.current_search_match, None);
        assert_eq!(doc.active_search_pattern_len, 1);
        assert_eq!(doc.search_status().as_deref(), Some("No matches"));
        assert_eq!(doc.search_error, None);
    }

    #[test]
    fn document_invalid_hex_search_clears_stale_results() {
        let mut doc = memory_doc(b"BIN HELLO BIN");
        doc.search_query = "BIN".to_string();
        doc.find_search_matches().unwrap();
        assert_eq!(doc.search_matches, vec![0, 10]);

        doc.search_mode = SearchMode::Hex;
        doc.search_query = "DE AD ZQ".to_string();
        doc.find_search_matches().unwrap();

        assert!(doc.search_matches.is_empty());
        assert_eq!(doc.current_search_match, None);
        assert_eq!(doc.active_search_query, "");
        assert_eq!(doc.active_search_pattern_len, 0);
        assert_eq!(doc.search_status(), None);
        assert!(doc
            .search_error
            .as_deref()
            .is_some_and(|error| error.contains("unexpected character")));
        let diagnostics = doc.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].category, DiagnosticCategory::Search);

        doc.clear_search();

        assert!(doc.diagnostics().is_empty());
    }
}
