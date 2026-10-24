//! Text model of an open document: a rope, multiple selections and undo
//! history. Positions are char indices into the rope.

use std::{collections::VecDeque, ops::Range, time::Instant};

use ropey::Rope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: usize,
    pub head: usize,
}

impl Selection {
    pub fn cursor(at: usize) -> Self {
        Self { anchor: at, head: at }
    }
    pub fn start(&self) -> usize {
        self.anchor.min(self.head)
    }
    pub fn end(&self) -> usize {
        self.anchor.max(self.head)
    }
    pub fn range(&self) -> Range<usize> {
        self.start()..self.end()
    }
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
}

#[derive(Clone, Debug)]
struct Change {
    start: usize,
    removed: String,
    inserted: String,
}

#[derive(Clone, Debug)]
struct Transaction {
    changes: Vec<Change>,
    before: Vec<Selection>,
    after: Vec<Selection>,
    revision_before: u64,
    revision_after: u64,
    user_history: bool,
}

/// An edit in byte coordinates, for incremental consumers (highlighting).
#[derive(Clone, Copy, Debug)]
pub struct ByteEdit {
    pub start: usize,
    pub old_end: usize,
    pub new_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    Other,
    External,
    Prediction,
}

#[derive(Clone, Debug)]
pub struct UserEditEvent {
    pub start: usize,
    pub before: String,
    pub after: String,
    pub typing: bool,
    pub deleting: bool,
}

pub struct Buffer {
    pub rope: Rope,
    pub selections: Vec<Selection>,
    /// Column each selection tries to keep when moving vertically.
    preferred_columns: Vec<Option<usize>>,
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    /// Bumped on every modification; never reused.
    pub version: u64,
    /// Identifies the content; undo restores older revisions.
    revision: u64,
    next_revision: u64,
    saved_revision: u64,
    last_edit: Option<(EditKind, Instant)>,
    pub byte_edits: Vec<ByteEdit>,
    /// Same edits in char coordinates: (start, old_end, new_end).
    pub char_edits: Vec<(usize, usize, usize)>,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub line_comment: Option<&'static str>,
    pub crlf: bool,
    user_edit_events: VecDeque<UserEditEvent>,
}

impl Buffer {
    pub fn new(text: &str) -> Self {
        let crlf = text.contains("\r\n");
        let text = if crlf { text.replace("\r\n", "\n") } else { text.to_string() };
        let indent = detect_indent(&text);
        Self {
            rope: Rope::from_str(&text),
            selections: vec![Selection::cursor(0)],
            preferred_columns: vec![None],
            undo: Vec::new(),
            redo: Vec::new(),
            version: 0,
            revision: 0,
            next_revision: 1,
            saved_revision: 0,
            last_edit: None,
            byte_edits: Vec::new(),
            char_edits: Vec::new(),
            tab_size: indent.map(|(_, size)| size).unwrap_or(4),
            insert_spaces: indent.map(|(spaces, _)| spaces).unwrap_or(true),
            line_comment: None,
            crlf,
            user_edit_events: VecDeque::new(),
        }
    }

    pub fn text(&self) -> String {
        let text = self.rope.to_string();
        if self.crlf { text.replace('\n', "\r\n") } else { text }
    }

    pub fn is_dirty(&self) -> bool {
        self.revision != self.saved_revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_revision = self.revision;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Marks the given revision (the one written to disk) as saved.
    pub fn mark_saved_at(&mut self, revision: u64) {
        self.saved_revision = revision;
    }

    /// Replaces the whole content (e.g. file reloaded from disk) as one undoable step.
    pub fn replace_all(&mut self, text: &str, keep_saved: bool) {
        let text = text.replace("\r\n", "\n");
        let len = self.rope.len_chars();
        let cursor = self.primary().head.min(text.chars().count());
        self.apply(vec![(0..len, text)], EditKind::Other, Some(vec![Selection::cursor(cursor)]));
        if keep_saved {
            self.mark_saved();
        }
    }

    pub fn primary(&self) -> Selection {
        *self.selections.last().unwrap()
    }

    pub fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    pub fn line_col(&self, char_index: usize) -> (usize, usize) {
        let char_index = char_index.min(self.rope.len_chars());
        let line = self.rope.char_to_line(char_index);
        (line, char_index - self.rope.line_to_char(line))
    }

    pub fn char_at(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.rope.len_lines().saturating_sub(1));
        let start = self.rope.line_to_char(line);
        start + col.min(self.line_len(line))
    }

    /// Line length in chars without the trailing newline.
    pub fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let slice = self.rope.line(line);
        let mut len = slice.len_chars();
        if len > 0 && slice.char(len - 1) == '\n' {
            len -= 1;
        }
        len
    }

    pub fn line_text(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let mut text = self.rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.rope.len_chars());
        self.rope.slice(range.start.min(end)..end).to_string()
    }

    pub fn char(&self, index: usize) -> Option<char> {
        (index < self.rope.len_chars()).then(|| self.rope.char(index))
    }

    pub fn set_cursor(&mut self, at: usize, extend: bool) {
        let at = at.min(self.len_chars());
        let anchor = if extend { self.primary().anchor } else { at };
        self.selections = vec![Selection { anchor, head: at }];
        self.preferred_columns = vec![None];
    }

    pub fn set_selections(&mut self, selections: Vec<Selection>) {
        let len = self.len_chars();
        let mut selections: Vec<Selection> =
            selections.into_iter().map(|s| Selection { anchor: s.anchor.min(len), head: s.head.min(len) }).collect();
        if selections.is_empty() {
            selections.push(Selection::cursor(0));
        }
        self.preferred_columns = vec![None; selections.len()];
        self.selections = selections;
        self.merge_selections();
    }

    pub fn add_selection(&mut self, selection: Selection) {
        self.selections.push(selection);
        self.preferred_columns.push(None);
        self.merge_selections();
    }

    fn merge_selections(&mut self) {
        if self.selections.len() < 2 {
            return;
        }
        let primary = self.primary();
        let mut sorted = self.selections.clone();
        sorted.sort_by_key(|s| s.start());
        let mut merged: Vec<Selection> = Vec::with_capacity(sorted.len());
        for selection in sorted {
            if let Some(last) = merged.last_mut()
                && (selection.start() < last.end() || selection.start() == last.start())
            {
                let start = last.start().min(selection.start());
                let end = last.end().max(selection.end());
                *last =
                    if last.head >= last.anchor { Selection { anchor: start, head: end } } else { Selection { anchor: end, head: start } };
                continue;
            }
            merged.push(selection);
        }
        // Keep the primary selection last.
        if let Some(index) = merged.iter().position(|s| s.range().contains(&primary.head) || *s == primary) {
            let primary = merged.remove(index);
            merged.push(primary);
        }
        self.preferred_columns = vec![None; merged.len()];
        self.selections = merged;
    }

    pub fn collapse_to_primary(&mut self) {
        let primary = self.primary();
        self.selections = vec![primary];
        self.preferred_columns = vec![None];
    }

    // ---------------------------------------------------------------- edits

    /// Applies non-overlapping replacements given in current coordinates.
    /// Selections are mapped through the edits unless `selections` is given.
    fn apply(&mut self, mut edits: Vec<(Range<usize>, String)>, kind: EditKind, selections: Option<Vec<Selection>>) {
        edits.retain(|(range, text)| !(range.is_empty() && text.is_empty()));
        if edits.is_empty() {
            if let Some(selections) = selections {
                self.set_selections(selections);
            }
            return;
        }
        edits.sort_by_key(|(range, _)| range.start);
        let before = self.selections.clone();

        // Map selections: each position shifts by the deltas of edits before it.
        let map = |pos: usize, edits: &[(Range<usize>, String)], assoc_after: bool| -> usize {
            let mut delta: isize = 0;
            for (range, text) in edits {
                let inserted = text.chars().count() as isize;
                if pos < range.start || (pos == range.start && !assoc_after && !range.is_empty()) {
                    break;
                }
                if pos <= range.end {
                    // Inside the replaced range: move to the end of the inserted text.
                    return (range.start as isize + delta + inserted) as usize;
                }
                delta += inserted - (range.end - range.start) as isize;
            }
            (pos as isize + delta) as usize
        };
        let mapped = selections.unwrap_or_else(|| {
            self.selections.iter().map(|s| Selection { anchor: map(s.anchor, &edits, true), head: map(s.head, &edits, true) }).collect()
        });

        let mut changes = Vec::with_capacity(edits.len());
        for (range, text) in edits.iter().rev() {
            let end = range.end.min(self.rope.len_chars());
            let start = range.start.min(end);
            let removed = self.rope.slice(start..end).to_string();
            if !matches!(kind, EditKind::External | EditKind::Prediction) {
                self.user_edit_events.push_back(UserEditEvent {
                    start,
                    before: removed.clone(),
                    after: text.clone(),
                    typing: kind == EditKind::Typing,
                    deleting: kind == EditKind::Deleting,
                });
                while self.user_edit_events.len() > 32 {
                    self.user_edit_events.pop_front();
                }
            }
            self.record_byte_edit(start, end, text);
            self.rope.remove(start..end);
            self.rope.insert(start, text);
            changes.push(Change { start, removed, inserted: text.clone() });
        }

        let revision_before = self.revision;
        self.revision = self.next_revision;
        self.next_revision += 1;
        self.version += 1;
        self.set_selections(mapped);
        let after = self.selections.clone();

        let now = Instant::now();
        let merge = matches!(kind, EditKind::Typing | EditKind::Deleting)
            && self.last_edit.is_some_and(|(last, at)| last == kind && now.duration_since(at).as_millis() < 1000)
            && self.redo.is_empty()
            && self.undo.last().is_some_and(|t| t.revision_after == revision_before)
            && self.saved_revision != revision_before;
        self.redo.clear();
        if merge {
            let last = self.undo.last_mut().unwrap();
            last.changes.extend(changes);
            last.after = after;
            last.revision_after = self.revision;
        } else {
            self.undo.push(Transaction {
                changes,
                before,
                after,
                revision_before,
                revision_after: self.revision,
                user_history: !matches!(kind, EditKind::External | EditKind::Prediction),
            });
            if self.undo.len() > 2000 {
                self.undo.remove(0);
            }
        }
        self.last_edit = Some((kind, now));
    }

    fn record_byte_edit(&mut self, start: usize, end: usize, text: &str) {
        let start_byte = self.rope.char_to_byte(start);
        let old_end = self.rope.char_to_byte(end);
        self.byte_edits.push(ByteEdit { start: start_byte, old_end, new_end: start_byte + text.len() });
        self.char_edits.push((start, end, start + text.chars().count()));
    }

    /// Public entry for edits coming from LSP or tools.
    pub fn apply_edits(&mut self, edits: Vec<(Range<usize>, String)>) {
        self.apply(edits, EditKind::External, None);
    }

    pub fn apply_edits_with_selection(&mut self, edits: Vec<(Range<usize>, String)>, selections: Vec<Selection>) {
        self.apply(edits, EditKind::Other, Some(selections));
    }

    pub fn apply_prediction_mapped(&mut self, edits: Vec<(Range<usize>, String)>) {
        self.apply(edits, EditKind::Prediction, None);
    }

    pub fn take_user_edit_events(&mut self) -> Vec<UserEditEvent> {
        self.user_edit_events.drain(..).collect()
    }

    pub fn undo(&mut self) {
        let Some(transaction) = self.undo.pop() else { return };
        if transaction.user_history {
            for change in transaction.changes.iter().rev() {
                self.user_edit_events.push_back(UserEditEvent {
                    start: change.start,
                    before: change.inserted.clone(),
                    after: change.removed.clone(),
                    typing: false,
                    deleting: false,
                });
            }
        }
        for change in transaction.changes.iter().rev() {
            let end = change.start + change.inserted.chars().count();
            self.record_byte_edit(change.start, end, &change.removed);
            self.rope.remove(change.start..end);
            self.rope.insert(change.start, &change.removed);
        }
        self.revision = transaction.revision_before;
        self.version += 1;
        self.set_selections(transaction.before.clone());
        self.redo.push(transaction);
        self.last_edit = None;
    }

    pub fn redo(&mut self) {
        let Some(transaction) = self.redo.pop() else { return };
        if transaction.user_history {
            for change in &transaction.changes {
                self.user_edit_events.push_back(UserEditEvent {
                    start: change.start,
                    before: change.removed.clone(),
                    after: change.inserted.clone(),
                    typing: false,
                    deleting: false,
                });
            }
        }
        for change in &transaction.changes {
            let end = change.start + change.removed.chars().count();
            self.record_byte_edit(change.start, end, &change.inserted);
            self.rope.remove(change.start..end);
            self.rope.insert(change.start, &change.inserted);
        }
        self.revision = transaction.revision_after;
        self.version += 1;
        self.set_selections(transaction.after.clone());
        self.undo.push(transaction);
        self.last_edit = None;
    }

    pub fn selected_text(&self) -> String {
        let mut sorted = self.selections.clone();
        sorted.sort_by_key(|s| s.start());
        sorted.iter().map(|s| self.slice(s.range())).collect::<Vec<_>>().join("\n")
    }

    pub fn indent_unit(&self) -> String {
        if self.insert_spaces { " ".repeat(self.tab_size) } else { "\t".into() }
    }

    /// Typing: replaces each selection, with bracket auto-closing.
    pub fn type_text(&mut self, text: &str) {
        let pairs = [('(', ')'), ('[', ']'), ('{', '}'), ('"', '"'), ('\'', '\''), ('`', '`')];
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(c), None) => Some(c),
            _ => None,
        };

        if let Some(c) = single {
            // Step over an auto-inserted closing character.
            let closing = pairs.iter().any(|(_, close)| *close == c);
            if closing && self.selections.iter().all(|s| s.is_empty() && self.char(s.head) == Some(c)) {
                let selections = self.selections.iter().map(|s| Selection::cursor(s.head + 1)).collect();
                self.set_selections(selections);
                return;
            }
            if let Some(&(open, close)) = pairs.iter().find(|(open, _)| *open == c) {
                let all_ok = self.selections.iter().all(|s| {
                    if !s.is_empty() {
                        return true;
                    }
                    let next = self.char(s.head);
                    let prev = s.head.checked_sub(1).and_then(|i| self.char(i));
                    let next_ok = next.is_none_or(|n| n.is_whitespace() || ")]};,".contains(n));
                    let quote = open == close;
                    let prev_ok = !quote || prev.is_none_or(|p| !p.is_alphanumeric() && p != open);
                    next_ok && prev_ok
                });
                if all_ok {
                    let edits: Vec<_> =
                        self.selections.iter().map(|s| (s.range(), format!("{open}{}{close}", self.slice(s.range())))).collect();
                    let selections: Vec<Selection> = {
                        let mut sorted: Vec<(usize, Selection)> = self.selections.iter().copied().enumerate().collect();
                        sorted.sort_by_key(|(_, s)| s.start());
                        let mut delta = 0usize;
                        let mut out = vec![Selection::cursor(0); sorted.len()];
                        for (index, s) in sorted {
                            let start = s.start() + delta + 1;
                            out[index] = if s.is_empty() {
                                Selection::cursor(start)
                            } else {
                                Selection { anchor: start, head: start + s.end() - s.start() }
                            };
                            delta += 2;
                        }
                        out
                    };
                    self.apply(edits, EditKind::Typing, Some(selections));
                    return;
                }
            }
        }
        let edits = self.selections.iter().map(|s| (s.range(), text.to_string())).collect();
        self.apply(edits, EditKind::Typing, None);
    }

    /// Paste: distributes lines over cursors when the counts match.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n");
        let lines: Vec<&str> = text.split('\n').collect();
        let edits = if self.selections.len() > 1 && lines.len() == self.selections.len() {
            let mut order: Vec<usize> = (0..self.selections.len()).collect();
            order.sort_by_key(|&i| self.selections[i].start());
            order.iter().enumerate().map(|(n, &i)| (self.selections[i].range(), lines[n].to_string())).collect()
        } else {
            self.selections.iter().map(|s| (s.range(), text.clone())).collect()
        };
        self.apply(edits, EditKind::Other, None);
    }

    pub fn newline(&mut self) {
        let mut edits = Vec::new();
        let mut cursors = Vec::new();
        let mut sorted = self.selections.clone();
        sorted.sort_by_key(|s| s.start());
        let mut delta = 0usize;
        for s in sorted {
            let (line, _) = self.line_col(s.start());
            let line_text = self.line_text(line);
            let indent: String = line_text.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            let before = s.start().checked_sub(1).and_then(|i| self.char(i));
            let after = self.char(s.end());
            let opens = before.is_some_and(|c| "{[(".contains(c));
            let colon = before == Some(':') && self.line_comment == Some("#");
            let unit = self.indent_unit();
            let (text, cursor_offset) = if opens && after.is_some_and(|c| "}])".contains(c)) {
                let inner = format!("\n{indent}{unit}");
                let len = inner.chars().count();
                (format!("{inner}\n{indent}"), len)
            } else if opens || colon {
                let text = format!("\n{indent}{unit}");
                let len = text.chars().count();
                (text, len)
            } else {
                let text = format!("\n{indent}");
                let len = text.chars().count();
                (text, len)
            };
            cursors.push(Selection::cursor(s.start() + delta + cursor_offset));
            delta += text.chars().count();
            delta -= s.end() - s.start();
            edits.push((s.range(), text));
        }
        self.apply(edits, EditKind::Other, Some(cursors));
    }

    pub fn delete_backward(&mut self, word: bool) {
        let ranges: Vec<Range<usize>> = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    return s.range();
                }
                let head = s.head;
                if head == 0 {
                    return 0..0;
                }
                if word {
                    return self.word_boundary_left(head)..head;
                }
                // Delete an empty auto-closed pair at once.
                let prev = self.char(head - 1);
                let next = self.char(head);
                if let (Some(p), Some(n)) = (prev, next)
                    && matches!((p, n), ('(', ')') | ('[', ']') | ('{', '}') | ('"', '"') | ('\'', '\''))
                {
                    return head - 1..head + 1;
                }
                // In leading spaces, delete back to the previous tab stop.
                let (line, col) = self.line_col(head);
                let line_start = head - col;
                let leading = self.slice(line_start..head);
                if self.insert_spaces && col > 0 && leading.chars().all(|c| c == ' ') {
                    let remove = match col % self.tab_size {
                        0 => self.tab_size,
                        rest => rest,
                    };
                    let _ = line;
                    return head - remove.min(col)..head;
                }
                head - 1..head
            })
            .collect();
        self.apply(ranges.into_iter().map(|r| (r, String::new())).collect(), EditKind::Deleting, None);
    }

    pub fn delete_forward(&mut self, word: bool) {
        let len = self.len_chars();
        let ranges: Vec<Range<usize>> = self
            .selections
            .iter()
            .map(|s| {
                if !s.is_empty() {
                    s.range()
                } else if word {
                    s.head..self.word_boundary_right(s.head)
                } else {
                    s.head..(s.head + 1).min(len)
                }
            })
            .collect();
        self.apply(ranges.into_iter().map(|r| (r, String::new())).collect(), EditKind::Deleting, None);
    }

    fn selected_lines(&self) -> Vec<usize> {
        let mut lines = Vec::new();
        for s in &self.selections {
            let (first, _) = self.line_col(s.start());
            let (mut last, last_col) = self.line_col(s.end());
            if last > first && last_col == 0 {
                last -= 1;
            }
            lines.extend(first..=last);
        }
        lines.sort_unstable();
        lines.dedup();
        lines
    }

    pub fn indent(&mut self) {
        let multi_line = self.selections.iter().any(|s| {
            let (a, _) = self.line_col(s.start());
            let (b, _) = self.line_col(s.end());
            a != b
        });
        if !multi_line {
            // Insert up to the next tab stop.
            let edits = self
                .selections
                .iter()
                .map(|s| {
                    let text = if self.insert_spaces {
                        let (_, col) = self.line_col(s.start());
                        " ".repeat(self.tab_size - col % self.tab_size)
                    } else {
                        "\t".into()
                    };
                    (s.range(), text)
                })
                .collect();
            self.apply(edits, EditKind::Typing, None);
            return;
        }
        let unit = self.indent_unit();
        let edits = self
            .selected_lines()
            .into_iter()
            .filter(|&line| self.line_len(line) > 0)
            .map(|line| {
                let at = self.rope.line_to_char(line);
                (at..at, unit.clone())
            })
            .collect();
        self.apply(edits, EditKind::Other, None);
    }

    pub fn outdent(&mut self) {
        let edits = self
            .selected_lines()
            .into_iter()
            .filter_map(|line| {
                let text = self.line_text(line);
                let start = self.rope.line_to_char(line);
                let remove = if text.starts_with('\t') { 1 } else { text.chars().take(self.tab_size).take_while(|c| *c == ' ').count() };
                (remove > 0).then(|| (start..start + remove, String::new()))
            })
            .collect();
        self.apply(edits, EditKind::Other, None);
    }

    pub fn toggle_comment(&mut self) {
        let Some(token) = self.line_comment else { return };
        let lines: Vec<usize> = self.selected_lines().into_iter().filter(|&l| !self.line_text(l).trim().is_empty()).collect();
        if lines.is_empty() {
            return;
        }
        let all_commented = lines.iter().all(|&l| self.line_text(l).trim_start().starts_with(token));
        let min_indent = lines.iter().map(|&l| self.line_text(l).chars().take_while(|c| c.is_whitespace()).count()).min().unwrap_or(0);
        let edits = lines
            .iter()
            .map(|&line| {
                let text = self.line_text(line);
                let start = self.rope.line_to_char(line);
                if all_commented {
                    let indent = text.chars().take_while(|c| c.is_whitespace()).count();
                    let after = &text[text.char_indices().nth(indent).map(|(i, _)| i).unwrap_or(0)..];
                    let mut remove = token.chars().count();
                    if after[token.len()..].starts_with(' ') {
                        remove += 1;
                    }
                    (start + indent..start + indent + remove, String::new())
                } else {
                    (start + min_indent..start + min_indent, format!("{token} "))
                }
            })
            .collect();
        self.apply(edits, EditKind::Other, None);
    }

    fn line_block(&self) -> (usize, usize) {
        let lines = self.selected_lines();
        (*lines.first().unwrap_or(&0), *lines.last().unwrap_or(&0))
    }

    pub fn duplicate_lines(&mut self) {
        let (first, last) = self.line_block();
        let start = self.rope.line_to_char(first);
        let end = self.char_at(last, usize::MAX);
        let text = self.slice(start..end);
        let len = text.chars().count() + 1;
        let selections = self.selections.iter().map(|s| Selection { anchor: s.anchor + len, head: s.head + len }).collect();
        self.apply(vec![(end..end, format!("\n{text}"))], EditKind::Other, Some(selections));
    }

    pub fn delete_lines(&mut self) {
        let (first, last) = self.line_block();
        let start = self.rope.line_to_char(first);
        let end = if last + 1 < self.line_count() { self.rope.line_to_char(last + 1) } else { self.len_chars() };
        let start = if end == self.len_chars() && first > 0 { start - 1 } else { start };
        self.apply(vec![(start..end, String::new())], EditKind::Other, Some(vec![Selection::cursor(start.min(self.len_chars()))]));
        let at = self.primary().head;
        let (line, _) = self.line_col(at);
        self.set_cursor(self.rope.line_to_char(line.min(first)), false);
    }

    pub fn move_lines(&mut self, down: bool) {
        let (first, last) = self.line_block();
        if (!down && first == 0) || (down && last + 1 >= self.line_count()) {
            return;
        }
        let block_start = self.rope.line_to_char(first);
        let block_end = self.char_at(last, usize::MAX);
        let block = self.slice(block_start..block_end);
        if down {
            let next_end = self.char_at(last + 1, usize::MAX);
            let next = self.slice(block_end + 1..next_end);
            let shift = next.chars().count() + 1;
            let selections = self.selections.iter().map(|s| Selection { anchor: s.anchor + shift, head: s.head + shift }).collect();
            self.apply(vec![(block_start..next_end, format!("{next}\n{block}"))], EditKind::Other, Some(selections));
        } else {
            let prev_start = self.rope.line_to_char(first - 1);
            let prev = self.slice(prev_start..block_start - 1);
            let shift = prev.chars().count() + 1;
            let selections = self.selections.iter().map(|s| Selection { anchor: s.anchor - shift, head: s.head - shift }).collect();
            self.apply(vec![(prev_start..block_end, format!("{block}\n{prev}"))], EditKind::Other, Some(selections));
        }
    }

    // ------------------------------------------------------------- movement

    pub fn word_boundary_left(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && self.char(i - 1).is_some_and(|c| c.is_whitespace() && c != '\n') {
            i -= 1;
        }
        if i > 0 {
            let class = char_class(self.char(i - 1).unwrap());
            while i > 0 && self.char(i - 1).is_some_and(|c| char_class(c) == class && c != '\n') {
                i -= 1;
            }
        }
        if i == from && from > 0 { from - 1 } else { i }
    }

    pub fn word_boundary_right(&self, from: usize) -> usize {
        let len = self.len_chars();
        let mut i = from;
        while i < len && self.char(i).is_some_and(|c| c.is_whitespace() && c != '\n') {
            i += 1;
        }
        if i < len {
            let class = char_class(self.char(i).unwrap());
            while i < len && self.char(i).is_some_and(|c| char_class(c) == class && c != '\n') {
                i += 1;
            }
        }
        if i == from && from < len { from + 1 } else { i }
    }

    pub fn word_at(&self, at: usize) -> Range<usize> {
        let is_word = |c: char| c.is_alphanumeric() || c == '_';
        let mut start = at;
        while start > 0 && self.char(start - 1).is_some_and(is_word) {
            start -= 1;
        }
        let mut end = at;
        while self.char(end).is_some_and(is_word) {
            end += 1;
        }
        start..end
    }

    fn move_each(&mut self, extend: bool, keep_column: bool, f: impl Fn(&Self, Selection, Option<usize>) -> (usize, Option<usize>)) {
        let mut columns = self.preferred_columns.clone();
        columns.resize(self.selections.len(), None);
        let mut new = Vec::with_capacity(self.selections.len());
        for (i, s) in self.selections.iter().enumerate() {
            let (head, column) = f(self, *s, columns[i]);
            columns[i] = if keep_column { column } else { None };
            new.push(Selection { anchor: if extend { s.anchor } else { head }, head });
        }
        self.selections = new;
        self.preferred_columns = columns;
        self.merge_selections_keep_columns();
    }

    fn merge_selections_keep_columns(&mut self) {
        let columns = self.preferred_columns.clone();
        let count = self.selections.len();
        self.merge_selections();
        if self.selections.len() == count {
            self.preferred_columns = columns;
        }
    }

    pub fn move_horizontal(&mut self, right: bool, word: bool, extend: bool) {
        let len = self.len_chars();
        self.move_each(extend, false, |buffer, s, _| {
            if !extend && !s.is_empty() && !word {
                return (if right { s.end() } else { s.start() }, None);
            }
            let head = match (right, word) {
                (true, true) => buffer.word_boundary_right(s.head),
                (true, false) => (s.head + 1).min(len),
                (false, true) => buffer.word_boundary_left(s.head),
                (false, false) => s.head.saturating_sub(1),
            };
            (head, None)
        });
    }

    /// `visual_col` maps (line, char col) to a display column; vertical
    /// movement keeps the display column so tabs line up.
    pub fn move_vertical(&mut self, lines: isize, extend: bool, tab: usize) {
        self.move_each(extend, true, |buffer, s, preferred| {
            let (line, col) = buffer.line_col(s.head);
            let display = preferred.unwrap_or_else(|| display_col(&buffer.line_text(line), col, tab));
            let target = line as isize + lines;
            if target < 0 {
                return (0, Some(display));
            }
            if target as usize >= buffer.line_count() {
                return (buffer.len_chars(), Some(display));
            }
            let target = target as usize;
            let col = char_col_for_display(&buffer.line_text(target), display, tab);
            (buffer.char_at(target, col), Some(display))
        });
    }

    pub fn move_home(&mut self, extend: bool) {
        self.move_each(extend, false, |buffer, s, _| {
            let (line, col) = buffer.line_col(s.head);
            let text = buffer.line_text(line);
            let indent = text.chars().take_while(|c| c.is_whitespace()).count();
            let target = if col == indent { 0 } else { indent };
            (buffer.char_at(line, target), None)
        });
    }

    pub fn move_end(&mut self, extend: bool) {
        self.move_each(extend, false, |buffer, s, _| {
            let (line, _) = buffer.line_col(s.head);
            (buffer.char_at(line, usize::MAX), None)
        });
    }

    pub fn move_doc(&mut self, end: bool, extend: bool) {
        let target = if end { self.len_chars() } else { 0 };
        self.move_each(extend, false, |_, _, _| (target, None));
    }

    pub fn select_all(&mut self) {
        self.selections = vec![Selection { anchor: 0, head: self.len_chars() }];
        self.preferred_columns = vec![None];
    }

    pub fn select_line(&mut self) {
        let selections = self
            .selections
            .iter()
            .map(|s| {
                let (first, _) = self.line_col(s.start());
                let (last, _) = self.line_col(s.end());
                let start = self.rope.line_to_char(first);
                let end = if last + 1 < self.line_count() { self.rope.line_to_char(last + 1) } else { self.len_chars() };
                Selection { anchor: start, head: end }
            })
            .collect();
        self.set_selections(selections);
    }

    pub fn add_cursor_vertical(&mut self, down: bool, tab: usize) {
        let s = self.primary();
        let (line, col) = self.line_col(s.head);
        let target = if down {
            line + 1
        } else {
            match line.checked_sub(1) {
                Some(l) => l,
                None => return,
            }
        };
        if target >= self.line_count() {
            return;
        }
        let display = display_col(&self.line_text(line), col, tab);
        let col = char_col_for_display(&self.line_text(target), display, tab);
        self.add_selection(Selection::cursor(self.char_at(target, col)));
    }

    /// Ctrl+D: select the word under the cursor, then the next occurrence.
    pub fn add_next_occurrence(&mut self) {
        let primary = self.primary();
        if primary.is_empty() {
            let word = self.word_at(primary.head);
            if !word.is_empty() {
                let last = self.selections.len() - 1;
                self.selections[last] = Selection { anchor: word.start, head: word.end };
            }
            return;
        }
        let needle = self.slice(primary.range());
        let text = self.rope.to_string();
        let from_byte = self.rope.char_to_byte(primary.end());
        let found = text[from_byte..].find(&needle).map(|i| i + from_byte).or_else(|| text.find(&needle));
        if let Some(byte) = found {
            let start = self.rope.byte_to_char(byte);
            let selection = Selection { anchor: start, head: start + needle.chars().count() };
            if !self.selections.contains(&selection) {
                self.add_selection(selection);
            }
        }
    }

    pub fn take_byte_edits(&mut self) -> Vec<ByteEdit> {
        std::mem::take(&mut self.byte_edits)
    }

    pub fn take_char_edits(&mut self) -> Vec<(usize, usize, usize)> {
        std::mem::take(&mut self.char_edits)
    }
}

fn char_class(c: char) -> u8 {
    if c.is_alphanumeric() || c == '_' {
        0
    } else if c.is_whitespace() {
        1
    } else {
        2
    }
}

pub fn display_col(line: &str, col: usize, tab: usize) -> usize {
    let mut display = 0;
    for c in line.chars().take(col) {
        display += if c == '\t' { tab - display % tab } else { 1 };
    }
    display
}

pub fn char_col_for_display(line: &str, target: usize, tab: usize) -> usize {
    let mut display = 0;
    for (i, c) in line.chars().enumerate() {
        let width = if c == '\t' { tab - display % tab } else { 1 };
        if display + width > target {
            return if target - display > width / 2 { i + 1 } else { i };
        }
        display += width;
    }
    line.chars().count()
}

/// Guesses (spaces?, width) from the first indented lines.
fn detect_indent(text: &str) -> Option<(bool, usize)> {
    let mut tabs = 0;
    let mut widths = [0usize; 9];
    for line in text.lines().take(2000) {
        if line.starts_with('\t') {
            tabs += 1;
        } else {
            let spaces = line.chars().take_while(|c| *c == ' ').count();
            if spaces > 0 && spaces < line.len() && !line.trim_start().starts_with('*') {
                for (w, count) in widths.iter_mut().enumerate().skip(2) {
                    if spaces % w == 0 {
                        *count += 1;
                    }
                }
            }
        }
    }
    let space_lines = widths[2];
    if tabs == 0 && space_lines == 0 {
        return None;
    }
    if tabs > space_lines {
        return Some((false, 4));
    }
    // Largest width that still explains most indented lines.
    let width = [8, 4, 2].into_iter().find(|&w| widths[w] * 10 >= space_lines * 9).unwrap_or(4);
    Some((true, width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_undo() {
        let mut buffer = Buffer::new("hello");
        buffer.set_cursor(5, false);
        buffer.type_text("!");
        assert_eq!(buffer.text(), "hello!");
        assert!(buffer.is_dirty());
        buffer.undo();
        assert_eq!(buffer.text(), "hello");
        assert!(!buffer.is_dirty());
        buffer.redo();
        assert_eq!(buffer.text(), "hello!");
    }

    #[test]
    fn multi_cursor_insert() {
        let mut buffer = Buffer::new("a\nb\nc");
        buffer.set_selections(vec![Selection::cursor(0), Selection::cursor(2), Selection::cursor(4)]);
        buffer.type_text("-");
        assert_eq!(buffer.text(), "-a\n-b\n-c");
        assert_eq!(buffer.selections.len(), 3);
    }

    #[test]
    fn auto_close_and_newline() {
        let mut buffer = Buffer::new("fn x() ");
        buffer.set_cursor(7, false);
        buffer.type_text("{");
        assert_eq!(buffer.text(), "fn x() {}");
        buffer.newline();
        assert_eq!(buffer.text(), "fn x() {\n    \n}");
        assert_eq!(buffer.primary().head, 13);
    }

    #[test]
    fn comment_toggle() {
        let mut buffer = Buffer::new("a\n  b");
        buffer.line_comment = Some("//");
        buffer.select_all();
        buffer.toggle_comment();
        assert_eq!(buffer.text(), "// a\n//   b");
        buffer.select_all();
        buffer.toggle_comment();
        assert_eq!(buffer.text(), "a\n  b");
    }

    #[test]
    fn move_line_down() {
        let mut buffer = Buffer::new("one\ntwo\nthree");
        buffer.set_cursor(1, false);
        buffer.move_lines(true);
        assert_eq!(buffer.text(), "two\none\nthree");
        assert_eq!(buffer.primary().head, 5);
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;

    #[test]
    fn delete_with_selection_everywhere() {
        let text = "fn main() {\n    let x = 1;\n}\n";
        let len = text.chars().count();
        for start in 0..=len {
            for end in start..=len {
                for forward in [true, false] {
                    let mut buffer = Buffer::new(text);
                    buffer.set_selections(vec![Selection { anchor: start, head: end }]);
                    if forward {
                        buffer.delete_forward(false)
                    } else {
                        buffer.delete_backward(false)
                    }
                    let _ = buffer.take_char_edits();
                    let _ = buffer.take_byte_edits();
                }
            }
        }
    }

    #[test]
    fn delete_reversed_selection() {
        let mut buffer = Buffer::new("hello world");
        buffer.set_selections(vec![Selection { anchor: 8, head: 2 }]);
        buffer.delete_forward(false);
        assert_eq!(buffer.text(), "herld");
    }
}
