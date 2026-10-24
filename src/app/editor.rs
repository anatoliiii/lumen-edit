//! The code editor widget: paints only visible lines and handles keyboard,
//! mouse and the popups (completion, hover, find, rename, code actions).

use std::{
    ops::Range,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Event, FontId, Id, Key, Modifiers, Pos2, Rect, RichText, Sense, Shape, Stroke, TextFormat, Vec2,
    pos2, text::LayoutJob, vec2,
};

use super::{
    Command,
    commands::Scope,
    document::Document,
    keymap::{self, Chord, ClipboardAction},
    util,
};
use crate::{
    git::LineChange,
    lsp::{self, CompletionItem, Severity},
    text::{Selection, char_col_for_display, display_col},
    theme,
};

const TEXT_PAD: f32 = 10.0;

pub enum EditorAction {
    Completion {
        trigger: Option<String>,
    },
    Hover(usize),
    SignatureHelp,
    GotoDefinition,
    References,
    Rename(String),
    CodeActions,
    ApplyCodeAction(usize),
    Format,
    /// Quote the selection to the pi agent, optionally with an instruction.
    AskAgent(Option<String>),
}

pub struct Completion {
    pub items: Vec<CompletionItem>,
    pub filtered: Vec<(usize, Vec<u32>)>,
    pub selected: usize,
    /// Start of the word being completed.
    pub anchor: usize,
    scroll_to_selected: bool,
}

impl Completion {
    pub fn new(items: Vec<CompletionItem>, anchor: usize) -> Self {
        Self { items, filtered: Vec::new(), selected: 0, anchor, scroll_to_selected: true }
    }

    fn refilter(&mut self, prefix: &str) {
        let mut fuzzy = util::Fuzzy::default();
        let mut scored: Vec<(u32, usize, Vec<u32>)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                let (score, indices) = fuzzy.score(&item.filter_text, prefix)?;
                // Highlight positions refer to the label when it equals the filter text.
                let indices = if item.filter_text == item.label { indices } else { Vec::new() };
                Some((score, i, indices))
            })
            .collect();
        if prefix.is_empty() {
            scored.sort_by(|a, b| self.items[a.1].sort_text.cmp(&self.items[b.1].sort_text));
        } else {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| self.items[a.1].sort_text.cmp(&self.items[b.1].sort_text)));
        }
        self.filtered = scored.into_iter().map(|(_, i, idx)| (i, idx)).collect();
        self.selected = 0;
        self.scroll_to_selected = true;
    }
}

pub struct HoverPopup {
    pub text: String,
    pub anchor: usize,
    pub pos: Pos2,
}

#[derive(Default)]
pub struct FindState {
    pub query: String,
    pub replace: String,
    pub show_replace: bool,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub matches: Vec<Range<usize>>,
    pub current: usize,
    pub error: Option<String>,
    computed_for: Option<(u64, String, bool, bool, bool)>,
    pub focus_query: bool,
}

pub struct CodeActionsPopup {
    pub titles: Vec<String>,
    pub selected: usize,
}

pub struct RenamePopup {
    pub text: String,
    pub focus: bool,
}

#[derive(Clone, Copy)]
struct Conflict {
    start: usize,
    middle: usize,
    end: usize,
}

#[derive(Default)]
pub struct ViewState {
    pub scroll: Vec2,
    pub reveal: bool,
    pub reveal_center: bool,
    pub completion: Option<Completion>,
    pub ghost: Option<crate::inline_completion::Ghost>,
    pub ai_manual: bool,
    pub next_edit_requested: bool,
    pub next_edit_preview: Option<crate::inline_completion::next_edit::Preview>,
    pub ai_rejected: bool,
    pub ai_accepted: u64,
    pub hover: Option<HoverPopup>,
    pub signature: Option<String>,
    pub find: Option<FindState>,
    pub code_actions: Option<CodeActionsPopup>,
    pub rename: Option<RenamePopup>,
    pub focus_request: bool,
    last_click: Option<(Instant, usize, u8)>,
    drag_mode: Option<(Selection, u8)>,
    hover_candidate: Option<(usize, Instant, bool)>,
    content_width: f32,
    conflicts: Vec<Conflict>,
    conflicts_version: u64,
    scrollbar_drag: Option<f32>,
}

impl ViewState {
    pub fn shift_positions(&mut self, start: usize, old_end: usize, new_end: usize) {
        let delta = new_end as isize - old_end as isize;
        let shift = |p: usize| {
            if p >= old_end {
                (p as isize + delta) as usize
            } else if p > start {
                start
            } else {
                p
            }
        };
        if let Some(c) = &mut self.completion {
            c.anchor = shift(c.anchor);
        }
        self.hover = None;
        self.code_actions = None;
        if let Some(find) = &mut self.find {
            find.computed_for = None;
        }
        self.conflicts_version = u64::MAX;
    }

    pub fn open_find(&mut self, selection: Option<String>, replace: bool) {
        let find = self.find.get_or_insert_with(FindState::default);
        if let Some(text) = selection.filter(|t| !t.is_empty() && !t.contains('\n')) {
            find.query = text;
            find.computed_for = None;
        }
        find.show_replace = replace;
        find.focus_query = true;
    }
}

pub struct EditorSettings<'a> {
    pub font_size: f32,
    pub show_whitespace: bool,
    pub inline_blame: bool,
    pub completion_triggers: &'a [String],
    pub signature_triggers: &'a [String],
    pub has_lsp: bool,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

pub fn show(ui: &mut egui::Ui, doc: &mut Document, settings: &EditorSettings, actions: &mut Vec<EditorAction>) {
    let font = FontId::monospace(settings.font_size);
    let (char_w, row_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
    let line_h = (row_h * 1.4).round();
    let rect = ui.available_rect_before_wrap();
    ui.allocate_rect(rect, Sense::hover());
    let id = Id::unique(("editor", doc.id));

    let line_count = doc.buffer.line_count();
    let digits = line_count.to_string().len().max(3) as f32;
    let gutter_w = (digits * char_w + 30.0).round();
    let scrollbar_w = 14.0;
    let gutter_rect = Rect::from_min_max(rect.min, pos2(rect.left() + gutter_w, rect.bottom()));
    let text_rect = Rect::from_min_max(pos2(gutter_rect.right(), rect.top()), pos2(rect.right() - scrollbar_w, rect.bottom()));
    let scrollbar_rect = Rect::from_min_max(pos2(text_rect.right(), rect.top()), rect.max);
    let visible_lines = ((text_rect.height() / line_h).floor() as usize).max(1);

    let response = ui.interact(gutter_rect.union(text_rect), id, Sense::click_and_drag());
    if doc.view.focus_request {
        response.request_focus();
        doc.view.focus_request = false;
    }
    let focused = response.has_focus();
    if focused {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(id, egui::EventFilter { tab: true, horizontal_arrows: true, vertical_arrows: true, escape: true })
        });
    }

    // ------------------------------------------------------------ keyboard
    if focused {
        handle_keyboard(ui, doc, settings, actions, visible_lines);
    }
    if doc.view.ghost.as_ref().is_some_and(|g| !g.valid(doc.id, &doc.buffer)) {
        doc.view.ghost = None;
    }
    doc.absorb_edits();
    // Edits above may have removed lines; everything below uses the new count.
    let line_count = doc.buffer.line_count();

    // --------------------------------------------------------------- mouse
    let tab = doc.buffer.tab_size;
    let pointer = ui.input(|i| i.pointer.clone());
    let modifiers = ui.input(|i| i.modifiers);
    let virtual_layout = ghost_layout(doc);
    let char_at_pos = |doc: &Document, pos: Pos2| -> usize {
        let row = ((pos.y - text_rect.top() + doc.view.scroll.y) / line_h).floor().max(0.0) as usize;
        if virtual_layout.in_ghost_rows(row) {
            return doc.buffer.primary().head;
        }
        let line = virtual_layout.line_at(row);
        if line >= doc.buffer.line_count() {
            return doc.buffer.len_chars();
        }
        let display = ((pos.x - text_rect.left() - TEXT_PAD + doc.view.scroll.x) / char_w).round().max(0.0) as usize;
        let col = char_col_for_display(&doc.buffer.line_text(line), display, tab);
        doc.buffer.char_at(line, col)
    };

    if response.hovered()
        && pointer.primary_pressed()
        && let Some(pos) = pointer.interact_pos()
        && !scrollbar_rect.contains(pos)
    {
        response.request_focus();
        let at = char_at_pos(doc, pos);
        let now = Instant::now();
        let count = match doc.view.last_click {
            Some((time, last_at, count)) if now.duration_since(time) < Duration::from_millis(450) && last_at == at => count % 3 + 1,
            _ => 1,
        };
        doc.view.last_click = Some((now, at, count));
        doc.view.completion = None;
        doc.view.hover = None;
        doc.view.code_actions = None;
        if gutter_rect.contains(pos) {
            // Clicking line numbers selects lines.
            let (line, _) = doc.buffer.line_col(at);
            let start = doc.buffer.char_at(line, 0);
            let end = if line + 1 < line_count { doc.buffer.char_at(line + 1, 0) } else { doc.buffer.len_chars() };
            doc.buffer.set_selections(vec![Selection { anchor: start, head: end }]);
            doc.view.drag_mode = Some((Selection { anchor: start, head: end }, 3));
        } else if modifiers.alt {
            doc.buffer.add_selection(Selection::cursor(at));
            doc.view.drag_mode = None;
        } else if modifiers.shift {
            doc.buffer.set_cursor(at, true);
            doc.view.drag_mode = Some((Selection { anchor: doc.buffer.primary().anchor, head: at }, 1));
        } else if modifiers.command && count == 1 {
            doc.buffer.set_cursor(at, false);
            actions.push(EditorAction::GotoDefinition);
        } else {
            let selection = match count {
                2 => {
                    let word = doc.buffer.word_at(at);
                    if word.is_empty() { Selection::cursor(at) } else { Selection { anchor: word.start, head: word.end } }
                }
                3 => {
                    let (line, _) = doc.buffer.line_col(at);
                    let start = doc.buffer.char_at(line, 0);
                    let end = if line + 1 < line_count { doc.buffer.char_at(line + 1, 0) } else { doc.buffer.len_chars() };
                    Selection { anchor: start, head: end }
                }
                _ => Selection::cursor(at),
            };
            doc.buffer.set_selections(vec![selection]);
            doc.view.drag_mode = Some((selection, count));
        }
    }
    if let Some((origin, mode)) = doc.view.drag_mode {
        if pointer.primary_down() {
            if let Some(pos) = pointer.interact_pos().or(pointer.hover_pos()) {
                let at = char_at_pos(doc, pos);
                let selection = match mode {
                    2 => {
                        let word = doc.buffer.word_at(at);
                        if at < origin.start() {
                            Selection { anchor: origin.end(), head: word.start.min(at) }
                        } else {
                            Selection { anchor: origin.start(), head: word.end.max(at) }
                        }
                    }
                    3 => {
                        let (line, _) = doc.buffer.line_col(at);
                        if at < origin.start() {
                            Selection { anchor: origin.end(), head: doc.buffer.char_at(line, 0) }
                        } else {
                            let end = if line + 1 < line_count { doc.buffer.char_at(line + 1, 0) } else { doc.buffer.len_chars() };
                            Selection { anchor: origin.start(), head: end }
                        }
                    }
                    _ => Selection { anchor: origin.anchor, head: at },
                };
                let last = doc.buffer.selections.len() - 1;
                doc.buffer.selections[last] = selection;
                // Auto-scroll while dragging past the edges.
                if pos.y > text_rect.bottom() - 4.0 {
                    doc.view.scroll.y += line_h;
                } else if pos.y < text_rect.top() + 4.0 {
                    doc.view.scroll.y -= line_h;
                }
                ui.ctx().request_repaint();
            }
        } else {
            doc.view.drag_mode = None;
        }
    }

    // Scroll wheel.
    if response.hovered() || ui.rect_contains_pointer(scrollbar_rect) {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta != Vec2::ZERO && !modifiers.command {
            doc.view.scroll -= delta;
            doc.view.hover = None;
        }
    }

    // Vertical scrollbar drag.
    let virtual_layout = ghost_layout(doc);
    let content_h = (line_count + virtual_layout.extra + visible_lines.saturating_sub(3)) as f32 * line_h;
    let max_scroll_y = (content_h - text_rect.height()).max(0.0);
    let sb_response = ui.interact(scrollbar_rect, id.with("vscroll"), Sense::click_and_drag());
    if let Some(pos) = sb_response.interact_pointer_pos() {
        let fraction = ((pos.y - scrollbar_rect.top()) / scrollbar_rect.height()).clamp(0.0, 1.0);
        let thumb_h = (scrollbar_rect.height() * text_rect.height() / (content_h + text_rect.height())).max(24.0);
        if sb_response.drag_started() || sb_response.clicked() {
            let thumb_top = scrollbar_rect.top() + (doc.view.scroll.y / max_scroll_y.max(1.0)) * (scrollbar_rect.height() - thumb_h);
            let grab = pos.y - thumb_top;
            doc.view.scrollbar_drag = Some(if (0.0..=thumb_h).contains(&grab) { grab } else { thumb_h / 2.0 });
        }
        let grab = doc.view.scrollbar_drag.unwrap_or(thumb_h / 2.0);
        let top = (pos.y - grab - scrollbar_rect.top()) / (scrollbar_rect.height() - thumb_h).max(1.0);
        let _ = fraction;
        doc.view.scroll.y = top.clamp(0.0, 1.0) * max_scroll_y;
    }
    if !sb_response.dragged() {
        doc.view.scroll_bar_release();
    }

    // Keep the cursor visible after keyboard actions.
    if doc.view.reveal {
        let (line, col) = doc.buffer.line_col(doc.buffer.primary().head);
        let y = line as f32 * line_h;
        if doc.view.reveal_center {
            doc.view.scroll.y = y - text_rect.height() / 3.0;
        } else if y < doc.view.scroll.y {
            doc.view.scroll.y = y;
        } else if y + line_h > doc.view.scroll.y + text_rect.height() {
            doc.view.scroll.y = y + line_h - text_rect.height();
        }
        let x = display_col(&doc.buffer.line_text(line), col, tab) as f32 * char_w + TEXT_PAD;
        let width = text_rect.width();
        if x < doc.view.scroll.x + TEXT_PAD {
            doc.view.scroll.x = (x - TEXT_PAD * 4.0).max(0.0);
        } else if x > doc.view.scroll.x + width - char_w * 4.0 {
            doc.view.scroll.x = x - width + char_w * 6.0;
        }
        doc.view.reveal = false;
        doc.view.reveal_center = false;
    }
    doc.view.scroll.y = doc.view.scroll.y.clamp(0.0, max_scroll_y);
    let max_scroll_x = (doc.view.content_width - text_rect.width() + char_w * 8.0).max(0.0);
    doc.view.scroll.x = doc.view.scroll.x.clamp(0.0, max_scroll_x);

    // ------------------------------------------------------------- paint
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::EDITOR_BACKGROUND);
    let text_painter = ui.painter_at(text_rect);
    let gutter_painter = ui.painter_at(gutter_rect);

    let first_row = (doc.view.scroll.y / line_h).floor() as usize;
    let first_line = virtual_layout.line_at(first_row).min(line_count.saturating_sub(1));
    let last_line = (virtual_layout.line_at(first_row + visible_lines + 2) + 1).min(line_count);
    let y_of = |line: usize, scroll_y: f32| text_rect.top() + virtual_layout.row(line) as f32 * line_h - scroll_y;
    let scroll = doc.view.scroll;
    let x_of_display = |display: usize| text_rect.left() + TEXT_PAD + display as f32 * char_w - scroll.x;
    let first_display = (scroll.x / char_w).floor() as usize;
    let last_display = first_display + (text_rect.width() / char_w).ceil() as usize + 2;

    let primary = doc.buffer.primary();
    let (cursor_line, _) = doc.buffer.line_col(primary.head);
    let single_cursor = doc.buffer.selections.len() == 1 && primary.is_empty();

    if single_cursor && cursor_line >= first_line && cursor_line < last_line {
        let y = y_of(cursor_line, scroll.y);
        painter.rect_filled(
            Rect::from_min_size(pos2(rect.left(), y), vec2(text_rect.right() - rect.left(), line_h)),
            0.0,
            theme::CURRENT_LINE,
        );
    }

    // Merge conflict regions.
    update_conflicts(doc);
    for conflict in &doc.view.conflicts {
        for (from, to, color) in [
            (conflict.start, conflict.middle, Color32::from_rgba_unmultiplied(80, 170, 110, 28)),
            (conflict.middle, conflict.end, Color32::from_rgba_unmultiplied(80, 140, 230, 28)),
        ] {
            let top = y_of(from, scroll.y);
            let bottom = y_of(to + 1, scroll.y);
            text_painter.rect_filled(Rect::from_x_y_ranges(text_rect.x_range(), top..=bottom), 0.0, color);
        }
    }

    // Line texts are needed by several passes.
    let lines: Vec<(usize, String)> = (first_line..last_line).map(|l| (l, doc.buffer.line_text(l))).collect();

    // Word occurrences and find matches.
    let occurrence = if primary.is_empty() {
        let word = doc.buffer.word_at(primary.head);
        (word.len() >= 2).then(|| doc.buffer.slice(word))
    } else {
        let text = doc.buffer.slice(primary.range());
        (text.chars().count() >= 2 && !text.contains('\n') && text.trim() == text).then_some(text)
    };
    if let Some(word) = &occurrence
        && doc.view.find.is_none()
    {
        let whole_word = word.chars().all(is_word);
        for (line, text) in &lines {
            for (byte, _) in text.match_indices(word.as_str()) {
                let start_col = text[..byte].chars().count();
                let end_col = start_col + word.chars().count();
                if whole_word {
                    let before = text[..byte].chars().last();
                    let after = text[byte + word.len()..].chars().next();
                    if before.is_some_and(is_word) || after.is_some_and(is_word) {
                        continue;
                    }
                }
                let x0 = x_of_display(display_col(text, start_col, tab));
                let x1 = x_of_display(display_col(text, end_col, tab));
                let y = y_of(*line, scroll.y);
                text_painter.rect_filled(
                    Rect::from_min_max(pos2(x0, y + 1.0), pos2(x1, y + line_h - 1.0)),
                    CornerRadius::same(2),
                    Color32::from_rgba_unmultiplied(120, 130, 150, 40),
                );
            }
        }
    }
    update_find(doc);
    if let Some(find) = &doc.view.find {
        let first_char = doc.buffer.char_at(first_line, 0);
        let last_char = doc.buffer.char_at(last_line.saturating_sub(1), usize::MAX);
        let start_index = find.matches.partition_point(|m| m.end < first_char);
        for (i, m) in find.matches[start_index..].iter().enumerate() {
            if m.start > last_char {
                break;
            }
            let color = if start_index + i == find.current { Color32::from_rgb(120, 100, 40) } else { theme::MATCH_HIGHLIGHT };
            paint_range(&text_painter, doc, m.clone(), color, &lines, first_line, &y_of, &x_of_display, line_h, char_w, scroll.y);
        }
    }

    // Selections.
    let selection_color = if focused { theme::SELECTION } else { theme::SELECTION_INACTIVE };
    for selection in &doc.buffer.selections {
        if !selection.is_empty() {
            paint_range(
                &text_painter,
                doc,
                selection.range(),
                selection_color,
                &lines,
                first_line,
                &y_of,
                &x_of_display,
                line_h,
                char_w,
                scroll.y,
            );
        }
    }

    // Indent guides.
    let unit = tab.max(1);
    let mut last_indent = 0usize;
    for (line, text) in &lines {
        let indent = if text.trim().is_empty() {
            last_indent
        } else {
            let ws: String = text.chars().take_while(|c| c.is_whitespace()).collect();
            display_col(&ws, ws.chars().count(), tab)
        };
        last_indent = indent;
        let y = y_of(*line, scroll.y);
        let mut level = unit;
        while level < indent + 1 && level <= indent {
            let x = x_of_display(level - unit) + 0.5;
            if level - unit > 0 || unit <= indent {
                text_painter.line_segment([pos2(x, y), pos2(x, y + line_h)], Stroke::new(1.0, theme::INDENT_GUIDE));
            }
            level += unit;
        }
    }

    // Ghost text occupies virtual rows, moving the real suffix and later lines.
    // The original syntax galley is clipped and translated for the suffix so
    // existing highlighting survives without parsing generated code.
    let ghost_visible = !virtual_layout.text.is_empty();
    let mut widest = doc.view.content_width;
    for (line, text) in &lines {
        let y = y_of(*line, scroll.y);
        let (job, width) = layout_line(doc, *line, text, &font, tab, first_display, last_display, settings.show_whitespace);
        widest = widest.max(width as f32 * char_w + TEXT_PAD * 2.0);
        let galley = text_painter.layout_job(job);
        let x = x_of_display(first_display.min(width));
        if ghost_visible && *line == virtual_layout.line {
            let cursor_x = x_of_display(virtual_layout.column);
            let prefix_clip = Rect::from_min_max(text_rect.min, pos2(cursor_x.min(text_rect.right()), text_rect.bottom()));
            text_painter.with_clip_rect(prefix_clip).galley(pos2(x, y + (line_h - row_h) / 2.0), galley.clone(), theme::TEXT);
            let ghost_color = Color32::from_rgba_unmultiplied(150, 170, 190, 145);
            let expanded = virtual_layout.expanded_lines(tab);
            let mut suffix_column = virtual_layout.column;
            for (offset, (column, text)) in expanded.iter().enumerate() {
                let gy = y + offset as f32 * line_h;
                suffix_column = *column + text.chars().count();
                widest = widest.max(suffix_column as f32 * char_w + TEXT_PAD * 2.0);
                text_painter.text(
                    pos2(x_of_display(*column), gy + (line_h - row_h) / 2.0),
                    Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    ghost_color,
                );
            }
            let suffix_x = x_of_display(suffix_column);
            let suffix_clip = Rect::from_min_max(pos2(suffix_x.max(text_rect.left()), text_rect.top()), text_rect.max);
            let suffix_y = y + virtual_layout.extra as f32 * line_h + (line_h - row_h) / 2.0;
            text_painter.with_clip_rect(suffix_clip).galley(pos2(x + suffix_x - cursor_x, suffix_y), galley, theme::TEXT);
        } else {
            text_painter.galley(pos2(x, y + (line_h - row_h) / 2.0), galley, theme::TEXT);
        }
    }
    doc.view.content_width = widest;

    // Diagnostics: squiggles and an inline message on the line end.
    let first_char = doc.buffer.char_at(first_line, 0);
    let last_char = doc.buffer.char_at(last_line.saturating_sub(1), usize::MAX);
    let mut line_messages: std::collections::BTreeMap<usize, (Severity, String)> = Default::default();
    let mut diagnostics: Vec<&lsp::Diagnostic> =
        doc.all_diagnostics().filter(|d| d.range.end >= first_char && d.range.start <= last_char).collect();
    diagnostics.sort_by_key(|d| std::cmp::Reverse(d.severity));
    for d in &diagnostics {
        let color = severity_color(d.severity);
        if d.severity == Severity::Hint {
            continue;
        }
        let range = if d.range.is_empty() { d.range.start..(d.range.start + 1).min(doc.buffer.len_chars()) } else { d.range.clone() };
        let (start_line, start_col) = doc.buffer.line_col(range.start);
        let (end_line, end_col) = doc.buffer.line_col(range.end);
        for (line, text) in &lines {
            if *line < start_line || *line > end_line {
                continue;
            }
            let from = if *line == start_line { start_col } else { 0 };
            let to = if *line == end_line { end_col } else { text.chars().count() };
            let x0 = x_of_display(display_col(text, from, tab));
            let x1 = x_of_display(display_col(text, to.max(from + 1), tab)).max(x0 + char_w);
            let y = y_of(*line, scroll.y) + line_h - 3.0;
            squiggle(&text_painter, x0, x1, y, color);
        }
        if d.severity <= Severity::Warning {
            let entry = line_messages.entry(start_line).or_insert((d.severity, d.message.clone()));
            if d.severity < entry.0 {
                *entry = (d.severity, d.message.clone());
            }
        }
    }
    for (line, (severity, message)) in &line_messages {
        if ghost_visible && *line == virtual_layout.line {
            continue;
        }
        if let Some((_, text)) = lines.iter().find(|(l, _)| l == line) {
            let end = display_col(text, text.chars().count(), tab);
            let x = x_of_display(end) + char_w * 3.0;
            let y = y_of(*line, scroll.y) + line_h / 2.0;
            let first_line_of_message = message.lines().next().unwrap_or("");
            text_painter.text(
                pos2(x, y),
                Align2::LEFT_CENTER,
                util::truncate_middle(first_line_of_message, 160),
                FontId::proportional(settings.font_size - 1.5),
                severity_color(*severity).gamma_multiply(0.75),
            );
        }
    }

    // Inline blame on the cursor line.
    if settings.inline_blame
        && !ghost_visible
        && single_cursor
        && !line_messages.contains_key(&cursor_line)
        && let Some((_, text)) = lines.iter().find(|(l, _)| *l == cursor_line)
    {
        let changed = doc.line_changes.iter().any(|(l, c)| *l == cursor_line && *c != LineChange::DeletedAbove);
        let label = if changed {
            Some(t!("You • not committed").to_string())
        } else {
            doc.blame.as_ref().and_then(|blame| blame.get(cursor_line)).map(|b| {
                if b.hash.starts_with("0000000") {
                    t!("You • not committed").to_string()
                } else {
                    format!("{}, {} • {}", b.author, util::time_ago(b.time), b.summary)
                }
            })
        };
        if let Some(label) = label {
            let end = display_col(text, text.chars().count(), tab);
            let x = x_of_display(end) + char_w * 4.0;
            let y = y_of(cursor_line, scroll.y) + line_h / 2.0;
            text_painter.text(
                pos2(x, y),
                Align2::LEFT_CENTER,
                util::truncate_middle(&label, 120),
                FontId::proportional(settings.font_size - 1.5),
                theme::FAINT_TEXT,
            );
        }
    }

    // Matching bracket.
    if single_cursor && let Some((a, b)) = matching_bracket(doc, primary.head) {
        for at in [a, b] {
            let (line, col) = doc.buffer.line_col(at);
            if let Some((_, text)) = lines.iter().find(|(l, _)| *l == line) {
                let x = x_of_display(display_col(text, col, tab));
                let y = y_of(line, scroll.y);
                text_painter.rect_stroke(
                    Rect::from_min_size(pos2(x, y + 1.0), vec2(char_w, line_h - 2.0)),
                    CornerRadius::same(2),
                    Stroke::new(1.0, Color32::from_rgb(130, 140, 160)),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    // Cursors.
    let mut primary_cursor_rect = None;
    for (index, selection) in doc.buffer.selections.iter().enumerate() {
        let (line, col) = doc.buffer.line_col(selection.head);
        if line < first_line || line >= last_line {
            continue;
        }
        let text = &lines[line - first_line].1;
        let x = x_of_display(display_col(text, col, tab));
        let y = y_of(line, scroll.y);
        let cursor = Rect::from_min_size(pos2(x - 1.0, y + 2.0), vec2(2.0, line_h - 4.0));
        if focused {
            text_painter.rect_filled(cursor, 0.0, theme::CURSOR);
        } else {
            text_painter.rect_filled(cursor, 0.0, theme::CURSOR.gamma_multiply(0.35));
        }
        if index == doc.buffer.selections.len() - 1 {
            primary_cursor_rect = Some(Rect::from_min_size(pos2(x, y), vec2(2.0, line_h)));
        }
    }
    if focused && let Some(cursor_rect) = primary_cursor_rect {
        ui.ctx().output_mut(|o| {
            o.ime = Some(egui::output::IMEOutput {
                purpose: Default::default(),
                rect: text_rect,
                cursor_rect,
                should_interrupt_composition: false,
            })
        });
    }

    // Gutter: line numbers, git changes, diagnostics.
    gutter_painter.rect_filled(gutter_rect, 0.0, theme::EDITOR_BACKGROUND);
    let mut line_severity: std::collections::HashMap<usize, Severity> = Default::default();
    for d in &diagnostics {
        let (line, _) = doc.buffer.line_col(d.range.start);
        let entry = line_severity.entry(line).or_insert(d.severity);
        if d.severity < *entry {
            *entry = d.severity;
        }
    }
    let number_font = FontId::monospace(settings.font_size - 1.0);
    for (line, _) in &lines {
        let y = y_of(*line, scroll.y);
        let color = match line_severity.get(line) {
            Some(Severity::Error) => theme::ERROR,
            Some(Severity::Warning) => theme::WARNING,
            _ if *line == cursor_line => theme::LINE_NUMBER_ACTIVE,
            _ => theme::LINE_NUMBER,
        };
        gutter_painter.text(
            pos2(gutter_rect.right() - 14.0, y + line_h / 2.0),
            Align2::RIGHT_CENTER,
            (line + 1).to_string(),
            number_font.clone(),
            color,
        );
    }
    let marker_x = gutter_rect.right() - 6.0;
    for &(line, change) in &doc.line_changes {
        if line < first_line || line > last_line {
            continue;
        }
        let y = y_of(line, scroll.y);
        match change {
            LineChange::Added => {
                gutter_painter.rect_filled(Rect::from_min_size(pos2(marker_x, y), vec2(3.0, line_h)), 0.0, theme::GIT_ADDED);
            }
            LineChange::Modified => {
                gutter_painter.rect_filled(Rect::from_min_size(pos2(marker_x, y), vec2(3.0, line_h)), 0.0, theme::GIT_MODIFIED);
            }
            LineChange::DeletedAbove => {
                let points = vec![pos2(marker_x - 1.0, y - 4.0), pos2(marker_x + 5.0, y), pos2(marker_x - 1.0, y + 4.0)];
                gutter_painter.add(Shape::convex_polygon(points, theme::GIT_DELETED, Stroke::NONE));
            }
        }
    }

    // Scrollbar with an overview of diagnostics, changes and matches.
    painter.rect_filled(scrollbar_rect, 0.0, theme::EDITOR_BACKGROUND);
    let ruler_y = |line: usize| scrollbar_rect.top() + (line as f32 / line_count.max(1) as f32) * scrollbar_rect.height();
    for &(line, change) in &doc.line_changes {
        let color = match change {
            LineChange::Added => theme::GIT_ADDED,
            LineChange::Modified => theme::GIT_MODIFIED,
            LineChange::DeletedAbove => theme::GIT_DELETED,
        };
        painter.rect_filled(Rect::from_min_size(pos2(scrollbar_rect.left() + 1.0, ruler_y(line)), vec2(3.0, 2.0)), 0.0, color);
    }
    if let Some(find) = &doc.view.find {
        for m in find.matches.iter().take(2000) {
            let (line, _) = doc.buffer.line_col(m.start);
            painter.rect_filled(
                Rect::from_min_size(pos2(scrollbar_rect.left() + 5.0, ruler_y(line)), vec2(4.0, 2.0)),
                0.0,
                Color32::from_rgb(200, 170, 70),
            );
        }
    }
    for d in doc.all_diagnostics() {
        if d.severity <= Severity::Warning {
            let (line, _) = doc.buffer.line_col(d.range.start);
            painter.rect_filled(
                Rect::from_min_size(pos2(scrollbar_rect.right() - 4.0, ruler_y(line)), vec2(3.0, 2.0)),
                0.0,
                severity_color(d.severity),
            );
        }
    }
    if max_scroll_y > 0.0 {
        let thumb_h = (scrollbar_rect.height() * text_rect.height() / (content_h + text_rect.height())).max(24.0);
        let thumb_top = scrollbar_rect.top() + (doc.view.scroll.y / max_scroll_y) * (scrollbar_rect.height() - thumb_h);
        let hovered = sb_response.hovered() || sb_response.dragged();
        painter.rect_filled(
            Rect::from_min_size(pos2(scrollbar_rect.left() + 2.0, thumb_top), vec2(scrollbar_w - 4.0, thumb_h)),
            CornerRadius::same(3),
            Color32::from_white_alpha(if hovered { 40 } else { 22 }),
        );
    }
    let cursor_y = ruler_y(cursor_line);
    painter.line_segment(
        [pos2(scrollbar_rect.left(), cursor_y), pos2(scrollbar_rect.right(), cursor_y)],
        Stroke::new(1.0, Color32::from_white_alpha(60)),
    );

    // Conflict buttons.
    let conflicts = doc.view.conflicts.clone();
    for conflict in conflicts {
        if conflict.start < first_line || conflict.start >= last_line {
            continue;
        }
        let y = y_of(conflict.start, scroll.y);
        let mut x = text_rect.right() - 12.0;
        let mut choice = None;
        for (label, which) in [(t!("Both"), 2), (t!("Accept incoming"), 1), (t!("Accept current"), 0)] {
            let galley = ui.painter().layout_no_wrap(label.to_string(), FontId::proportional(12.0), theme::ACCENT);
            let w = galley.size().x + 12.0;
            x -= w + 6.0;
            let button_rect = Rect::from_min_size(pos2(x, y + 1.0), vec2(w, line_h - 2.0));
            let button = ui.interact(button_rect, id.with(("conflict", conflict.start, which)), Sense::click());
            let fill = if button.hovered() { theme::SURFACE_HOVER } else { theme::SURFACE };
            painter.rect_filled(button_rect, CornerRadius::same(3), fill);
            painter.galley(pos2(button_rect.left() + 6.0, button_rect.center().y - galley.size().y / 2.0), galley, theme::ACCENT);
            if button.clicked() {
                choice = Some(which);
            }
        }
        if let Some(which) = choice {
            resolve_conflict(doc, conflict, which);
        }
    }

    // Hover detection.
    if response.hovered()
        && doc.view.drag_mode.is_none()
        && !pointer.any_down()
        && let Some(pos) = pointer.hover_pos().filter(|p| text_rect.contains(*p))
    {
        let at = char_at_pos(doc, pos);
        let on_text = doc.buffer.char(at).is_some_and(|c| !c.is_whitespace()) || at > 0 && doc.buffer.char(at - 1).is_some_and(is_word);
        match doc.view.hover_candidate {
            Some((candidate, since, fired)) if doc.buffer.word_at(candidate) == doc.buffer.word_at(at) && candidate.abs_diff(at) < 40 => {
                if !fired && since.elapsed() > Duration::from_millis(450) && on_text {
                    doc.view.hover_candidate = Some((candidate, since, true));
                    let diagnostics_here: Vec<String> = doc
                        .all_diagnostics()
                        .filter(|d| d.range.start <= at && at <= d.range.end.max(d.range.start + 1))
                        .map(|d| format!("{}: {}", d.source, d.message))
                        .collect();
                    doc.view.hover = Some(HoverPopup { text: diagnostics_here.join("\n\n"), anchor: at, pos });
                    if settings.has_lsp {
                        actions.push(EditorAction::Hover(at));
                    }
                } else if !fired {
                    ui.ctx().request_repaint_after(Duration::from_millis(120));
                }
            }
            _ => {
                doc.view.hover_candidate = Some((at, Instant::now(), false));
                ui.ctx().request_repaint_after(Duration::from_millis(500));
            }
        }
    }

    response.clone().on_hover_cursor(egui::CursorIcon::Text);

    // Right click outside the selection moves the cursor there first.
    if response.secondary_clicked()
        && let Some(pos) = pointer.interact_pos()
    {
        let at = char_at_pos(doc, pos);
        if !doc.buffer.selections.iter().any(|s| s.range().contains(&at)) {
            doc.buffer.set_cursor(at, false);
        }
        response.request_focus();
    }
    response.context_menu(|ui| {
        ui.set_min_width(250.0);
        let has_selection = !doc.buffer.primary().is_empty();
        let shortcut = |text: Option<String>| RichText::new(text.unwrap_or_default()).color(theme::FAINT_TEXT);
        // Copy and cut come from the system, they are not in the keymap.
        let system = |key| Some(Chord { ctrl: true, ..Chord::plain(key) }.label());
        if ui.add_enabled(has_selection, egui::Button::new(t!("Cut")).shortcut_text(shortcut(system(Key::X)))).clicked() {
            ui.ctx().copy_text(doc.buffer.selected_text());
            doc.buffer.delete_backward(false);
            ui.close();
        }
        if ui.add_enabled(has_selection, egui::Button::new(t!("Copy")).shortcut_text(shortcut(system(Key::C)))).clicked() {
            ui.ctx().copy_text(doc.buffer.selected_text());
            ui.close();
        }
        if settings.has_lsp {
            ui.separator();
            if ui.add(egui::Button::new(t!("Go to Definition")).shortcut_text(shortcut(Command::GotoDefinition.shortcut_text()))).clicked()
            {
                actions.push(EditorAction::GotoDefinition);
                ui.close();
            }
            if ui.add(egui::Button::new(t!("Find All References")).shortcut_text(shortcut(Command::References.shortcut_text()))).clicked() {
                actions.push(EditorAction::References);
                ui.close();
            }
            if ui.add(egui::Button::new(t!("Rename Symbol")).shortcut_text(shortcut(Command::Rename.shortcut_text()))).clicked() {
                let word = doc.buffer.word_at(doc.buffer.primary().head);
                doc.view.rename = Some(RenamePopup { text: doc.buffer.slice(word), focus: true });
                ui.close();
            }
            if ui.add(egui::Button::new(t!("Quick fixes")).shortcut_text(shortcut(Command::CodeActions.shortcut_text()))).clicked() {
                actions.push(EditorAction::CodeActions);
                ui.close();
            }
            if ui.add(egui::Button::new(t!("Format Document")).shortcut_text(shortcut(Command::Format.shortcut_text()))).clicked() {
                actions.push(EditorAction::Format);
                ui.close();
            }
        }
        ui.separator();
        let what = if has_selection { t!("the selected fragment") } else { t!("the current line") };
        ui.menu_button(RichText::new(t!("π  Ask the agent")).color(theme::ACCENT), |ui| {
            ui.set_min_width(260.0);
            ui.label(RichText::new(t!("Sends {what}", what = what)).small().color(theme::FAINT_TEXT));
            let asks: [(&str, Option<&str>); 5] = [
                (t!("Add to chat (your own question)"), None),
                (t!("Explain"), Some(t!("Explain what this fragment does and why."))),
                (t!("Find bugs"), Some(t!("Find bugs and potential problems in this fragment. Do not change files, only describe them."))),
                (t!("Fix / improve"), Some(t!("Fix the problems and improve this fragment, then briefly describe the changes."))),
                (t!("Write tests"), Some(t!("Write tests for this fragment in the style of the project."))),
            ];
            for (label, instruction) in asks {
                if ui.button(label).clicked() {
                    actions.push(EditorAction::AskAgent(instruction.map(str::to_string)));
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text(t!("Ctrl+Alt+A — add the selection to the chat"));
    });

    // ------------------------------------------------------------- popups
    show_popups(ui, doc, settings, actions, primary_cursor_rect, text_rect, line_h);
    show_find_bar(ui, doc, text_rect);
}

impl ViewState {
    fn scroll_bar_release(&mut self) {
        self.scrollbar_drag = None;
    }
}

fn severity_color(severity: Severity) -> Color32 {
    match severity {
        Severity::Error => theme::ERROR,
        Severity::Warning => theme::WARNING,
        Severity::Info => theme::INFO,
        Severity::Hint => theme::HINT,
    }
}

fn squiggle(painter: &egui::Painter, x0: f32, x1: f32, y: f32, color: Color32) {
    let mut points = Vec::new();
    let mut x = x0;
    let mut up = true;
    while x <= x1 {
        points.push(pos2(x, if up { y } else { y + 2.0 }));
        x += 2.0;
        up = !up;
    }
    if points.len() > 1 {
        painter.add(Shape::line(points, Stroke::new(1.0, color)));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_range(
    painter: &egui::Painter,
    doc: &Document,
    range: Range<usize>,
    color: Color32,
    lines: &[(usize, String)],
    first_line: usize,
    y_of: &impl Fn(usize, f32) -> f32,
    x_of_display: &impl Fn(usize) -> f32,
    line_h: f32,
    char_w: f32,
    scroll_y: f32,
) {
    let tab = doc.buffer.tab_size;
    let (start_line, start_col) = doc.buffer.line_col(range.start);
    let (end_line, end_col) = doc.buffer.line_col(range.end);
    for line in start_line.max(first_line)..=end_line {
        let Some((_, text)) = lines.get(line - first_line) else { break };
        let from = if line == start_line { start_col } else { 0 };
        let to = if line == end_line { end_col } else { text.chars().count() };
        let x0 = x_of_display(display_col(text, from, tab));
        let mut x1 = x_of_display(display_col(text, to, tab));
        if line != end_line {
            x1 += char_w * 0.6;
        }
        if x1 > x0 {
            let y = y_of(line, scroll_y);
            painter.rect_filled(Rect::from_min_max(pos2(x0, y), pos2(x1, y + line_h)), CornerRadius::same(2), color);
        }
    }
}

fn ghost_layout(doc: &Document) -> crate::inline_completion::renderer::Layout {
    if doc.view.completion.is_none()
        && let Some(ghost) = &doc.view.ghost
        && ghost.valid(doc.id, &doc.buffer)
    {
        let (line, col) = doc.buffer.line_col(ghost.identity.cursor);
        return crate::inline_completion::renderer::Layout::new(
            line,
            display_col(&doc.buffer.line_text(line), col, doc.buffer.tab_size),
            &ghost.text,
        );
    }
    Default::default()
}

/// Builds the syntax-colored layout for the visible columns of one line.
/// Returns the job and the line's display width.
#[allow(clippy::too_many_arguments)]
fn layout_line(
    doc: &Document,
    line: usize,
    text: &str,
    font: &FontId,
    tab: usize,
    first_display: usize,
    last_display: usize,
    show_whitespace: bool,
) -> (LayoutJob, usize) {
    let mut job = LayoutJob::default();
    let line_start_byte = doc.buffer.rope.line_to_byte(line);
    let spans = &doc.spans;
    let mut span_index = spans.partition_point(|s| s.end <= line_start_byte);
    let mut display = 0usize;
    let mut run = String::new();
    let mut run_color = theme::TEXT;
    let flush = |job: &mut LayoutJob, run: &mut String, color: Color32| {
        if !run.is_empty() {
            job.append(run, 0.0, TextFormat { font_id: font.clone(), color, ..Default::default() });
            run.clear();
        }
    };
    for (byte, c) in text.char_indices() {
        let width = if c == '\t' { tab - display % tab } else { 1 };
        if display + width <= first_display {
            display += width;
            continue;
        }
        if display > last_display {
            // Still need the full width for horizontal scrolling.
            display += width;
            continue;
        }
        let absolute = line_start_byte + byte;
        while span_index < spans.len() && spans[span_index].end <= absolute {
            span_index += 1;
        }
        let mut color = match spans.get(span_index) {
            Some(span) if span.start <= absolute => theme::syntax_color(span.highlight),
            _ => theme::TEXT,
        };
        let is_ws = c == ' ' || c == '\t';
        let visible_ws = show_whitespace && is_ws;
        if visible_ws {
            color = theme::INDENT_GUIDE.gamma_multiply(2.0);
        }
        if color != run_color {
            flush(&mut job, &mut run, run_color);
            run_color = color;
        }
        if c == '\t' {
            if visible_ws {
                run.push('→');
                run.extend(std::iter::repeat_n(' ', width - 1));
            } else {
                run.extend(std::iter::repeat_n(' ', width));
            }
        } else if c == ' ' && visible_ws || c.is_control() {
            run.push('·');
        } else {
            run.push(c);
        }
        display += width;
    }
    flush(&mut job, &mut run, run_color);
    (job, display)
}

fn update_conflicts(doc: &mut Document) {
    if doc.view.conflicts_version == doc.buffer.version {
        return;
    }
    doc.view.conflicts_version = doc.buffer.version;
    doc.view.conflicts.clear();
    // Cheap pre-check before scanning lines.
    let has_marker = doc.buffer.rope.chunks().any(|chunk| chunk.contains("<<<<<<<"));
    if !has_marker {
        return;
    }
    let mut start = None;
    let mut middle = None;
    for line in 0..doc.buffer.line_count() {
        let text = doc.buffer.rope.line(line);
        let prefix: String = text.chars().take(8).collect();
        if prefix.starts_with("<<<<<<<") {
            start = Some(line);
            middle = None;
        } else if prefix.starts_with("=======") && start.is_some() {
            middle = Some(line);
        } else if prefix.starts_with(">>>>>>>")
            && let (Some(s), Some(m)) = (start, middle)
        {
            doc.view.conflicts.push(Conflict { start: s, middle: m, end: line });
            start = None;
            middle = None;
        }
    }
}

/// 0 = keep current (ours), 1 = incoming (theirs), 2 = both.
fn resolve_conflict(doc: &mut Document, conflict: Conflict, which: u8) {
    let buffer = &doc.buffer;
    let line_range = |from: usize, to: usize| -> String { (from..to).map(|l| format!("{}\n", buffer.line_text(l))).collect() };
    // diff3 style has a `|||||||` base section inside "ours".
    let mut ours_end = conflict.middle;
    for line in conflict.start + 1..conflict.middle {
        if buffer.line_text(line).starts_with("|||||||") {
            ours_end = line;
            break;
        }
    }
    let ours = line_range(conflict.start + 1, ours_end);
    let theirs = line_range(conflict.middle + 1, conflict.end);
    let replacement = match which {
        0 => ours,
        1 => theirs,
        _ => format!("{ours}{theirs}"),
    };
    let start = buffer.char_at(conflict.start, 0);
    let end = if conflict.end + 1 < buffer.line_count() { buffer.char_at(conflict.end + 1, 0) } else { buffer.len_chars() };
    doc.buffer.apply_edits_with_selection(vec![(start..end, replacement)], vec![Selection::cursor(start)]);
}

fn matching_bracket(doc: &Document, at: usize) -> Option<(usize, usize)> {
    let pairs = [('(', ')'), ('[', ']'), ('{', '}')];
    let candidates = [at.checked_sub(1), Some(at)];
    for position in candidates.into_iter().flatten() {
        let Some(c) = doc.buffer.char(position) else { continue };
        if let Some(&(open, close)) = pairs.iter().find(|(o, _)| *o == c) {
            let mut depth = 0;
            let end = (position + 20_000).min(doc.buffer.len_chars());
            for (i, ch) in doc.buffer.rope.slice(position..end).chars().enumerate() {
                if ch == open {
                    depth += 1;
                } else if ch == close {
                    depth -= 1;
                    if depth == 0 {
                        return Some((position, position + i));
                    }
                }
            }
        } else if let Some(&(open, close)) = pairs.iter().find(|(_, cl)| *cl == c) {
            let mut depth = 0;
            let start = position.saturating_sub(20_000);
            let slice: Vec<char> = doc.buffer.rope.slice(start..=position).chars().collect();
            for (i, ch) in slice.iter().enumerate().rev() {
                if *ch == close {
                    depth += 1;
                } else if *ch == open {
                    depth -= 1;
                    if depth == 0 {
                        return Some((start + i, position));
                    }
                }
            }
        }
    }
    None
}

fn update_find(doc: &mut Document) {
    let Some(find) = &mut doc.view.find else { return };
    let key = (doc.buffer.version, find.query.clone(), find.regex, find.case_sensitive, find.whole_word);
    if find.computed_for.as_ref() == Some(&key) {
        return;
    }
    find.computed_for = Some(key);
    find.matches.clear();
    find.error = None;
    if find.query.is_empty() {
        return;
    }
    let query = lumen_shared::fsops::SearchQuery {
        pattern: find.query.clone(),
        regex: find.regex,
        case_sensitive: find.case_sensitive,
        whole_word: find.whole_word,
        max_results: 0,
    };
    match lumen_shared::fsops::build_regex(&query) {
        Ok(regex) => {
            let text = doc.buffer.rope.to_string();
            for m in regex.find_iter(&text).take(20_000) {
                if m.start() == m.end() {
                    continue;
                }
                let start = doc.buffer.rope.byte_to_char(m.start());
                let end = doc.buffer.rope.byte_to_char(m.end());
                find.matches.push(start..end);
            }
            let cursor = doc.buffer.primary().start();
            find.current = find.matches.iter().position(|m| m.start >= cursor).unwrap_or(0);
        }
        Err(error) => find.error = Some(error),
    }
}

pub fn find_step(doc: &mut Document, forward: bool) {
    update_find(doc);
    let Some(find) = &mut doc.view.find else { return };
    if find.matches.is_empty() {
        return;
    }
    let cursor = doc.buffer.primary();
    find.current = if forward {
        find.matches.iter().position(|m| m.start >= cursor.end() && m.start != cursor.start()).unwrap_or(0)
    } else {
        find.matches.iter().rposition(|m| m.end <= cursor.start() && m.start != cursor.start()).unwrap_or(find.matches.len() - 1)
    };
    let m = find.matches[find.current].clone();
    doc.buffer.set_selections(vec![Selection { anchor: m.start, head: m.end }]);
    doc.view.reveal = true;
    doc.view.reveal_center = true;
}

fn replace_current(doc: &mut Document) {
    update_find(doc);
    let Some(find) = &doc.view.find else { return };
    let Some(m) = find.matches.get(find.current).cloned() else { return };
    let replacement = replacement_for(find, &doc.buffer.slice(m.clone()));
    doc.buffer
        .apply_edits_with_selection(vec![(m.clone(), replacement.clone())], vec![Selection::cursor(m.start + replacement.chars().count())]);
    doc.absorb_edits();
    find_step(doc, true);
}

fn replace_all(doc: &mut Document) {
    update_find(doc);
    let Some(find) = &doc.view.find else { return };
    let edits: Vec<(Range<usize>, String)> =
        find.matches.iter().map(|m| (m.clone(), replacement_for(find, &doc.buffer.slice(m.clone())))).collect();
    doc.buffer.apply_edits(edits);
}

fn replacement_for(find: &FindState, matched: &str) -> String {
    if !find.regex {
        return find.replace.clone();
    }
    let query = lumen_shared::fsops::SearchQuery {
        pattern: find.query.clone(),
        regex: true,
        case_sensitive: find.case_sensitive,
        whole_word: find.whole_word,
        max_results: 0,
    };
    match lumen_shared::fsops::build_regex(&query) {
        Ok(regex) => regex.replace(matched, find.replace.as_str()).into_owned(),
        Err(_) => find.replace.clone(),
    }
}

fn show_find_bar(ui: &mut egui::Ui, doc: &mut Document, text_rect: Rect) {
    let Some(find) = &mut doc.view.find else { return };
    let mut close = false;
    let mut step: Option<bool> = None;
    let mut do_replace = false;
    let mut do_replace_all = false;
    let width = 430.0;
    egui::Area::new(Id::unique(("find_bar", doc.id)))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(text_rect.right() - width - 16.0, text_rect.top() + 4.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    let toggle = if find.show_replace { "▾" } else { "▸" };
                    if ui.small_button(toggle).on_hover_text(t!("Toggle replace")).clicked() {
                        find.show_replace = !find.show_replace;
                    }
                    let input = ui.add(egui::TextEdit::singleline(&mut find.query).hint_text(t!("Find")).desired_width(200.0));
                    if find.focus_query {
                        input.request_focus();
                        find.focus_query = false;
                    }
                    if input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        step = Some(!ui.input(|i| i.modifiers.shift));
                        input.request_focus();
                    }
                    if input.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                        close = true;
                    }
                    ui.toggle_value(&mut find.case_sensitive, "Aa").on_hover_text(t!("Match case"));
                    ui.toggle_value(&mut find.whole_word, "ab").on_hover_text(t!("Whole word"));
                    ui.toggle_value(&mut find.regex, ".*").on_hover_text(t!("Regular expression"));
                    let status = if find.error.is_some() {
                        egui::RichText::new(t!("error")).color(theme::ERROR).small()
                    } else if find.matches.is_empty() {
                        egui::RichText::new(t!("none")).color(theme::MUTED_TEXT).small()
                    } else {
                        egui::RichText::new(format!("{}/{}", find.current + 1, find.matches.len())).color(theme::MUTED_TEXT).small()
                    };
                    ui.label(status);
                    if ui.small_button("↑").clicked() {
                        step = Some(false);
                    }
                    if ui.small_button("↓").clicked() {
                        step = Some(true);
                    }
                    if ui.small_button("×").clicked() {
                        close = true;
                    }
                });
                if find.show_replace {
                    ui.horizontal(|ui| {
                        ui.add_space(22.0);
                        let input = ui.add(egui::TextEdit::singleline(&mut find.replace).hint_text(t!("Replace")).desired_width(200.0));
                        if input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                            do_replace = true;
                            input.request_focus();
                        }
                        if ui.button(t!("Replace")).clicked() {
                            do_replace = true;
                        }
                        if ui.button(t!("All")).clicked() {
                            do_replace_all = true;
                        }
                    });
                }
            });
        });
    if let Some(forward) = step {
        find_step(doc, forward);
    }
    if do_replace {
        replace_current(doc);
    }
    if do_replace_all {
        replace_all(doc);
    }
    if close {
        doc.view.find = None;
        doc.view.focus_request = true;
    }
}

fn show_popups(
    ui: &mut egui::Ui,
    doc: &mut Document,
    settings: &EditorSettings,
    actions: &mut Vec<EditorAction>,
    cursor_rect: Option<Rect>,
    text_rect: Rect,
    line_h: f32,
) {
    let ctx = ui.ctx().clone();
    // Completion list.
    if let (Some(completion), Some(cursor)) = (&mut doc.view.completion, cursor_rect) {
        if completion.filtered.is_empty() {
            doc.view.completion = None;
        } else {
            let mut accept = None;
            let row_h = 22.0;
            let shown = completion.filtered.len().min(12);
            let height = shown as f32 * row_h + 8.0;
            let below = cursor.bottom() + height < text_rect.bottom();
            let pos = if below { cursor.left_bottom() + vec2(-4.0, 2.0) } else { cursor.left_top() - vec2(4.0, height + 2.0) };
            let area = egui::Area::new(Id::unique(("completion", doc.id))).order(egui::Order::Foreground).fixed_pos(pos).show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                    ui.set_width(460.0);
                    egui::ScrollArea::vertical().max_height(shown as f32 * row_h).auto_shrink([false, true]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (row, (index, matches)) in completion.filtered.iter().enumerate() {
                            let item = &completion.items[*index];
                            let selected = row == completion.selected;
                            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                            if selected {
                                ui.painter().rect_filled(rect, CornerRadius::same(3), theme::ACCENT_DIM);
                                if completion.scroll_to_selected {
                                    response.scroll_to_me(None);
                                }
                            } else if response.hovered() {
                                ui.painter().rect_filled(rect, CornerRadius::same(3), theme::SURFACE_HOVER);
                            }
                            let (icon, color) = lsp::kind_icon(item.kind);
                            ui.painter().text(
                                rect.left_center() + vec2(12.0, 0.0),
                                Align2::CENTER_CENTER,
                                icon,
                                FontId::proportional(13.0),
                                color,
                            );
                            let job = util::highlighted_label(&item.label, matches, theme::TEXT, theme::ACCENT, 13.5);
                            let galley = ui.painter().layout_job(job);
                            let label_w = galley.size().x;
                            ui.painter().galley(pos2(rect.left() + 26.0, rect.center().y - galley.size().y / 2.0), galley, theme::TEXT);
                            if !item.detail.is_empty() {
                                let available = rect.width() - label_w - 40.0;
                                if available > 40.0 {
                                    let detail = util::truncate_middle(&item.detail, (available / 7.0) as usize);
                                    ui.painter().text(
                                        rect.right_center() - vec2(6.0, 0.0),
                                        Align2::RIGHT_CENTER,
                                        detail,
                                        FontId::proportional(12.0),
                                        theme::MUTED_TEXT,
                                    );
                                }
                            }
                            if response.clicked() {
                                accept = Some(row);
                            }
                        }
                    });
                    if let Some((index, _)) = completion.filtered.get(completion.selected)
                        && let Some(doc_text) = &completion.items[*index].documentation
                        && !doc_text.trim().is_empty()
                    {
                        ui.separator();
                        let text: String = doc_text.lines().take(8).collect::<Vec<_>>().join("\n");
                        ui.label(egui::RichText::new(strip_markdown(&text)).small().color(theme::MUTED_TEXT));
                    }
                });
            });
            completion.scroll_to_selected = false;
            let _ = area;
            if let Some(row) = accept {
                completion.selected = row;
                accept_completion(doc);
                doc.view.focus_request = true;
            }
        }
    }

    // Signature help above the cursor.
    if let (Some(signature), Some(cursor)) = (&doc.view.signature, cursor_rect)
        && doc.view.completion.is_none()
    {
        egui::Area::new(Id::unique(("signature", doc.id)))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(cursor.left_top() - vec2(4.0, 4.0))
            .interactable(false)
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                    ui.set_max_width(640.0);
                    ui.label(egui::RichText::new(signature).monospace().size(settings.font_size - 1.0));
                });
            });
    }

    // Hover.
    let mut close_hover = false;
    if let Some(hover) = &doc.view.hover {
        if hover.text.trim().is_empty() {
            // Waiting for the server; nothing to show yet.
        } else {
            let area = egui::Area::new(Id::unique(("hover", doc.id)))
                .order(egui::Order::Foreground)
                .pivot(Align2::LEFT_BOTTOM)
                .fixed_pos(hover.pos - vec2(8.0, line_h * 0.6))
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                        ui.set_max_width(640.0);
                        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            let _ = render_markdown(ui, &hover.text, settings.font_size);
                        });
                    });
                });
            let pointer = ctx.input(|i| i.pointer.hover_pos());
            if let Some(pointer) = pointer {
                let near = area.response.rect.expand(16.0).contains(pointer) || pointer.distance(hover.pos) < 24.0;
                if !near {
                    close_hover = true;
                }
            }
        }
    }
    if close_hover {
        doc.view.hover = None;
    }

    // Code actions.
    if let (Some(popup), Some(cursor)) = (&mut doc.view.code_actions, cursor_rect) {
        let mut chosen = None;
        let mut close = false;
        egui::Area::new(Id::unique(("code_actions", doc.id)))
            .order(egui::Order::Foreground)
            .fixed_pos(cursor.left_bottom() + vec2(0.0, 2.0))
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(4)).show(ui, |ui| {
                    ui.set_min_width(260.0);
                    if popup.titles.is_empty() {
                        ui.label(egui::RichText::new(t!("No actions available")).color(theme::MUTED_TEXT));
                    }
                    for (i, title) in popup.titles.iter().enumerate() {
                        if ui.selectable_label(i == popup.selected, title).clicked() {
                            chosen = Some(i);
                        }
                    }
                });
            });
        ctx.input(|i| {
            if i.key_pressed(Key::ArrowDown) {
                popup.selected = (popup.selected + 1).min(popup.titles.len().saturating_sub(1));
            }
            if i.key_pressed(Key::ArrowUp) {
                popup.selected = popup.selected.saturating_sub(1);
            }
            if i.key_pressed(Key::Enter) && !popup.titles.is_empty() {
                chosen = Some(popup.selected);
            }
            if i.key_pressed(Key::Escape) {
                close = true;
            }
        });
        if let Some(i) = chosen {
            actions.push(EditorAction::ApplyCodeAction(i));
            close = true;
        }
        if close {
            doc.view.code_actions = None;
            doc.view.focus_request = true;
        }
    }

    // Rename input.
    if let (Some(rename), Some(cursor)) = (&mut doc.view.rename, cursor_rect) {
        let mut done = None;
        egui::Area::new(Id::unique(("rename", doc.id)))
            .order(egui::Order::Foreground)
            .fixed_pos(cursor.left_bottom() + vec2(0.0, 2.0))
            .show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                    ui.label(egui::RichText::new(t!("Rename (Enter to apply)")).small().color(theme::MUTED_TEXT));
                    let input = ui.add(egui::TextEdit::singleline(&mut rename.text).desired_width(260.0));
                    if rename.focus {
                        input.request_focus();
                        rename.focus = false;
                    }
                    if input.lost_focus() {
                        done = Some(ui.input(|i| i.key_pressed(Key::Enter)));
                    }
                });
            });
        if let Some(apply) = done {
            if apply && !rename.text.trim().is_empty() {
                actions.push(EditorAction::Rename(rename.text.trim().to_string()));
            }
            doc.view.rename = None;
            doc.view.focus_request = true;
        }
    }

    super::next_edit_view::show(ui.ctx(), doc);
}

pub fn strip_markdown(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with("```")).collect::<Vec<_>>().join("\n")
}

/// Markdown for hovers and agent replies.
pub fn render_markdown(ui: &mut egui::Ui, text: &str, size: f32) -> Option<String> {
    super::markdown::render(ui, text, size)
}

pub fn accept_completion(doc: &mut Document) {
    let Some(completion) = doc.view.completion.take() else { return };
    let Some((index, _)) = completion.filtered.get(completion.selected) else { return };
    let item = completion.items[*index].clone();
    let cursor = doc.buffer.primary().head;
    let mut range = match &item.edit_range {
        Some(range) => lsp::from_range(&doc.buffer, range, item.utf8),
        None => completion.anchor..cursor,
    };
    if range.end < cursor && range.start <= cursor {
        range.end = cursor;
    }
    let (text, cursor_offset) = if item.snippet { lsp::expand_snippet(&item.insert_text) } else { (item.insert_text.clone(), None) };
    let mut edits = vec![(range.clone(), text.clone())];
    let mut shift: isize = 0;
    for extra in lsp::text_edits_to_ranges(&doc.buffer, &item.additional_edits, item.utf8) {
        if extra.0.end <= range.start {
            shift += extra.1.chars().count() as isize - (extra.0.end - extra.0.start) as isize;
            edits.push(extra);
        } else if extra.0.start >= range.end {
            edits.push(extra);
        }
    }
    let caret = (range.start as isize + shift) as usize + cursor_offset.unwrap_or(text.chars().count());
    doc.buffer.apply_edits_with_selection(edits, vec![Selection::cursor(caret)]);
    doc.view.reveal = true;
}

fn handle_keyboard(
    ui: &mut egui::Ui,
    doc: &mut Document,
    settings: &EditorSettings,
    actions: &mut Vec<EditorAction>,
    visible_lines: usize,
) {
    let events = ui.input(|i| i.events.clone());
    let tab = doc.buffer.tab_size;
    let mut typed: Option<String> = None;
    let mut moved = false;
    let keymap = keymap::current();
    for event in events {
        // Ctrl+C / Ctrl+X / Ctrl+V go by the keymap: they may belong to
        // another command, or to none.
        if !keymap::requested_paste(ui.ctx())
            && let Some(action) = keymap::clipboard_action(&keymap, Scope::Editor, &event)
            && action != ClipboardAction::Native
        {
            if let ClipboardAction::Run(command) = action {
                run_command(ui.ctx(), doc, command, actions, settings.has_lsp);
            }
            continue;
        }
        match event {
            Event::Text(text) | Event::Ime(egui::ImeEvent::Commit(text)) => {
                if text.is_empty() || text.chars().all(|c| c.is_control()) {
                    continue;
                }
                doc.buffer.type_text(&text);
                typed = Some(text);
                doc.view.reveal = true;
                doc.view.hover = None;
            }
            Event::Paste(text) => {
                doc.buffer.paste(&text);
                doc.view.reveal = true;
                doc.view.completion = None;
            }
            Event::Copy | Event::Cut => copy_or_cut(ui.ctx(), doc, matches!(event, Event::Cut)),
            Event::Key { key, physical_key, pressed: true, modifiers, .. }
                if handle_key(ui.ctx(), doc, key, physical_key, modifiers, actions, visible_lines, tab, settings) =>
            {
                moved = true;
            }
            _ => {}
        }
    }

    if moved {
        doc.view.hover = None;
        // Leaving the completed word closes the list.
        if let Some(completion) = &doc.view.completion {
            let head = doc.buffer.primary().head;
            if head < completion.anchor || !doc.buffer.slice(completion.anchor..head).chars().all(is_word) {
                doc.view.completion = None;
            } else {
                let prefix = doc.buffer.slice(completion.anchor..head);
                match doc.view.completion.as_mut() {
                    Some(completion) => completion.refilter(&prefix),
                    None => unreachable!("the outer match guarantees completion is present"),
                }
            }
        }
    }

    if let Some(text) = typed {
        let last = text.chars().last().unwrap_or(' ');
        let head = doc.buffer.primary().head;
        let trigger = settings.completion_triggers.iter().any(|t| text.ends_with(t.as_str()));
        if let Some(completion) = &mut doc.view.completion {
            if is_word(last) && head >= completion.anchor {
                let prefix = doc.buffer.slice(completion.anchor..head);
                completion.refilter(&prefix);
                if completion.filtered.is_empty() {
                    doc.view.completion = None;
                }
            } else {
                doc.view.completion = None;
            }
        }
        if settings.has_lsp && doc.view.completion.is_none() {
            if trigger {
                actions.push(EditorAction::Completion { trigger: Some(last.to_string()) });
            } else if is_word(last) {
                let word = doc.buffer.word_at(head);
                if word.end == head && head - word.start >= 1 && !last.is_ascii_digit() {
                    actions.push(EditorAction::Completion { trigger: None });
                }
            }
        }
        if settings.has_lsp {
            if settings.signature_triggers.iter().any(|t| text.ends_with(t.as_str())) {
                actions.push(EditorAction::SignatureHelp);
            } else if last == ')' {
                doc.view.signature = None;
            }
        }
    }
}

/// Returns true when the cursor moved without editing.
/// Copies the selection, or the whole line when nothing is selected; `cut`
/// also removes it.
fn copy_or_cut(ctx: &egui::Context, doc: &mut Document, cut: bool) {
    if doc.buffer.selections.iter().all(|s| s.is_empty()) {
        let (line, _) = doc.buffer.line_col(doc.buffer.primary().head);
        ctx.copy_text(format!("{}\n", doc.buffer.line_text(line)));
        if cut {
            doc.buffer.delete_lines();
        }
    } else {
        ctx.copy_text(doc.buffer.selected_text());
        if cut {
            doc.buffer.delete_backward(false);
        }
    }
    doc.view.reveal = true;
}

#[allow(clippy::too_many_arguments)]
fn handle_key(
    ctx: &egui::Context,
    doc: &mut Document,
    key: Key,
    physical: Option<Key>,
    m: Modifiers,
    actions: &mut Vec<EditorAction>,
    visible_lines: usize,
    tab: usize,
    settings: &EditorSettings,
) -> bool {
    let ctrl = m.command;

    // Completion navigation takes precedence.
    if let Some(completion) = &mut doc.view.completion {
        match key {
            Key::ArrowDown => {
                completion.selected = (completion.selected + 1) % completion.filtered.len().max(1);
                completion.scroll_to_selected = true;
                return false;
            }
            Key::ArrowUp => {
                completion.selected = completion.selected.checked_sub(1).unwrap_or(completion.filtered.len().saturating_sub(1));
                completion.scroll_to_selected = true;
                return false;
            }
            Key::PageDown => {
                completion.selected = (completion.selected + 10).min(completion.filtered.len().saturating_sub(1));
                completion.scroll_to_selected = true;
                return false;
            }
            Key::PageUp => {
                completion.selected = completion.selected.saturating_sub(10);
                completion.scroll_to_selected = true;
                return false;
            }
            Key::Enter | Key::Tab if !m.shift && !ctrl => {
                accept_completion(doc);
                return false;
            }
            Key::Escape => {
                doc.view.completion = None;
                return false;
            }
            _ => {}
        }
    }
    if doc.view.code_actions.is_some() && matches!(key, Key::ArrowDown | Key::ArrowUp | Key::Enter | Key::Escape) {
        return false; // handled by the popup
    }

    if doc.view.rename.is_none() && doc.view.code_actions.is_none() && doc.view.find.is_none() {
        if key == Key::Escape {
            doc.view.ai_rejected = true;
            if doc.view.ghost.take().is_some() {
                return false;
            }
        }
        if key == Key::Tab
            && !m.shift
            && !ctrl
            && !m.alt
            && let Some(ghost) = doc.view.ghost.take()
            && ghost.accept(doc.id, &mut doc.buffer)
        {
            doc.view.ai_accepted += 1;
            doc.view.reveal = true;
            return false;
        }
    }
    if let Some(command) = keymap::current().lookup(Scope::Editor, key, physical, m) {
        run_command(ctx, doc, command, actions, settings.has_lsp);
        return false;
    }

    // Keys that are not commands: moving, deleting, new lines, indentation.
    doc.view.reveal = true;
    match key {
        Key::ArrowLeft | Key::ArrowRight => {
            doc.buffer.move_horizontal(key == Key::ArrowRight, ctrl || m.alt, m.shift);
            return true;
        }
        Key::ArrowUp | Key::ArrowDown if !ctrl && !m.alt => {
            doc.buffer.move_vertical(if key == Key::ArrowDown { 1 } else { -1 }, m.shift, tab);
            return true;
        }
        Key::Home => {
            if ctrl {
                doc.buffer.move_doc(false, m.shift);
            } else {
                doc.buffer.move_home(m.shift);
            }
            return true;
        }
        Key::End => {
            if ctrl {
                doc.buffer.move_doc(true, m.shift);
            } else {
                doc.buffer.move_end(m.shift);
            }
            return true;
        }
        Key::PageUp | Key::PageDown => {
            let lines = visible_lines.saturating_sub(2).max(1) as isize;
            doc.buffer.move_vertical(if key == Key::PageDown { lines } else { -lines }, m.shift, tab);
            return true;
        }
        Key::Backspace => {
            doc.buffer.delete_backward(ctrl || m.alt);
            if let Some(completion) = &mut doc.view.completion {
                let head = doc.buffer.primary().head;
                if head <= completion.anchor {
                    doc.view.completion = None;
                } else {
                    let prefix = doc.buffer.slice(completion.anchor..head);
                    completion.refilter(&prefix);
                }
            }
        }
        Key::Delete => doc.buffer.delete_forward(ctrl),
        Key::Enter => {
            doc.view.completion = None;
            doc.buffer.newline();
        }
        Key::Tab => {
            if m.shift {
                doc.buffer.outdent();
            } else {
                doc.buffer.indent();
            }
        }
        Key::Escape => {
            if doc.view.hover.take().is_some() || doc.view.signature.take().is_some() || doc.view.rename.take().is_some() {
            } else if doc.buffer.selections.len() > 1 {
                doc.buffer.collapse_to_primary();
            } else if doc.view.find.is_some() {
                doc.view.find = None;
            } else if !doc.buffer.primary().is_empty() {
                let head = doc.buffer.primary().head;
                doc.buffer.set_cursor(head, false);
            }
            doc.view.reveal = false;
        }
        _ => {
            doc.view.reveal = false;
        }
    }
    false
}

/// Runs an editor command on `doc`, from a shortcut, the palette or the CLI.
/// Returns false for commands that are not the editor's.
pub fn run_command(ctx: &egui::Context, doc: &mut Document, command: Command, actions: &mut Vec<EditorAction>, has_lsp: bool) -> bool {
    let tab = doc.buffer.tab_size;
    doc.view.reveal = true;
    match command {
        Command::Copy | Command::Cut => copy_or_cut(ctx, doc, command == Command::Cut),
        // The text arrives next frame as a paste event.
        Command::Paste => keymap::request_paste(ctx),
        Command::Undo | Command::Redo => {
            if command == Command::Undo {
                doc.buffer.undo();
            } else {
                doc.buffer.redo();
            }
            doc.view.completion = None;
        }
        Command::SelectAll => {
            doc.buffer.select_all();
            doc.view.reveal = false;
        }
        Command::SelectLine => doc.buffer.select_line(),
        Command::AddNextOccurrence => doc.buffer.add_next_occurrence(),
        Command::AddCursorAbove | Command::AddCursorBelow => doc.buffer.add_cursor_vertical(command == Command::AddCursorBelow, tab),
        Command::MoveLinesUp | Command::MoveLinesDown => doc.buffer.move_lines(command == Command::MoveLinesDown),
        Command::CopyLinesDown => doc.buffer.duplicate_lines(),
        Command::CopyLinesUp => {
            doc.buffer.duplicate_lines();
            doc.buffer.move_vertical(-1, false, tab);
        }
        Command::DeleteLines => doc.buffer.delete_lines(),
        Command::InsertLineBelow => {
            doc.view.completion = None;
            doc.buffer.move_end(false);
            doc.buffer.newline();
        }
        Command::InsertLineAbove => {
            doc.view.completion = None;
            doc.buffer.move_home(false);
            doc.buffer.move_home(false);
            doc.buffer.newline();
            doc.buffer.move_vertical(-1, false, tab);
        }
        Command::ToggleComment => doc.buffer.toggle_comment(),
        Command::Indent => doc.buffer.indent(),
        Command::Outdent => doc.buffer.outdent(),
        Command::ScrollLineUp | Command::ScrollLineDown => {
            doc.view.scroll.y += if command == Command::ScrollLineDown { 20.0 } else { -20.0 };
            doc.view.reveal = false;
        }
        Command::TriggerInlineCompletion => {
            doc.view.completion = None;
            doc.view.ai_manual = true;
            doc.view.ghost = None;
        }
        Command::TriggerNextEdit => {
            doc.view.next_edit_requested = true;
            doc.view.next_edit_preview = None;
        }
        Command::AcceptNextEdit => {
            accept_next_edit(doc);
        }
        Command::DismissNextEdit => {
            doc.view.next_edit_preview = None;
        }
        Command::TriggerSuggest => {
            if has_lsp {
                actions.push(EditorAction::Completion { trigger: None });
            }
        }
        Command::ShowHover => actions.push(EditorAction::Hover(doc.buffer.primary().head)),
        Command::Find | Command::Replace => {
            let selection = doc.buffer.selected_text();
            doc.view.open_find(Some(selection), command == Command::Replace);
            doc.view.reveal = false;
        }
        Command::FindNext | Command::FindPrevious => {
            if doc.view.find.is_none() {
                let selection = doc.buffer.selected_text();
                doc.view.open_find(Some(selection), false);
            }
            find_step(doc, command == Command::FindNext);
        }
        Command::GotoDefinition => actions.push(EditorAction::GotoDefinition),
        Command::References => actions.push(EditorAction::References),
        Command::Rename => {
            let word = doc.buffer.word_at(doc.buffer.primary().head);
            doc.view.rename = Some(RenamePopup { text: doc.buffer.slice(word), focus: true });
        }
        Command::CodeActions => actions.push(EditorAction::CodeActions),
        Command::Format => actions.push(EditorAction::Format),
        _ => {
            doc.view.reveal = false;
            return false;
        }
    }
    true
}

pub fn accept_next_edit(doc: &mut Document) -> bool {
    let Some(preview) = doc.view.next_edit_preview.take() else {
        return false;
    };
    if !preview.is_current(doc.id, &doc.buffer) {
        return false;
    }
    doc.buffer.apply_prediction_mapped(vec![(preview.range, preview.replacement)]);
    doc.view.completion = None;
    doc.view.ghost = None;
    true
}

pub fn refilter_completion(completion: &mut Completion, prefix: &str) {
    completion.refilter(prefix);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: deleting a multi-line selection crashed the frame that
    /// handled the key, because the line count was taken before the edit.
    #[test]
    fn deleting_lines_in_the_same_frame_does_not_panic() {
        let ctx = egui::Context::default();
        let mut doc = Document::new(1, "/tmp/t.sh".into(), "a\nbb\nccc\ndddd\neeee\n", 4, true);
        doc.view.focus_request = true;
        let settings = EditorSettings {
            font_size: 14.0,
            show_whitespace: false,
            inline_blame: false,
            completion_triggers: &[],
            signature_triggers: &[],
            has_lsp: false,
        };
        let mut actions = Vec::new();
        for frame in 0..3 {
            let mut input = egui::RawInput::default();
            if frame == 1 {
                // Select from line 2 to the end and delete it.
                let start = doc.buffer.char_at(1, 0);
                doc.buffer.set_selections(vec![Selection { anchor: start, head: doc.buffer.len_chars() }]);
                input.events.push(egui::Event::Key {
                    key: Key::Delete,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(input, |ui| show(ui, &mut doc, &settings, &mut actions));
            output.textures_delta.clear();
        }
        assert_eq!(doc.buffer.text(), "a\n");
    }
    fn inline_settings() -> EditorSettings<'static> {
        EditorSettings {
            font_size: 14.0,
            show_whitespace: false,
            inline_blame: false,
            completion_triggers: &[],
            signature_triggers: &[],
            has_lsp: true,
        }
    }
    fn attach_ghost(doc: &mut Document, text: &str) {
        doc.view.ghost = Some(crate::inline_completion::Ghost {
            identity: crate::inline_completion::Identity {
                project: "test".into(),
                doc: doc.id,
                version: doc.buffer.version,
                cursor: doc.buffer.primary().head,
                epoch: 0,
            },
            text: text.into(),
        });
    }
    #[test]
    fn lsp_tab_has_priority_and_ai_undo_is_atomic() {
        let ctx = egui::Context::default();
        let settings = inline_settings();
        let mut actions = Vec::new();
        let mut doc = Document::new(1, "test.rs".into(), "client.", 4, true);
        doc.buffer.set_cursor(7, false);
        attach_ghost(&mut doc, "hallucinated()");
        let items = lsp::parse_completions(serde_json::json!([{"label":"send","insertText":"send()"}]), "fixture");
        doc.view.completion = Some(Completion::new(items, 7));
        match doc.view.completion.as_mut() {
            Some(completion) => completion.refilter(""),
            None => unreachable!("completion was just assigned"),
        }
        handle_key(&ctx, &mut doc, Key::Tab, None, Modifiers::NONE, &mut actions, 20, 4, &settings);
        assert_eq!(doc.buffer.text(), "client.send()");
        assert_eq!(doc.view.ai_accepted, 0);
        doc.buffer.undo();
        assert_eq!(doc.buffer.text(), "client.");
        attach_ghost(&mut doc, "send(\n    value\n)");
        handle_key(&ctx, &mut doc, Key::Tab, None, Modifiers::NONE, &mut actions, 20, 4, &settings);
        assert_eq!(doc.buffer.text(), "client.send(\n    value\n)");
        assert_eq!(doc.view.ai_accepted, 1);
        doc.buffer.undo();
        assert_eq!(doc.buffer.text(), "client.");
        attach_ghost(&mut doc, "x");
        handle_key(&ctx, &mut doc, Key::Tab, None, Modifiers::SHIFT, &mut actions, 20, 4, &settings);
        assert_eq!(doc.view.ai_accepted, 1);
        attach_ghost(&mut doc, "x");
        handle_key(&ctx, &mut doc, Key::Escape, None, Modifiers::NONE, &mut actions, 20, 4, &settings);
        assert!(doc.view.ghost.is_none());
        assert!(doc.view.ai_rejected);
    }
    #[test]
    fn multiline_ghost_renders_virtual_rows_without_editing() {
        let ctx = egui::Context::default();
        let source = "fn main() {\n    client.;\n    next();\n}\n";
        let mut doc = Document::new(1, "test.rs".into(), source, 4, true);
        let at = source.find(".;").unwrap_or_else(|| panic!("expected \".;\"")) + 1;
        doc.buffer.set_cursor(at, false);
        attach_ghost(&mut doc, "send(\n        value\n    )");
        for size in [12.0, 20.0] {
            let settings = EditorSettings { font_size: size, ..inline_settings() };
            let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 360.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| show(ui, &mut doc, &settings, &mut Vec::new()));
            out.textures_delta.clear();
            let labels: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) => Some((t.pos.y, t.galley.job.text.clone())),
                    _ => None,
                })
                .collect();
            let ghost_y = labels.iter().find(|(_, text)| text.contains("send(")).unwrap_or_else(|| panic!("Ghost text is painted")).0;
            let next_y =
                labels.iter().find(|(_, text)| text.contains("next();")).unwrap_or_else(|| panic!("Existing text remains painted")).0;
            assert!(next_y > ghost_y + size * 3.0, "virtual rows must move following code");
            assert_eq!(doc.buffer.text(), source);
            assert_eq!(doc.buffer.version, 0);
        }
    }
}
