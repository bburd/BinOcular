use std::{fs, path::PathBuf, sync::Arc};

use binocular_core::buffer::{FileBuffer, MemoryBuffer, MmapBuffer};
use binocular_core::interpret::{interpret_schema, FieldEval, FieldValue};
use binocular_schema::ast::Schema;
use binocular_schema::parser::parse_schema_file;
use eframe::egui;

const HEX_PAGE_SIZE: usize = 1024;
const HEX_VIEW_HEIGHT: f32 = 300.0;
const SIDE_BY_SIDE_MIN_WIDTH: f32 = 760.0;
const HEX_PANE_WIDTH_FRACTION: f32 = 0.60;
const MMAP_THRESHOLD_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DISPLAY_BYTES: usize = 256;
const SEARCH_CHUNK_SIZE: usize = 64 * 1024;

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
    selected_field_range: Option<(u64, usize)>,
    selected_field_name: Option<String>,
    search_query: String,
    active_search_query: String,
    search_matches: Vec<u64>,
    current_search_match: Option<usize>,
    field_filter_query: String,
    field_filter_errors_only: bool,
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

        let schema = match parse_schema_file(&path) {
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
        doc.clear_selected_field();
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
            selected_field_range: None,
            selected_field_name: None,
            search_query: String::new(),
            active_search_query: String::new(),
            search_matches: Vec::new(),
            current_search_match: None,
            field_filter_query: String::new(),
            field_filter_errors_only: false,
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

        let schema = match parse_schema_file(&schema_path) {
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
        doc.clear_selected_field();
    }
}

impl Document {
    fn read_bytes(&self, offset: u64, len: usize) -> Option<&[u8]> {
        self.buffer.read_bytes(offset, len).ok()
    }

    fn clear_selected_field(&mut self) {
        self.selected_field_range = None;
        self.selected_field_name = None;
    }

    fn select_field(&mut self, name: String, offset: u64, byte_len: usize) {
        eprintln!(
            "select_field: doc={:p} name={} offset=0x{:X} len={}",
            self, name, offset, byte_len
        );
        self.selected_field_name = Some(name);
        self.selected_field_range = Some((offset, byte_len));

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

    fn find_search_matches(&mut self) -> Result<(), String> {
        if self.search_query.is_empty() {
            return Ok(());
        }

        let query = self.search_query.clone();
        let matches = find_ascii_matches(self.buffer.as_ref(), self.size, query.as_bytes())?;
        self.active_search_query = query;
        self.search_matches = matches;
        self.current_search_match = None;

        if !self.search_matches.is_empty() {
            self.select_search_match(0);
        }

        Ok(())
    }

    fn clear_search(&mut self) {
        self.search_query.clear();
        self.active_search_query.clear();
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

fn draw_hex_view(ui: &mut egui::Ui, doc: &Document) {
    const BYTES_PER_ROW: usize = 16;
    const SELECTED_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(255, 196, 0);
    const SELECTED_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(24, 24, 24);
    const SEARCH_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(117, 180, 255);
    const SEARCH_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(16, 24, 32);
    const ACTIVE_SEARCH_BYTE_BG: egui::Color32 = egui::Color32::from_rgb(105, 235, 165);
    const ACTIVE_SEARCH_BYTE_FG: egui::Color32 = egui::Color32::from_rgb(8, 28, 18);
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
        doc.active_search_query.len(),
        doc.hex_start_offset,
        visible_end,
    );

    for row_start in (0..bytes.len()).step_by(BYTES_PER_ROW) {
        let row_end = (row_start + BYTES_PER_ROW).min(bytes.len());
        let row = &bytes[row_start..row_end];
        let mut ascii_column = String::new();

        for i in 0..BYTES_PER_ROW {
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

            for i in 0..BYTES_PER_ROW {
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

fn find_ascii_matches(
    buffer: &dyn FileBuffer,
    file_size: u64,
    query: &[u8],
) -> Result<Vec<u64>, String> {
    if query.is_empty() || u64::try_from(query.len()).unwrap_or(u64::MAX) > file_size {
        return Ok(Vec::new());
    }

    let overlap = query.len().saturating_sub(1);
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
        let scan_end = scan_len.min(bytes.len().saturating_sub(query.len()).saturating_add(1));

        for start in 0..scan_end {
            if &bytes[start..start + query.len()] == query {
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

fn field_matches_filter(eval: &FieldEval, query: &str, errors_only: bool) -> bool {
    if errors_only && eval.error.is_none() {
        return false;
    }

    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    eval.display_name
        .to_lowercase()
        .contains(&query.to_lowercase())
}

fn draw_field_table(
    ui: &mut egui::Ui,
    evaluations: &[FieldEval],
    selected_range: Option<(u64, usize)>,
    filter_query: &str,
    filter_errors_only: bool,
) -> (Option<(String, u64, usize)>, Vec<egui::Rect>) {
    let mut clicked_field = None;
    let mut row_rects = Vec::new();

    egui::Grid::new("field_evaluations")
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Name");
            ui.strong("Offset");
            ui.strong("Type");
            ui.strong("Value");
            ui.strong("Error");
            ui.end_row();

            for eval in evaluations
                .iter()
                .filter(|eval| field_matches_filter(eval, filter_query, filter_errors_only))
            {
                let is_selected = selected_range
                    .is_some_and(|selected| selected == (eval.resolved_offset, eval.byte_len));
                let mut row_clicked = false;

                let response = ui.selectable_label(is_selected, &eval.display_name);
                row_clicked |= response.clicked();
                let mut row_rect = response.rect;

                let response = ui.selectable_label(
                    is_selected,
                    egui::RichText::new(format_resolved_offset(
                        eval.resolved_offset,
                        eval.offset_valid,
                    ))
                    .monospace(),
                );
                row_clicked |= response.clicked();
                row_rect = row_rect.union(response.rect);

                let response = ui.selectable_label(is_selected, format!("{:?}", eval.field.ty));
                row_clicked |= response.clicked();
                row_rect = row_rect.union(response.rect);

                if let Some(value) = &eval.value {
                    let response =
                        ui.selectable_label(is_selected, format_value(value, eval.byte_len));
                    row_clicked |= response.clicked();
                    row_rect = row_rect.union(response.rect);
                } else {
                    let response = ui.selectable_label(is_selected, "-");
                    row_clicked |= response.clicked();
                    row_rect = row_rect.union(response.rect);
                }

                if let Some(error) = &eval.error {
                    let response = ui.selectable_label(
                        is_selected,
                        egui::RichText::new(error).color(ui.visuals().error_fg_color),
                    );
                    row_clicked |= response.clicked();
                    row_rect = row_rect.union(response.rect);
                } else {
                    let response = ui.selectable_label(is_selected, "-");
                    row_clicked |= response.clicked();
                    row_rect = row_rect.union(response.rect);
                }

                if row_clicked && eval.offset_valid {
                    eprintln!(
                        "field row clicked: name={} offset=0x{:X} len={}",
                        eval.display_name, eval.resolved_offset, eval.byte_len
                    );
                    clicked_field = Some((
                        eval.display_name.clone(),
                        eval.resolved_offset,
                        eval.byte_len,
                    ));
                }

                row_rects.push(row_rect.expand2(egui::vec2(ui.spacing().item_spacing.x, 0.0)));

                ui.end_row();
            }
        });

    (clicked_field, row_rects)
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

fn draw_document_view(ui: &mut egui::Ui, doc: &mut Document) {
    let panel_rect = ui.max_rect();
    let mut protected_rects = Vec::new();

    if ui.available_width() >= SIDE_BY_SIDE_MIN_WIDTH {
        draw_side_by_side_document_view(ui, doc, &mut protected_rects);
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
) {
    let available_width = ui.available_width();
    let pane_gap = ui.spacing().item_spacing.x;
    let pane_height = ui.available_height();
    let left_width = ((available_width - pane_gap) * HEX_PANE_WIDTH_FRACTION).max(0.0);
    let right_width = (available_width - pane_gap - left_width).max(0.0);

    // TODO: This is the future insertion point for a schema/YAML investigation pane.
    ui.horizontal(|ui| {
        let mut hex_view_top = None;

        ui.allocate_ui_with_layout(
            egui::vec2(left_width, pane_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(left_width);
                hex_view_top = Some(draw_hex_pane(ui, doc, protected_rects, true).top());
            },
        );

        ui.allocate_ui_with_layout(
            egui::vec2(right_width, pane_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(right_width);
                draw_fields_pane(ui, doc, protected_rects, hex_view_top);
            },
        );
    });
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

    let hex_output = egui::ScrollArea::both()
        .id_source("hex_view_scroll_area")
        .max_height(max_height)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            draw_hex_view(ui, doc);
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
                    doc.last_error = Some(format!("Search failed: {err}"));
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
            || !doc.search_matches.is_empty();
        if ui
            .add_enabled(has_search_state, egui::Button::new("Clear"))
            .clicked()
        {
            doc.clear_search();
        }

        if let Some(status) = doc.search_status() {
            ui.label(status);
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
            let visible_count = evaluations
                .iter()
                .filter(|eval| {
                    field_matches_filter(
                        eval,
                        &doc.field_filter_query,
                        doc.field_filter_errors_only,
                    )
                })
                .count();
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
            let table_output = egui::ScrollArea::both()
                .id_source("field_table_scroll_area")
                .max_height(max_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    draw_field_table(
                        ui,
                        evaluations,
                        doc.selected_field_range,
                        &doc.field_filter_query,
                        doc.field_filter_errors_only,
                    )
                });
            protected_rects.extend(table_output.inner.1);
            table_output.inner.0
        }
    } else {
        None
    };

    if let Some((name, offset, byte_len)) = clicked_field {
        doc.select_field(name, offset, byte_len);
    }
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
                    draw_document_view(ui, doc);
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
            hex_start_offset: 0,
            hex_offset_input: "0x0".to_string(),
            selected_field_range: None,
            selected_field_name: None,
            search_query: String::new(),
            active_search_query: String::new(),
            search_matches: Vec::new(),
            current_search_match: None,
            field_filter_query: String::new(),
            field_filter_errors_only: false,
        }
    }

    fn field_eval(display_name: &str, error: Option<&str>) -> FieldEval {
        FieldEval {
            field: binocular_schema::ast::FieldDef {
                name: display_name.to_string(),
                ty: binocular_schema::ast::FieldType::U8,
                offset: binocular_schema::ast::OffsetKind::Absolute(0),
                length: None,
                endianness: None,
                description: None,
                repeat: None,
                when: None,
            },
            display_name: display_name.to_string(),
            resolved_offset: 0,
            offset_valid: true,
            byte_len: 1,
            value: None,
            error: error.map(str::to_string),
        }
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
    fn ascii_search_finds_exact_matches() {
        let buffer = MemoryBuffer::from_vec(b"abc BIN def BIN".to_vec());

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"BIN").unwrap();

        assert_eq!(matches, vec![4, 12]);
    }

    #[test]
    fn ascii_search_finds_overlapping_matches() {
        let buffer = MemoryBuffer::from_vec(b"AAAA".to_vec());

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"AA").unwrap();

        assert_eq!(matches, vec![0, 1, 2]);
    }

    #[test]
    fn ascii_search_reports_no_matches() {
        let buffer = MemoryBuffer::from_vec(b"BINOCULAR".to_vec());

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"HELLO").unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn ascii_search_empty_query_is_empty() {
        let buffer = MemoryBuffer::from_vec(b"BINOCULAR".to_vec());

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"").unwrap();

        assert!(matches.is_empty());
    }

    #[test]
    fn ascii_search_is_case_sensitive() {
        let buffer = MemoryBuffer::from_vec(b"bin BIN".to_vec());

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"BIN").unwrap();

        assert_eq!(matches, vec![4]);
    }

    #[test]
    fn ascii_search_finds_match_crossing_chunk_boundary() {
        let mut bytes = vec![b'.'; SEARCH_CHUNK_SIZE + 8];
        bytes[SEARCH_CHUNK_SIZE - 2..SEARCH_CHUNK_SIZE + 3].copy_from_slice(b"HELLO");
        let buffer = MemoryBuffer::from_vec(bytes);

        let matches = find_ascii_matches(&buffer, buffer.file_size(), b"HELLO").unwrap();

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
    }

    #[test]
    fn document_editing_query_does_not_rescan() {
        let mut doc = memory_doc(b"BIN HELLO BIN");
        doc.search_query = "BIN".to_string();
        doc.find_search_matches().unwrap();

        doc.search_query = "HELLO".to_string();

        assert_eq!(doc.search_matches, vec![0, 10]);
        assert_eq!(doc.active_search_query, "BIN");
        assert_eq!(doc.search_status().as_deref(), Some("Match 1 of 2"));
    }
}
