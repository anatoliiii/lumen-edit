//! Non-text views: an image viewer tab (raster formats), live SVG preview,
//! and CSV/TSV tables.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Vec2, load::Bytes, pos2, vec2};
use egui_extras::{Column, TableBuilder};

use super::{App, Tab};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Raster,
    Svg,
    Table,
}

pub fn kind(path: &str) -> Option<Media> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    Some(match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" => Media::Raster,
        "svg" => Media::Svg,
        "csv" | "tsv" => Media::Table,
        _ => return None,
    })
}

pub struct ImageTab {
    pub path: String,
    pub bytes: Arc<[u8]>,
    uri: String,
    /// `None` = fit to the window.
    zoom: Option<f32>,
}

impl ImageTab {
    pub fn new(path: String, bytes: Vec<u8>) -> Self {
        let uri = bytes_uri(&path, &bytes);
        Self { path, bytes: bytes.into(), uri, zoom: None }
    }

    pub fn uri(&self) -> String {
        self.uri.clone()
    }

    /// Takes new content after the file changed on disk.
    pub fn replace(&mut self, ctx: &egui::Context, bytes: Vec<u8>) {
        ctx.forget_image(&self.uri);
        self.uri = bytes_uri(&self.path, &bytes);
        self.bytes = bytes.into();
    }
}

/// egui caches textures by uri, so the uri names the content: a file
/// replaced on disk gets a new texture instead of the old one.
pub fn bytes_uri(path: &str, bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    format!("bytes://{hash:016x}/{}", path.trim_start_matches('/'))
}

/// Light/dark squares behind transparent images.
fn checkerboard(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(rect, 0.0, Color32::from_gray(58));
    let cell = 10.0;
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left() + if row % 2 == 0 { 0.0 } else { cell };
        while x < rect.right() {
            let square = egui::Rect::from_min_max(pos2(x, y), pos2((x + cell).min(rect.right()), (y + cell).min(rect.bottom())));
            painter.rect_filled(square, 0.0, Color32::from_gray(74));
            x += cell * 2.0;
        }
        y += cell;
        row += 1;
    }
}

fn human_size(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        t!("{n1} MB", n1 = format!("{:.1}", bytes as f64 / 1048576.0))
    } else if bytes >= 1024 {
        t!("{n1} KB", n1 = format!("{:.1}", bytes as f64 / 1024.0))
    } else {
        t!("{bytes} B", bytes = bytes)
    }
}

/// Draws an image from bytes with zoom controls; shared by raster tabs and
/// the SVG preview. Returns nothing: zoom state lives in `zoom`.
fn image_view(ui: &mut egui::Ui, uri: String, bytes: Bytes, file_size: usize, zoom: &mut Option<f32>, title: &str) {
    let image = egui::Image::from_bytes(uri, bytes);
    let native = match image.load_for_size(ui.ctx(), Vec2::splat(4096.0)) {
        Ok(egui::load::TexturePoll::Ready { texture }) => Some(texture.size),
        Ok(egui::load::TexturePoll::Pending { size }) => size,
        Err(error) => {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(t!("Could not show the image: {error}", error = error)).color(theme::ERROR));
            });
            return;
        }
    };

    // Toolbar.
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(RichText::new(title).strong());
        if let Some(size) = native {
            ui.label(
                RichText::new(format!("{}×{} px · {}", size.x as u32, size.y as u32, human_size(file_size)))
                    .small()
                    .color(theme::MUTED_TEXT),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(10.0);
            if ui.small_button("+").clicked() {
                *zoom = Some((zoom.unwrap_or(1.0) * 1.25).min(16.0));
            }
            let label = match zoom {
                Some(z) => format!("{:.0}%", *z * 100.0),
                None => t!("fit").into(),
            };
            ui.label(RichText::new(label).small().color(theme::MUTED_TEXT));
            if ui.small_button("−").clicked() {
                *zoom = Some((zoom.unwrap_or(1.0) / 1.25).max(0.05));
            }
            if ui.selectable_label(zoom.is_none(), t!("fit")).clicked() {
                *zoom = None;
            }
            if ui.selectable_label(*zoom == Some(1.0), "100%").clicked() {
                *zoom = Some(1.0);
            }
        });
    });

    let Some(native) = native else {
        ui.spinner();
        return;
    };
    let area = ui.available_rect_before_wrap();
    if ui.rect_contains_pointer(area) {
        let delta = ui.input(|i| i.zoom_delta());
        if delta != 1.0 {
            *zoom = Some((zoom.unwrap_or(1.0) * delta).clamp(0.05, 16.0));
        }
    }
    let fit = ((area.width() - 32.0) / native.x).min((area.height() - 32.0) / native.y).clamp(0.01, 1.0);
    let scale = zoom.unwrap_or(fit);
    let size = native * scale;
    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        let available = ui.available_size();
        let offset = vec2(((available.x - size.x) / 2.0).max(16.0), ((available.y - size.y) / 2.0).max(16.0));
        let (rect, _) = ui.allocate_exact_size(size + offset * 2.0, Sense::hover());
        let image_rect = egui::Rect::from_min_size(rect.min + offset, size);
        checkerboard(ui.painter(), image_rect);
        image.fit_to_exact_size(size).paint_at(ui, image_rect);
        ui.painter().rect_stroke(image_rect, CornerRadius::ZERO, egui::Stroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);
    });
}

pub fn show_image(ui: &mut egui::Ui, app: &mut App, index: usize) {
    let Some(Tab::Image(tab)) = app.tabs.get_mut(index) else { return };
    let uri = tab.uri();
    let bytes = Bytes::Shared(tab.bytes.clone());
    let size = tab.bytes.len();
    let title = crate::host::file_name(&tab.path).to_string();
    ui.add_space(6.0);
    image_view(ui, uri, bytes, size, &mut tab.zoom, &title);
}

/// Live preview of the SVG being edited.
pub fn show_svg(ui: &mut egui::Ui, doc: &mut super::document::Document) {
    let uri = format!("bytes://{}?v{}.svg", doc.path, doc.buffer.version);
    if doc.media_uri.as_deref() != Some(uri.as_str())
        && let Some(old) = doc.media_uri.replace(uri.clone())
    {
        ui.ctx().forget_image(&old);
    }
    let text = doc.buffer.text();
    let size = text.len();
    let title = doc.name().to_string();
    ui.add_space(6.0);
    image_view(ui, uri, Bytes::from(text.into_bytes()), size, &mut doc.media_zoom, &title);
}

/// Splits CSV/TSV text into rows, honoring quotes (with `""` escapes and
/// newlines inside quoted fields). The delimiter is guessed from the header.
pub fn parse_csv(text: &str, path: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let delimiter = if path.to_ascii_lowercase().ends_with(".tsv") {
        '\t'
    } else {
        [',', ';', '\t', '|'].into_iter().max_by_key(|d| first.matches(*d).count()).unwrap_or(',')
    };
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if field.is_empty() => quoted = true,
            c if c == delimiter && !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

pub fn show_table(ui: &mut egui::Ui, doc: &mut super::document::Document) {
    let version = doc.buffer.version;
    if doc.csv.as_ref().is_none_or(|(v, _)| *v != version) {
        let rows = parse_csv(&doc.buffer.text(), &doc.path);
        doc.csv = Some((version, Arc::new(rows)));
    }
    let rows = doc.csv.as_ref().map(|(_, rows)| rows.clone()).unwrap_or_default();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        ui.label(
            RichText::new(t!("{n1} rows × {n2} columns", n1 = rows.len().saturating_sub(1), n2 = columns)).small().color(theme::MUTED_TEXT),
        );
    });
    if rows.is_empty() || columns == 0 {
        ui.label(RichText::new(t!("Empty table")).color(theme::MUTED_TEXT));
        return;
    }
    let header = &rows[0];
    // Width from the widest of the first rows, within reason.
    let char_w = 7.5;
    let widths: Vec<f32> = (0..columns)
        .map(|c| {
            let longest = rows.iter().take(200).map(|r| r.get(c).map(|s| s.chars().count()).unwrap_or(0)).max().unwrap_or(4);
            (longest as f32 * char_w + 20.0).clamp(60.0, 360.0)
        })
        .collect();
    let numbers_width = (rows.len().to_string().len() as f32 * 8.0 + 16.0).max(36.0);
    egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
        egui::ScrollArea::horizontal().auto_shrink([false, false]).show(ui, |ui| {
            let mut table =
                TableBuilder::new(ui).striped(true).resizable(true).auto_shrink([false, false]).column(Column::exact(numbers_width));
            for width in &widths {
                table = table.column(Column::initial(*width).at_least(40.0).clip(true));
            }
            table
                .header(24.0, |mut row| {
                    row.col(|ui| {
                        ui.label(RichText::new("#").small().color(theme::FAINT_TEXT));
                    });
                    for c in 0..columns {
                        row.col(|ui| {
                            ui.add(egui::Label::new(RichText::new(header.get(c).map(String::as_str).unwrap_or("")).strong()).truncate());
                        });
                    }
                })
                .body(|body| {
                    body.rows(21.0, rows.len() - 1, |mut row| {
                        let index = row.index() + 1;
                        let record = &rows[index];
                        row.col(|ui| {
                            ui.label(RichText::new(index.to_string()).small().monospace().color(theme::FAINT_TEXT));
                        });
                        for c in 0..columns {
                            row.col(|ui| {
                                let value = record.get(c).map(String::as_str).unwrap_or("");
                                // Numbers right-aligned, like spreadsheets.
                                let numeric = !value.is_empty() && value.trim().parse::<f64>().is_ok();
                                let text = RichText::new(value.replace('\n', " ⏎ ")).monospace().size(12.5);
                                if numeric {
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        ui.add(egui::Label::new(text.color(Color32::from_rgb(209, 154, 102))).truncate());
                                    });
                                } else {
                                    ui.add(egui::Label::new(text).truncate()).on_hover_text(value);
                                }
                            });
                        }
                    });
                });
        });
    });
}

/// Opens a raster image file in a viewer tab.
pub fn open_image_tab(app: &mut App, path: String, bytes: Vec<u8>) {
    let insert_at = app.active_tab.map(|i| i + 1).unwrap_or(app.tabs.len()).min(app.tabs.len());
    app.tabs.insert(insert_at, Tab::Image(Box::new(ImageTab::new(path, bytes))));
    app.active_tab = Some(insert_at);
}

/// Image bytes loaded from the project host for Markdown previews.
#[derive(Clone)]
pub enum ImageEntry {
    Loading,
    /// Bytes and their content uri (see [`bytes_uri`]).
    Ready(Arc<[u8]>, String),
    Failed(String),
}

/// Resolves `relative` against `base_dir`, folding `.` and `..`.
pub fn resolve_path(base_dir: &str, relative: &str) -> String {
    let joined =
        if crate::host::is_absolute(relative) { relative.to_string() } else { format!("{}/{relative}", base_dir.trim_end_matches('/')) };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_with_quotes_and_newlines() {
        let text = "name,note,qty\n\"Smith, J\",\"says \"\"hi\"\"\",3\nx,\"multi\nline\",4\n";
        let rows = parse_csv(text, "a.csv");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec!["Smith, J", "says \"hi\"", "3"]);
        assert_eq!(rows[2][1], "multi\nline");
    }

    #[test]
    fn csv_guesses_semicolon_and_tsv() {
        assert_eq!(parse_csv("a;b\n1;2\n", "x.csv")[1], vec!["1", "2"]);
        assert_eq!(parse_csv("a\tb\n1\t2", "x.tsv")[1], vec!["1", "2"]);
    }

    #[test]
    fn resolves_relative_paths() {
        assert_eq!(resolve_path("/p/docs", "img/a.png"), "/p/docs/img/a.png");
        assert_eq!(resolve_path("/p/docs", "../assets/./b.svg"), "/p/assets/b.svg");
        assert_eq!(resolve_path("/p/docs", "/abs/c.png"), "/abs/c.png");
    }

    #[test]
    fn kinds() {
        assert_eq!(kind("/a/logo.PNG"), Some(Media::Raster));
        assert_eq!(kind("/a/icon.svg"), Some(Media::Svg));
        assert_eq!(kind("/a/data.tsv"), Some(Media::Table));
        assert_eq!(kind("/a/main.rs"), None);
    }
}
