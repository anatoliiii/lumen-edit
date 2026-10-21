//! Read-only tabs: file diffs with per-hunk staging, commit details and the
//! history graph.

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke, pos2, vec2};

use super::{App, Tab, util};
use crate::{git, theme};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffMode {
    Unstaged,
    Staged,
    Untracked,
    Commit(String),
}

impl DiffMode {
    pub fn label(&self) -> &str {
        match self {
            DiffMode::Unstaged => t!("changes"),
            DiffMode::Staged => t!("staged"),
            DiffMode::Untracked => t!("new"),
            DiffMode::Commit(_) => t!("commit"),
        }
    }
}

pub struct DiffTab {
    pub title: String,
    pub key: String,
    /// Repository-relative path for file diffs.
    pub path: Option<String>,
    pub mode: DiffMode,
    pub preamble: Vec<String>,
    pub files: Vec<git::DiffFile>,
    pub loading: bool,
    pub error: Option<String>,
}

impl DiffTab {
    pub fn new(title: String, key: String, path: Option<String>, mode: DiffMode) -> Self {
        Self { title, key, path, mode, preamble: Vec::new(), files: Vec::new(), loading: true, error: None }
    }

    pub fn set_content(&mut self, result: Result<String, String>) {
        self.loading = false;
        match result {
            Ok(text) => {
                // `git show` output starts with the commit header and stat.
                let first_diff = text.find("\ndiff --git").map(|i| i + 1).unwrap_or(if text.starts_with("diff") { 0 } else { text.len() });
                self.preamble = text[..first_diff].lines().map(str::to_string).collect();
                self.files = git::parse_diff(&text[first_diff..]);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}

enum HunkAction {
    Stage(usize, usize),
    Unstage(usize, usize),
    Revert(usize, usize),
    Open(String, usize),
}

pub fn show_diff(ui: &mut egui::Ui, app: &mut App, index: usize) {
    let Some(Tab::Diff(tab)) = app.tabs.get(index) else { return };
    let font_size = app.config.editor_font_size - 1.0;
    let mono = FontId::monospace(font_size);
    let mut action = None;
    let mode = tab.mode.clone();

    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&tab.title).strong());
            if tab.loading {
                ui.spinner();
            }
            if let Some(path) = &tab.path
                && ui.small_button(t!("Open File")).clicked()
            {
                action = Some(HunkAction::Open(path.clone(), 0));
            }
        });
    });
    if let Some(error) = &tab.error {
        ui.label(RichText::new(error).color(theme::ERROR));
    }
    let row_h = ui.fonts_mut(|f| f.row_height(&mono)) + 2.0;

    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for line in &tab.preamble {
            ui.label(RichText::new(line).font(mono.clone()).color(theme::MUTED_TEXT));
        }
        if tab.files.is_empty() && !tab.loading && tab.error.is_none() {
            ui.label(RichText::new(t!("No changes")).color(theme::MUTED_TEXT));
        }
        for (file_index, file) in tab.files.iter().enumerate() {
            ui.add_space(8.0);
            egui::Frame::new().fill(theme::SURFACE).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&file.path).strong());
                    if matches!(mode, DiffMode::Commit(_)) && ui.small_button(t!("Open")).clicked() {
                        action = Some(HunkAction::Open(file.path.clone(), 0));
                    }
                });
            });
            for (hunk_index, hunk) in file.hunks.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().min(600.0), row_h + 4.0), Sense::hover());
                    ui.painter().rect_filled(rect, 0.0, theme::DIFF_HUNK_BG);
                    ui.painter().text(rect.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &hunk.header, mono.clone(), theme::INFO);
                    match mode {
                        DiffMode::Unstaged => {
                            if ui.small_button(t!("+ Stage")).clicked() {
                                action = Some(HunkAction::Stage(file_index, hunk_index));
                            }
                            if ui.small_button(t!("↺ Discard")).clicked() {
                                action = Some(HunkAction::Revert(file_index, hunk_index));
                            }
                        }
                        DiffMode::Staged if ui.small_button(t!("− Unstage")).clicked() => {
                            action = Some(HunkAction::Unstage(file_index, hunk_index));
                        }
                        _ => {}
                    }
                    if ui.small_button("→").on_hover_text(t!("Go to this place in the file")).clicked() {
                        action = Some(HunkAction::Open(file.path.clone(), hunk.new_start.saturating_sub(1)));
                    }
                });
                let mut old_line = hunk.header.split(['-', ',', ' ']).nth(2).and_then(|n| n.parse::<usize>().ok()).unwrap_or(1);
                let mut new_line = hunk.new_start;
                for line in &hunk.lines {
                    let (bg, fg, old, new) = match line.chars().next() {
                        Some('+') => {
                            new_line += 1;
                            (theme::DIFF_ADDED_BG, Color32::from_rgb(170, 220, 170), String::new(), (new_line - 1).to_string())
                        }
                        Some('-') => {
                            old_line += 1;
                            (theme::DIFF_REMOVED_BG, Color32::from_rgb(235, 170, 170), (old_line - 1).to_string(), String::new())
                        }
                        Some('\\') => (Color32::TRANSPARENT, theme::MUTED_TEXT, String::new(), String::new()),
                        _ => {
                            old_line += 1;
                            new_line += 1;
                            (Color32::TRANSPARENT, theme::TEXT, (old_line - 1).to_string(), (new_line - 1).to_string())
                        }
                    };
                    let galley = ui.painter().layout_no_wrap(line.replace('\t', "    "), mono.clone(), fg);
                    let width = (galley.size().x + 120.0).max(ui.available_width());
                    let (rect, _) = ui.allocate_exact_size(vec2(width, row_h), Sense::hover());
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(rect, 0.0, bg);
                    }
                    ui.painter().text(rect.left_center() + vec2(36.0, 0.0), Align2::RIGHT_CENTER, old, mono.clone(), theme::LINE_NUMBER);
                    ui.painter().text(rect.left_center() + vec2(76.0, 0.0), Align2::RIGHT_CENTER, new, mono.clone(), theme::LINE_NUMBER);
                    ui.painter().galley(pos2(rect.left() + 90.0, rect.top() + 1.0), galley, fg);
                }
            }
        }
    });

    let Some(action) = action else { return };
    let Some(Tab::Diff(tab)) = app.tabs.get(index) else { return };
    let patch_for = |file: usize, hunk: usize| git::hunk_patch(&tab.files[file], &tab.files[file].hunks[hunk]);
    match action {
        HunkAction::Stage(f, h) => {
            let patch = patch_for(f, h);
            app.git_op("stage hunk", move |host, repo| git::apply_patch(host, repo, &patch, false, true).map(|_| String::new()));
        }
        HunkAction::Unstage(f, h) => {
            let patch = patch_for(f, h);
            app.git_op("unstage hunk", move |host, repo| git::apply_patch(host, repo, &patch, true, true).map(|_| String::new()));
        }
        HunkAction::Revert(f, h) => {
            let patch = patch_for(f, h);
            app.dialog = Some(super::Dialog::Confirm {
                title: t!("Discard hunk").into(),
                message: t!("Changes in this hunk will be lost.").into(),
                action: Box::new(move |app: &mut App| {
                    app.git_op("revert hunk", move |host, repo| git::apply_patch(host, repo, &patch, true, false).map(|_| String::new()));
                }),
            });
        }
        HunkAction::Open(path, line) => {
            if let Some(repo) = app.workspace.as_ref().and_then(|w| w.git.repo_root.clone()) {
                app.open_file(crate::host::join(&repo, &path), Some((line, 0)));
            }
        }
    }
}

pub struct LogTab {
    pub all: bool,
    pub entries: Vec<git::LogEntry>,
    pub loading: bool,
    pub error: Option<String>,
    pub filter: String,
}

impl LogTab {
    pub fn new(all: bool) -> Self {
        Self { all, entries: Vec::new(), loading: true, error: None, filter: String::new() }
    }
}

pub fn show_log(ui: &mut egui::Ui, app: &mut App, index: usize) {
    let font_size = app.config.editor_font_size - 1.0;
    let mono = FontId::monospace(font_size);
    let mut open: Option<(String, String)> = None;
    let mut reload = false;
    let mut checkout: Option<String> = None;
    let mut cherry_pick: Option<String> = None;
    let mut revert: Option<String> = None;
    let mut reset: Option<String> = None;
    let Some(Tab::Log(log)) = app.tabs.get_mut(index) else { return };

    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("History")).strong());
            if ui.checkbox(&mut log.all, t!("All branches")).changed() {
                reload = true;
            }
            ui.add(egui::TextEdit::singleline(&mut log.filter).hint_text(t!("Filter (text, author, hash)")).desired_width(260.0));
            if ui.small_button("⟳").on_hover_text(t!("Refresh")).clicked() {
                reload = true;
            }
            if log.loading {
                ui.spinner();
            }
        });
    });
    if let Some(error) = &log.error {
        ui.label(RichText::new(error).color(theme::ERROR));
    }

    let filter = log.filter.to_lowercase();
    let rows: Vec<&git::LogEntry> = log
        .entries
        .iter()
        .filter(|e| {
            filter.is_empty()
                || e.subject.to_lowercase().contains(&filter)
                || e.author.to_lowercase().contains(&filter)
                || e.hash.starts_with(&filter)
                || e.refs.to_lowercase().contains(&filter)
        })
        .collect();
    let row_h = 22.0;
    let graph_w =
        rows.iter().map(|r| r.graph.chars().count()).max().unwrap_or(2) as f32 * ui.fonts_mut(|f| f.glyph_width(&mono, 'M')) + 12.0;

    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_h, rows.len(), |ui, range| {
        for entry in &rows[range] {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            if response.hovered() {
                ui.painter().rect_filled(rect, 0.0, theme::SURFACE_HOVER);
            }
            let painter = ui.painter_at(rect);
            // Graph characters colored per column.
            let mut x = rect.left() + 8.0;
            let char_w = ui.fonts_mut(|f| f.glyph_width(&mono, 'M'));
            for (column, c) in entry.graph.chars().enumerate() {
                let color = lane_color(column / 2);
                let glyph = match c {
                    '*' => "●",
                    '|' => "│",
                    '/' => "╱",
                    '\\' => "╲",
                    '_' => "_",
                    _ => " ",
                };
                painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, glyph, mono.clone(), color);
                x += char_w;
            }
            if entry.hash.is_empty() {
                continue;
            }
            let mut x = rect.left() + graph_w;
            for reference in entry.refs.split(", ").filter(|r| !r.is_empty()) {
                let label = reference.replace("HEAD -> ", "➤ ");
                let color = if reference.contains("HEAD") {
                    theme::ACCENT
                } else if reference.starts_with("tag:") {
                    theme::WARNING
                } else if reference.contains('/') {
                    theme::MUTED_TEXT
                } else {
                    theme::GIT_ADDED
                };
                let galley = painter.layout_no_wrap(label, FontId::proportional(11.5), color);
                let badge = egui::Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(galley.size().x + 10.0, row_h - 6.0));
                painter.rect_stroke(badge, 3.0, Stroke::new(1.0, color.gamma_multiply(0.6)), egui::StrokeKind::Inside);
                painter.galley(pos2(x + 5.0, rect.center().y - galley.size().y / 2.0), galley, color);
                x += badge.width() + 6.0;
            }
            painter.text(pos2(x + 2.0, rect.center().y), Align2::LEFT_CENTER, &entry.subject, FontId::proportional(13.0), theme::TEXT);
            let meta = format!("{}  {}  {}", entry.author, entry.date, entry.short);
            painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, meta, FontId::proportional(12.0), theme::MUTED_TEXT);
            if response.clicked() {
                open = Some((entry.hash.clone(), entry.subject.clone()));
            }
            response.context_menu(|ui| {
                if ui.button(t!("Show changes")).clicked() {
                    open = Some((entry.hash.clone(), entry.subject.clone()));
                    ui.close();
                }
                if ui.button(t!("Copy hash")).clicked() {
                    ui.ctx().copy_text(entry.hash.clone());
                    ui.close();
                }
                ui.separator();
                if ui.button(t!("Check out this commit (detached)")).clicked() {
                    checkout = Some(entry.hash.clone());
                    ui.close();
                }
                if ui.button(t!("Cherry-pick onto current branch")).clicked() {
                    cherry_pick = Some(entry.hash.clone());
                    ui.close();
                }
                if ui.button(t!("Revert (inverse commit)")).clicked() {
                    revert = Some(entry.hash.clone());
                    ui.close();
                }
                if ui.button(RichText::new(t!("Reset --hard to this commit")).color(theme::ERROR)).clicked() {
                    reset = Some(entry.hash.clone());
                    ui.close();
                }
            });
        }
    });

    if reload {
        app.reload_log();
    }
    if let Some((hash, subject)) = open {
        app.open_commit(hash, subject);
    }
    if let Some(hash) = checkout {
        app.git_op("checkout", move |h, r| git::git_verbose(h, r, &["switch", "--detach", &hash]));
    }
    if let Some(hash) = cherry_pick {
        app.git_op("cherry-pick", move |h, r| git::git_verbose(h, r, &["cherry-pick", &hash]));
    }
    if let Some(hash) = revert {
        app.git_op("revert", move |h, r| git::git_verbose(h, r, &["revert", "--no-edit", &hash]));
    }
    if let Some(hash) = reset {
        app.dialog = Some(super::Dialog::Confirm {
            title: "git reset --hard".into(),
            message: t!("All uncommitted changes will be lost and the branch will move to {n1}.", n1 = util::truncate_middle(&hash, 12)),
            action: Box::new(move |app: &mut App| app.git_op("reset", move |h, r| git::git_verbose(h, r, &["reset", "--hard", &hash]))),
        });
    }
}

fn lane_color(lane: usize) -> Color32 {
    const COLORS: [Color32; 6] = [
        Color32::from_rgb(97, 175, 239),
        Color32::from_rgb(152, 195, 121),
        Color32::from_rgb(229, 192, 123),
        Color32::from_rgb(198, 120, 221),
        Color32::from_rgb(224, 108, 117),
        Color32::from_rgb(86, 182, 194),
    ];
    COLORS[lane % COLORS.len()]
}
