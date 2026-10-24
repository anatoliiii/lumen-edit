//! An open file: text buffer plus everything derived from it (highlights,
//! diagnostics, git markers) and per-view state.

use std::{collections::HashMap, time::Instant};

use crate::{
    git::{BlameLine, LineChange},
    lsp::{self, Diagnostic},
    syntax::{Lang, Span},
    text::Buffer,
};

use super::editor::ViewState;

pub struct Document {
    pub id: u64,
    pub path: String,
    pub buffer: Buffer,
    pub lang: Lang,
    pub spans: Vec<Span>,
    pub highlighted_version: u64,
    pub highlight_requested: u64,
    /// Diagnostics per source (server name or `shellcheck`).
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// Version number last sent to language servers.
    pub lsp_version: i32,
    pub lsp_synced: u64,
    pub lsp_opened: Vec<String>,
    pub last_edit: Instant,
    pub git_base: Option<String>,
    pub git_base_loaded: bool,
    pub line_changes: Vec<(usize, LineChange)>,
    pub line_changes_version: u64,
    pub blame: Option<Vec<BlameLine>>,
    pub blame_version: u64,
    pub disk_mtime: u64,
    pub saving: bool,
    pub shellcheck_version: u64,
    pub view: ViewState,
    /// Too large or binary-ish: highlighting and LSP are skipped.
    pub large: bool,
    /// Markdown files open rendered; toggled with Ctrl+Shift+V.
    pub preview: bool,
    /// SVG preview: current image uri (to free the old texture) and zoom.
    pub media_uri: Option<String>,
    pub media_zoom: Option<f32>,
    /// Parsed CSV rows for the table view, by buffer version.
    pub csv: Option<(u64, std::sync::Arc<Vec<Vec<String>>>)>,
    pub next_edit_history: std::collections::VecDeque<crate::inline_completion::next_edit::EditHistoryEntry>,
}

impl Document {
    pub fn new(id: u64, path: String, text: &str, tab_size: usize, insert_spaces: bool) -> Self {
        let lang = Lang::from_path_and_text(&path, text);
        let mut buffer = Buffer::new(text);
        // Keep the file's own indentation if detected, else the settings.
        if !text.lines().take(2000).any(|l| l.starts_with(' ') || l.starts_with('\t')) {
            buffer.tab_size = tab_size;
            buffer.insert_spaces = insert_spaces;
        }
        if lang == Lang::Go || lang == Lang::Makefile {
            buffer.insert_spaces = false;
        }
        buffer.line_comment = lang.line_comment();
        let large = text.len() > 16 * 1024 * 1024;
        let preview = lang == Lang::Markdown || super::media::kind(&path).is_some();
        Self {
            id,
            path,
            buffer,
            lang,
            spans: Vec::new(),
            highlighted_version: u64::MAX,
            highlight_requested: u64::MAX,
            diagnostics: HashMap::new(),
            lsp_version: 0,
            lsp_synced: 0,
            lsp_opened: Vec::new(),
            last_edit: Instant::now(),
            git_base: None,
            git_base_loaded: false,
            line_changes: Vec::new(),
            line_changes_version: u64::MAX,
            blame: None,
            blame_version: 0,
            disk_mtime: 0,
            saving: false,
            shellcheck_version: u64::MAX,
            view: ViewState::default(),
            large,
            preview,
            media_uri: None,
            media_zoom: None,
            csv: None,
            next_edit_history: std::collections::VecDeque::new(),
        }
    }

    pub fn name(&self) -> &str {
        crate::host::file_name(&self.path)
    }

    pub fn is_dirty(&self) -> bool {
        self.buffer.is_dirty()
    }

    /// Moves highlight spans and diagnostics along with edits made since the
    /// last frame so they stay in place until fresh results arrive.
    pub fn absorb_edits(&mut self) {
        for event in self.buffer.take_user_edit_events() {
            if let Some(last) = self.next_edit_history.back_mut() {
                if event.typing
                    && event.before.is_empty()
                    && !last.after.is_empty()
                    && event.start == last.start + last.after.chars().count()
                {
                    last.after.push_str(&event.after);
                    continue;
                }
                if event.deleting
                    && event.after.is_empty()
                    && !last.before.is_empty()
                    && event.start + event.before.chars().count() == last.start
                {
                    last.before = format!("{}{}", event.before, last.before);
                    last.start = event.start;
                    continue;
                }
            }
            self.next_edit_history.push_back(crate::inline_completion::next_edit::EditHistoryEntry {
                start: event.start,
                before: event.before,
                after: event.after,
            });
            while self.next_edit_history.len() > 5 {
                self.next_edit_history.pop_front();
            }
            let total: usize = self.next_edit_history.iter().map(|e| e.before.len() + e.after.len()).sum();
            while total > 16 * 1024 && self.next_edit_history.len() > 1 {
                self.next_edit_history.pop_front();
            }
        }
        let byte_edits = self.buffer.take_byte_edits();
        let char_edits = self.buffer.take_char_edits();
        if byte_edits.is_empty() && char_edits.is_empty() {
            return;
        }
        self.last_edit = Instant::now();
        for edit in &byte_edits {
            let delta = edit.new_end as isize - edit.old_end as isize;
            let shift = |p: usize, is_end: bool| -> usize {
                if p >= edit.old_end {
                    (p as isize + delta) as usize
                } else if p > edit.start {
                    if is_end { edit.new_end } else { edit.start }
                } else {
                    p
                }
            };
            for span in &mut self.spans {
                span.start = shift(span.start, false);
                span.end = shift(span.end, true);
            }
            self.spans.retain(|s| s.end > s.start);
        }
        for &(start, old_end, new_end) in &char_edits {
            let delta = new_end as isize - old_end as isize;
            let shift = |p: usize| -> usize {
                if p >= old_end {
                    (p as isize + delta) as usize
                } else if p > start {
                    start
                } else {
                    p
                }
            };
            for list in self.diagnostics.values_mut() {
                for d in list.iter_mut() {
                    d.range = shift(d.range.start)..shift(d.range.end).max(shift(d.range.start));
                }
            }
            self.view.shift_positions(start, old_end, new_end);
        }
    }

    pub fn all_diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.values().flatten()
    }

    pub fn diagnostic_counts(&self) -> (usize, usize) {
        let mut errors = 0;
        let mut warnings = 0;
        for d in self.all_diagnostics() {
            match d.severity {
                lsp::Severity::Error => errors += 1,
                lsp::Severity::Warning => warnings += 1,
                _ => {}
            }
        }
        (errors, warnings)
    }

    pub fn refresh_line_changes(&mut self) {
        if self.line_changes_version == self.buffer.version {
            return;
        }
        // Debounce while typing in big files.
        if self.buffer.line_count() > 5000 && self.last_edit.elapsed().as_millis() < 300 {
            return;
        }
        self.line_changes_version = self.buffer.version;
        self.line_changes = match &self.git_base {
            Some(base) => crate::git::line_changes(base, &self.buffer.rope.to_string()),
            None => Vec::new(),
        };
    }
}
