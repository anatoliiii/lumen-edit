//! File tree with lazily loaded directories and git decorations.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, pos2, vec2};
use lumen_shared::fsops::DirEntry;

use super::{App, Dialog, Msg, Tab, util};
use crate::{git, host, theme};

pub enum DirState {
    Loading,
    Loaded(Vec<DirEntry>),
    Error(String),
}

pub struct ExplorerState {
    dirs: HashMap<String, DirState>,
    expanded: HashSet<String>,
    pub selected: Option<String>,
    git: HashMap<String, (char, Color32)>,
    dir_git: HashMap<String, Color32>,
    scroll_to: Option<String>,
    pub filter: String,
}

impl ExplorerState {
    pub fn new(root: &str) -> Self {
        Self {
            dirs: HashMap::new(),
            expanded: HashSet::from([root.to_string()]),
            selected: None,
            git: HashMap::new(),
            dir_git: HashMap::new(),
            scroll_to: None,
            filter: String::new(),
        }
    }

    pub fn mark_loading(&mut self, path: &str) {
        self.dirs.entry(path.to_string()).or_insert(DirState::Loading);
    }

    pub fn loaded(&mut self, path: String, result: Result<Vec<DirEntry>, String>) {
        let state = match result {
            Ok(entries) => DirState::Loaded(entries.into_iter().filter(|e| e.name != ".git").collect()),
            Err(error) => DirState::Error(error),
        };
        self.dirs.insert(path, state);
    }

    /// Loaded directories that are visible (root or expanded).
    pub fn loaded_dirs(&self) -> Vec<String> {
        self.dirs.keys().filter(|d| self.expanded.contains(*d)).cloned().collect()
    }

    pub fn is_loaded(&self, path: &str) -> bool {
        self.dirs.contains_key(path)
    }

    /// Directory for "new file": the selected folder or the selected file's folder.
    pub fn target_dir(&self, root: &str) -> String {
        match &self.selected {
            Some(path) if self.expanded.contains(path) || matches!(self.dirs.get(path), Some(DirState::Loaded(_))) => path.clone(),
            Some(path) => host::parent(path),
            None => root.to_string(),
        }
    }

    /// Expands parents so `path` is visible and scrolls to it. Returns the
    /// directories that still have to be loaded.
    pub fn reveal(&mut self, path: &str, root: &str) -> Vec<String> {
        let mut missing = Vec::new();
        let mut dir = host::parent(path);
        while dir.len() >= root.len() && dir.starts_with(root) {
            self.expanded.insert(dir.clone());
            if !self.dirs.contains_key(&dir) {
                missing.push(dir.clone());
            }
            if dir == root {
                break;
            }
            dir = host::parent(&dir);
        }
        self.selected = Some(path.to_string());
        self.scroll_to = Some(path.to_string());
        missing
    }

    pub fn set_git_status(&mut self, status: &git::Status) {
        self.git.clear();
        self.dir_git.clear();
        for file in &status.files {
            let path = host::join(&status.root, &file.path);
            let (letter, color) = if file.conflict {
                ('!', theme::GIT_CONFLICT)
            } else if file.untracked {
                ('U', theme::GIT_UNTRACKED)
            } else {
                let code = if file.worktree != '.' { file.worktree } else { file.index };
                match code {
                    'A' => ('A', theme::GIT_ADDED),
                    'D' => ('D', theme::GIT_DELETED),
                    'R' => ('R', theme::GIT_ADDED),
                    _ => ('M', theme::GIT_MODIFIED),
                }
            };
            let mut dir = host::parent(&path);
            while dir.len() >= status.root.len() {
                let entry = self.dir_git.entry(dir.clone()).or_insert(color);
                if color == theme::GIT_CONFLICT {
                    *entry = color;
                }
                if dir == status.root {
                    break;
                }
                dir = host::parent(&dir);
            }
            self.git.insert(path, (letter, color));
        }
    }
}

struct Row {
    path: String,
    name: String,
    depth: usize,
    is_dir: bool,
    placeholder: Option<String>,
}

fn collect_rows(state: &ExplorerState, dir: &str, depth: usize, rows: &mut Vec<Row>, filter: &str) {
    match state.dirs.get(dir) {
        None | Some(DirState::Loading) => {
            rows.push(Row {
                path: format!("{dir}/…"), name: String::new(), depth, is_dir: false, placeholder: Some("загрузка…".into())
            })
        }
        Some(DirState::Error(error)) => {
            rows.push(Row { path: format!("{dir}/!"), name: String::new(), depth, is_dir: false, placeholder: Some(error.clone()) })
        }
        Some(DirState::Loaded(entries)) => {
            if entries.is_empty() && depth > 0 {
                rows.push(Row {
                    path: format!("{dir}/∅"), name: String::new(), depth, is_dir: false, placeholder: Some("пусто".into())
                });
            }
            for entry in entries {
                let path = host::join(dir, &entry.name);
                if !filter.is_empty() && !entry.is_dir && !entry.name.to_lowercase().contains(filter) {
                    continue;
                }
                rows.push(Row { path: path.clone(), name: entry.name.clone(), depth, is_dir: entry.is_dir, placeholder: None });
                if entry.is_dir && state.expanded.contains(&path) {
                    collect_rows(state, &path, depth + 1, rows, filter);
                }
            }
        }
    }
}

enum Action {
    Toggle(String),
    Open(String),
    Select(String),
    NewFile(String),
    NewFolder(String),
    Rename(String),
    Delete(String),
    CopyPath(String),
    CopyRelative(String),
    Terminal(String),
    Refresh,
    CollapseAll,
    SendToAgent(String),
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(workspace) = &mut app.workspace else { return };
    let root = workspace.root.clone();
    let mut actions = Vec::new();

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(RichText::new(workspace.name().to_uppercase()).small().strong().color(theme::MUTED_TEXT))
            .on_hover_text(format!("{} — {root}", workspace.host.label()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            if ui.small_button("⊟").on_hover_text("Свернуть все").clicked() {
                actions.push(Action::CollapseAll);
            }
            if ui.small_button("⟳").on_hover_text("Обновить").clicked() {
                actions.push(Action::Refresh);
            }
            if ui.small_button("🗀").on_hover_text("Новая папка").clicked() {
                actions.push(Action::NewFolder(workspace.explorer.target_dir(&root)));
            }
            if ui.small_button("🗋").on_hover_text("Новый файл").clicked() {
                actions.push(Action::NewFile(workspace.explorer.target_dir(&root)));
            }
        });
    });
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add(
            egui::TextEdit::singleline(&mut workspace.explorer.filter).hint_text("Фильтр файлов").desired_width(ui.available_width() - 8.0),
        );
    });
    ui.add_space(4.0);

    let filter = workspace.explorer.filter.to_lowercase();
    let mut rows = Vec::new();
    collect_rows(&workspace.explorer, &root, 0, &mut rows, &filter);
    let row_h = 22.0;
    let active_path = app.active_tab.and_then(|i| match app.tabs.get(i) {
        Some(Tab::Doc(doc)) => Some(doc.path.clone()),
        _ => None,
    });
    let dirty: HashSet<String> =
        app.tabs.iter().filter_map(|t| if let Tab::Doc(d) = t { d.is_dirty().then(|| d.path.clone()) } else { None }).collect();
    let explorer = &mut workspace.explorer;
    let scroll_index = explorer.scroll_to.take().and_then(|target| rows.iter().position(|r| r.path == target));

    // Background for the empty-space context menu. It must be created before
    // the rows so that rows, added later, get the clicks.
    let background_rect = ui.available_rect_before_wrap();
    let background = ui.interact(background_rect, ui.id().with("explorer_empty"), Sense::click());
    let mut scroll = egui::ScrollArea::vertical().id_salt("explorer_scroll").auto_shrink([false, false]);
    if let Some(index) = scroll_index {
        let offset = (index as f32 * row_h - 120.0).max(0.0);
        scroll = scroll.vertical_scroll_offset(offset);
    }
    scroll.show_rows(ui, row_h, rows.len(), |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &rows[range] {
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            let indent = 10.0 + row.depth as f32 * 14.0;
            let painter = ui.painter_at(rect);
            if let Some(text) = &row.placeholder {
                painter.text(
                    pos2(rect.left() + indent + 18.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    text,
                    FontId::proportional(12.0),
                    theme::FAINT_TEXT,
                );
                continue;
            }
            let selected = explorer.selected.as_deref() == Some(row.path.as_str());
            let active = active_path.as_deref() == Some(row.path.as_str());
            let highlight = rect.shrink2(vec2(4.0, 1.0));
            if selected {
                painter.rect_filled(highlight, egui::CornerRadius::same(4), theme::ACCENT_DIM);
            } else if response.hovered() {
                painter.rect_filled(highlight, egui::CornerRadius::same(4), theme::SURFACE);
            } else if active {
                painter.rect_filled(highlight, egui::CornerRadius::same(4), theme::SURFACE.gamma_multiply(0.6));
            }
            // Indent guides, faint.
            for level in 0..row.depth {
                let x = rect.left() + 14.0 + level as f32 * 14.0;
                painter.line_segment(
                    [pos2(x, rect.top()), pos2(x, rect.bottom())],
                    egui::Stroke::new(1.0, theme::INDENT_GUIDE.gamma_multiply(0.8)),
                );
            }
            let x = rect.left() + 8.0 + row.depth as f32 * 14.0;
            let (name_color, badge) = if row.is_dir {
                (explorer.dir_git.get(&row.path).copied().unwrap_or(theme::TEXT), None)
            } else {
                match explorer.git.get(&row.path) {
                    Some((letter, color)) => (*color, Some((*letter, *color))),
                    None => (theme::TEXT, None),
                }
            };
            let center_y = rect.center().y;
            if row.is_dir {
                let open = explorer.expanded.contains(&row.path);
                super::icons::chevron(&painter, pos2(x + 6.0, center_y), open, theme::MUTED_TEXT);
                super::icons::folder(&painter, pos2(x + 21.0, center_y), open);
            } else {
                super::icons::file(&painter, pos2(x + 21.0, center_y), &row.path);
            }
            // Right side: git letter and unsaved dot.
            let mut right = rect.right() - 12.0;
            if let Some((letter, color)) = badge {
                painter.text(pos2(right, center_y), Align2::RIGHT_CENTER, letter.to_string(), FontId::monospace(11.0), color);
                right -= 14.0;
            }
            if dirty.contains(&row.path) {
                painter.circle_filled(pos2(right - 3.0, center_y), 3.5, theme::TEXT);
                right -= 12.0;
            }
            let name_x = x + 33.0;
            let galley = util::elided(&painter, &row.name, FontId::proportional(13.0), name_color, (right - name_x - 4.0).max(20.0));
            painter.galley(pos2(name_x, center_y - galley.size().y / 2.0), galley, name_color);

            if response.clicked() {
                actions.push(Action::Select(row.path.clone()));
                actions.push(if row.is_dir { Action::Toggle(row.path.clone()) } else { Action::Open(row.path.clone()) });
            }
            response.context_menu(|ui| {
                let dir = if row.is_dir { row.path.clone() } else { host::parent(&row.path) };
                if ui.button("Новый файл…").clicked() {
                    actions.push(Action::NewFile(dir.clone()));
                    ui.close();
                }
                if ui.button("Новая папка…").clicked() {
                    actions.push(Action::NewFolder(dir.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Переименовать…").clicked() {
                    actions.push(Action::Rename(row.path.clone()));
                    ui.close();
                }
                if ui.button(RichText::new("Удалить").color(theme::ERROR)).clicked() {
                    actions.push(Action::Delete(row.path.clone()));
                    ui.close();
                }
                ui.separator();
                if ui.button("Копировать путь").clicked() {
                    actions.push(Action::CopyPath(row.path.clone()));
                    ui.close();
                }
                if ui.button("Копировать относительный путь").clicked() {
                    actions.push(Action::CopyRelative(row.path.clone()));
                    ui.close();
                }
                if ui.button("Открыть в терминале").clicked() {
                    actions.push(Action::Terminal(dir));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("π  Отправить в агента").color(theme::ACCENT)).clicked() {
                    actions.push(Action::SendToAgent(row.path.clone()));
                    ui.close();
                }
            });
        }
    });
    // Right-click on empty space.
    background.context_menu(|ui| {
        if ui.button("Новый файл…").clicked() {
            actions.push(Action::NewFile(root.clone()));
            ui.close();
        }
        if ui.button("Новая папка…").clicked() {
            actions.push(Action::NewFolder(root.clone()));
            ui.close();
        }
    });

    for action in actions {
        apply(app, action, &root);
    }
}

fn apply(app: &mut App, action: Action, root: &str) {
    match action {
        Action::Toggle(path) => {
            let Some(workspace) = &mut app.workspace else { return };
            if !workspace.explorer.expanded.remove(&path) {
                workspace.explorer.expanded.insert(path.clone());
                if !workspace.explorer.is_loaded(&path) {
                    app.load_dir(path);
                }
            }
        }
        Action::Open(path) => app.open_file(path, None),
        Action::Select(path) => {
            if let Some(workspace) = &mut app.workspace {
                workspace.explorer.selected = Some(path);
            }
        }
        Action::NewFile(dir) => prompt_create(app, dir, false),
        Action::NewFolder(dir) => prompt_create(app, dir, true),
        Action::Rename(path) => {
            let name = host::file_name(&path).to_string();
            app.dialog = Some(Dialog::Input {
                title: format!("Переименовать «{name}»"),
                value: name,
                focus: true,
                action: Box::new(move |app: &mut App, new_name: String| {
                    let Some(host) = app.host() else { return };
                    let target = host::join(&host::parent(&path), &new_name);
                    let from = path.clone();
                    app.bus.spawn(move || {
                        let result = host.rename(&from, &target);
                        Msg::FsOpDone {
                            reload: vec![host::parent(&from)],
                            open: None,
                            renamed: result.is_ok().then(|| (from.clone(), target.clone())),
                            error: result.err(),
                        }
                    });
                }),
            });
        }
        Action::Delete(path) => {
            let name = host::file_name(&path).to_string();
            app.dialog = Some(Dialog::Confirm {
                title: "Удаление".into(),
                message: format!("Удалить «{name}»? Это действие нельзя отменить."),
                action: Box::new(move |app: &mut App| {
                    let Some(host) = app.host() else { return };
                    // Close tabs for the deleted path.
                    let prefix = format!("{path}/");
                    let closing: Vec<usize> = app
                        .tabs
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| matches!(t, Tab::Doc(d) if d.path == path || d.path.starts_with(&prefix)))
                        .map(|(i, _)| i)
                        .rev()
                        .collect();
                    for index in closing {
                        app.close_tab(index);
                    }
                    let target = path.clone();
                    app.bus.spawn(move || {
                        let result = host.remove(&target);
                        Msg::FsOpDone { reload: vec![host::parent(&target)], open: None, renamed: None, error: result.err() }
                    });
                }),
            });
        }
        Action::CopyPath(path) => app.ctx.copy_text(path),
        Action::CopyRelative(path) => {
            let relative = path.strip_prefix(root).unwrap_or(&path).trim_start_matches('/').to_string();
            app.ctx.copy_text(relative);
        }
        Action::Terminal(dir) => {
            if let Some(host) = app.host() {
                let title = host::file_name(&dir).to_string();
                app.spawn_terminal_in(host, dir, title);
            }
        }
        Action::Refresh => {
            let Some(workspace) = &mut app.workspace else { return };
            let loaded: Vec<String> =
                workspace.explorer.dirs.keys().filter(|dir| *dir == root || workspace.explorer.expanded.contains(*dir)).cloned().collect();
            workspace.explorer.dirs.clear();
            workspace.file_index = None;
            for dir in loaded {
                app.load_dir(dir);
            }
            app.git_refresh();
        }
        Action::SendToAgent(path) => super::agent::attach_file(app, &path),
        Action::CollapseAll => {
            if let Some(workspace) = &mut app.workspace {
                workspace.explorer.expanded.retain(|p| p == root);
            }
        }
    }
}

pub fn prompt_create(app: &mut App, dir: String, folder: bool) {
    app.dialog = Some(Dialog::Input {
        title: if folder {
            "Новая папка".into()
        } else {
            "Новый файл (можно с подпапками: src/mod.rs)".into()
        },
        value: String::new(),
        focus: true,
        action: Box::new(move |app: &mut App, name: String| {
            let Some(host) = app.host() else { return };
            let path = host::join(&dir, name.trim_start_matches('/'));
            let parent = host::parent(&path);
            app.bus.spawn(move || {
                let result = if folder {
                    host.create_dir(&path)
                } else {
                    host.create_dir(&parent).and_then(|_| match host.stat(&path) {
                        Ok(_) => Err("файл уже существует".into()),
                        Err(_) => host.write_file(&path, b""),
                    })
                };
                let first_new = {
                    // Reload the directory directly under `dir` that changed.
                    let mut top = parent.clone();
                    while host::parent(&top).len() > dir.len() {
                        top = host::parent(&top);
                    }
                    if top.len() > dir.len() { host::parent(&top) } else { dir.clone() }
                };
                let error = result.err();
                Msg::FsOpDone { reload: vec![first_new, parent], open: (!folder && error.is_none()).then_some(path), renamed: None, error }
            });
        }),
    });
}
