//! Project-wide search and replace (runs on the host: agent, ripgrep or grep).

use codium_shared::fsops::{SearchMatch, SearchQuery};
use eframe::egui::{self, Align2, FontId, RichText, Sense, TextFormat, pos2, text::LayoutJob, vec2};

use super::{App, Msg, Tab, util};
use crate::{host, theme};

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub replace: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub include: String,
    pub show_replace: bool,
    pub results: Vec<SearchMatch>,
    pub truncated: bool,
    pub running: bool,
    pub error: Option<String>,
    generation: u64,
    pub focus: bool,
    pub pending_run: bool,
    collapsed: std::collections::HashSet<String>,
    last_query: Option<(String, bool, bool, bool)>,
    /// When the query or options last changed (search-as-you-type).
    changed_at: Option<std::time::Instant>,
    seen: (String, bool, bool, bool, String),
}

impl SearchState {
    pub fn on_results(&mut self, generation: u64, result: Result<(Vec<SearchMatch>, bool), String>) {
        if generation != self.generation {
            return;
        }
        self.running = false;
        match result {
            Ok((matches, truncated)) => {
                let include = self.include.trim().to_lowercase();
                self.results = if include.is_empty() {
                    matches
                } else {
                    let parts: Vec<String> = include.split(',').map(|p| p.trim().trim_start_matches('*').to_string()).filter(|p| !p.is_empty()).collect();
                    matches.into_iter().filter(|m| parts.iter().any(|p| m.path.to_lowercase().contains(p))).collect()
                };
                self.truncated = truncated;
                self.error = None;
            }
            Err(error) => {
                self.results.clear();
                self.error = Some(error);
            }
        }
    }
}

pub fn start(app: &mut App) {
    let Some(workspace) = &mut app.workspace else { return };
    let search = &mut workspace.search;
    if search.query.is_empty() {
        search.results.clear();
        return;
    }
    search.generation += 1;
    search.running = true;
    search.last_query = Some((search.query.clone(), search.regex, search.case_sensitive, search.whole_word));
    let generation = search.generation;
    let query = SearchQuery {
        pattern: search.query.clone(),
        regex: search.regex,
        case_sensitive: search.case_sensitive,
        whole_word: search.whole_word,
        max_results: 10_000,
    };
    let host = workspace.host.clone();
    let root = workspace.root.clone();
    app.bus.spawn(move || Msg::SearchResults { generation, result: host.search(&root, &query) });
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(workspace) = &mut app.workspace else { return };
    let search = &mut workspace.search;
    let mut run = std::mem::take(&mut search.pending_run);
    let mut replace_all = false;
    let mut open: Option<(String, usize, usize, usize)> = None;

    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.label(RichText::new("ПОИСК").small().strong().color(theme::MUTED_TEXT));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.small_button(if search.show_replace { "⏷" } else { "⏵" }).on_hover_text("Замена").clicked() {
                search.show_replace = !search.show_replace;
            }
            let input = ui.add(egui::TextEdit::singleline(&mut search.query).hint_text("Искать").desired_width(ui.available_width()));
            if search.focus {
                input.request_focus();
                search.focus = false;
            }
            if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run = true;
            }
        });
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            run |= ui.toggle_value(&mut search.case_sensitive, "Aa").on_hover_text("Учитывать регистр").changed();
            run |= ui.toggle_value(&mut search.whole_word, "ab").on_hover_text("Слово целиком").changed();
            run |= ui.toggle_value(&mut search.regex, ".*").on_hover_text("Регулярное выражение").changed();
        });
        if search.show_replace {
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.add(egui::TextEdit::singleline(&mut search.replace).hint_text("Заменить").desired_width(ui.available_width() - 70.0));
                if ui.button("Все").on_hover_text("Заменить во всех найденных файлах").clicked() {
                    replace_all = true;
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add_space(22.0);
            let input = ui.add(egui::TextEdit::singleline(&mut search.include).hint_text("Файлы (напр. .rs, src/)").desired_width(ui.available_width()));
            if input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                run = true;
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if search.running {
                ui.spinner();
                ui.label(RichText::new("Поиск…").small().color(theme::MUTED_TEXT));
            } else if let Some(error) = &search.error {
                ui.label(RichText::new(error).small().color(theme::ERROR));
            } else if search.last_query.is_some() {
                let files = {
                    let mut paths: Vec<&str> = search.results.iter().map(|m| m.path.as_str()).collect();
                    paths.dedup();
                    paths.len()
                };
                let more = if search.truncated { "+" } else { "" };
                ui.label(RichText::new(format!("{}{more} совпадений в {files} файлах", search.results.len())).small().color(theme::MUTED_TEXT));
            }
        });
    });

    // Search as you type, after a short pause.
    let current = (search.query.clone(), search.regex, search.case_sensitive, search.whole_word, search.include.clone());
    if current != search.seen {
        search.seen = current;
        search.changed_at = Some(std::time::Instant::now());
    }
    if let Some(at) = search.changed_at {
        let pause = std::time::Duration::from_millis(300);
        if at.elapsed() >= pause {
            search.changed_at = None;
            if search.query.chars().count() >= 2 || search.regex && !search.query.is_empty() {
                run = true;
            } else if search.query.is_empty() {
                search.results.clear();
                search.last_query = None;
            }
        } else {
            ui.ctx().request_repaint_after(pause - at.elapsed());
        }
    }

    let root = workspace.root.clone();
    let row_h = 20.0;
    // Flatten into file headers and match rows.
    let mut rows: Vec<(Option<usize>, String)> = Vec::new();
    let mut last_path = None;
    for (index, m) in search.results.iter().enumerate() {
        if last_path != Some(&m.path) {
            rows.push((None, m.path.clone()));
            last_path = Some(&m.path);
        }
        if !search.collapsed.contains(&m.path) {
            rows.push((Some(index), m.path.clone()));
        }
    }
    let mut toggle = None;
    egui::ScrollArea::vertical().id_salt("search_results").auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (index, path) in &rows[range] {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
            }
            match index {
                None => {
                    let collapsed = search.collapsed.contains(path);
                    let (icon, color) = util::file_icon(path);
                    ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, if collapsed { "⏵" } else { "⏷" }, FontId::proportional(10.0), theme::MUTED_TEXT);
                    ui.painter().text(rect.left_center() + vec2(26.0, 0.0), Align2::CENTER_CENTER, icon, FontId::monospace(11.0), color);
                    let name = host::file_name(path);
                    let dir = path.strip_suffix(name).unwrap_or("").trim_end_matches('/');
                    let galley = ui.painter().layout_no_wrap(name.to_string(), FontId::proportional(13.0), theme::TEXT);
                    let w = galley.size().x;
                    ui.painter().galley(rect.left_center() + vec2(36.0, -galley.size().y / 2.0), galley, theme::TEXT);
                    ui.painter().text(rect.left_center() + vec2(42.0 + w, 0.0), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme::FAINT_TEXT);
                    if response.clicked() {
                        toggle = Some(path.clone());
                    }
                }
                Some(i) => {
                    let m = &search.results[*i];
                    let mut job = LayoutJob::default();
                    let text = m.text.trim_start();
                    let trimmed = m.text.len() - text.len();
                    let start = (m.start as usize).saturating_sub(trimmed).min(text.len());
                    let end = (m.end as usize).saturating_sub(trimmed).clamp(start, text.len());
                    let (start, end) = if text.is_char_boundary(start) && text.is_char_boundary(end) { (start, end) } else { (0, 0) };
                    let format = |color, background| TextFormat { font_id: FontId::proportional(12.5), color, background, ..Default::default() };
                    job.append(&text[..start], 0.0, format(theme::MUTED_TEXT, eframe::egui::Color32::TRANSPARENT));
                    job.append(&text[start..end], 0.0, format(theme::TEXT, theme::MATCH_HIGHLIGHT));
                    job.append(&text[end..], 0.0, format(theme::MUTED_TEXT, eframe::egui::Color32::TRANSPARENT));
                    let galley = ui.painter().layout_job(job);
                    let painter = ui.painter_at(rect);
                    painter.text(pos2(rect.left() + 46.0, rect.center().y), Align2::RIGHT_CENTER, (m.line + 1).to_string(), FontId::monospace(11.0), theme::FAINT_TEXT);
                    painter.galley(pos2(rect.left() + 54.0, rect.center().y - galley.size().y / 2.0), galley, theme::TEXT);
                    if response.clicked() {
                        let col = m.text[..(m.start as usize).min(m.text.len())].chars().count();
                        let len = m.text.get(m.start as usize..m.end as usize).map(|t| t.chars().count()).unwrap_or(0);
                        open = Some((host::join(&root, &m.path), m.line as usize, col, len));
                    }
                }
            }
        }
    });
    if let Some(path) = toggle
        && !search.collapsed.remove(&path)
    {
        search.collapsed.insert(path);
    }
    if run {
        start(app);
    }
    if replace_all {
        replace_everywhere(app);
    }
    if let Some((path, line, col, len)) = open {
        // Select the match, like VS Code.
        app.with_doc(
            path,
            Box::new(move |app, result| {
                if let Ok(index) = result {
                    app.activate_tab(index);
                    if let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) {
                        let anchor = doc.buffer.char_at(line, col);
                        doc.buffer.set_selections(vec![crate::text::Selection { anchor, head: anchor + len }]);
                        doc.view.reveal = true;
                        doc.view.reveal_center = true;
                        doc.view.focus_request = true;
                    }
                }
            }),
        );
    }
}

/// Replaces matches in open documents (undoable) and in other files on disk.
fn replace_everywhere(app: &mut App) {
    let Some(workspace) = &app.workspace else { return };
    let search = &workspace.search;
    let query = SearchQuery {
        pattern: search.query.clone(),
        regex: search.regex,
        case_sensitive: search.case_sensitive,
        whole_word: search.whole_word,
        max_results: 0,
    };
    let regex = match codium_shared::fsops::build_regex(&query) {
        Ok(regex) => regex,
        Err(error) => {
            app.notify(error, true);
            return;
        }
    };
    let replacement = search.replace.clone();
    let root = workspace.root.clone();
    let mut paths: Vec<String> = search.results.iter().map(|m| host::join(&root, &m.path)).collect();
    paths.dedup();
    let host = workspace.host.clone();

    let mut on_disk = Vec::new();
    for path in paths {
        if let Some(doc) = app.tabs.iter_mut().find_map(|t| match t {
            Tab::Doc(d) if d.path == path => Some(d),
            _ => None,
        }) {
            let text = doc.buffer.rope.to_string();
            let edits: Vec<_> = regex
                .find_iter(&text)
                .map(|m| {
                    let start = doc.buffer.rope.byte_to_char(m.start());
                    let end = doc.buffer.rope.byte_to_char(m.end());
                    let new = if query.regex { regex.replace(m.as_str(), replacement.as_str()).into_owned() } else { replacement.clone() };
                    (start..end, new)
                })
                .collect();
            doc.buffer.apply_edits(edits);
        } else {
            on_disk.push(path);
        }
    }
    let is_regex = query.regex;
    app.bus.spawn(move || {
        let mut count = 0;
        for path in on_disk {
            let Ok(data) = host.read_file(&path) else { continue };
            let text = String::from_utf8_lossy(&data);
            let new = if is_regex { regex.replace_all(&text, replacement.as_str()) } else { regex.replace_all(&text, regex::NoExpand(&replacement)) };
            if new != text {
                if let Err(error) = host.write_file(&path, new.as_bytes()) {
                    return Msg::WorkspaceEditDone(Err(format!("{path}: {error}")));
                }
                count += 1;
            }
        }
        Msg::WorkspaceEditDone(Ok(count))
    });
    if let Some(workspace) = &mut app.workspace {
        workspace.search.results.clear();
        workspace.search.last_query = None;
    }
}
