//! Quick pickers: files (Ctrl+P), commands (Ctrl+Shift+P), go to line,
//! recent projects, tasks and branches.

use eframe::egui::{self, Align2, FontId, Key, RichText, Sense, pos2, vec2};

use super::{App, Command, commands, util};
use crate::{config::RecentProject, git, host, tasks::Task, theme};

#[derive(Clone, PartialEq)]
pub enum Mode {
    Files,
    Commands,
    GotoLine,
    Recent,
    Tasks,
    Branches { merge: bool },
}

#[derive(Clone)]
enum Action {
    OpenFile(String),
    Command(Command),
    GotoLine(usize, usize),
    OpenRecent(RecentProject),
    RunTask(Task),
    Checkout(git::Branch),
    Merge(String),
    CreateBranch(String),
}

#[derive(Clone)]
struct Item {
    label: String,
    detail: String,
    hint: String,
    matches: Vec<u32>,
    action: Action,
}

pub struct Palette {
    mode: Mode,
    query: String,
    selected: usize,
    focus: bool,
    cache: Option<(String, usize, Vec<Item>)>,
}

impl Palette {
    fn new(mode: Mode, query: &str) -> Self {
        Self { mode, query: query.into(), selected: 0, focus: true, cache: None }
    }
    pub fn files() -> Self {
        Self::new(Mode::Files, "")
    }
    pub fn commands() -> Self {
        Self::new(Mode::Commands, "")
    }
    pub fn goto_line() -> Self {
        Self::new(Mode::GotoLine, "")
    }
    pub fn recent() -> Self {
        Self::new(Mode::Recent, "")
    }
    pub fn tasks() -> Self {
        Self::new(Mode::Tasks, "")
    }
    pub fn branches(merge: bool) -> Self {
        Self::new(Mode::Branches { merge }, "")
    }
}

fn placeholder(mode: &Mode) -> &'static str {
    match mode {
        Mode::Files => t!("File name (> commands, : line)"),
        Mode::Commands => t!("Command"),
        Mode::GotoLine => t!("Line number[:column]"),
        Mode::Recent => t!("Recent project"),
        Mode::Tasks => t!("Task to run"),
        Mode::Branches { merge: false } => t!("Branch to switch to (or a new name)"),
        Mode::Branches { merge: true } => t!("Branch to merge into current"),
    }
}

fn build_items(app: &App, mode: &Mode, query: &str) -> Vec<Item> {
    let mut fuzzy = util::Fuzzy::default();
    let mut items: Vec<(u32, Item)> = Vec::new();
    let mut push = |label: String, detail: String, hint: String, action: Action, haystack: Option<&str>| {
        let target = haystack.unwrap_or(&label).to_string();
        if let Some((score, indices)) = fuzzy.score(&target, query) {
            let matches = if haystack.is_none() { indices } else { Vec::new() };
            items.push((score, Item { label, detail, hint, matches, action }));
        }
    };
    match mode {
        Mode::Files => {
            let Some(workspace) = &app.workspace else { return Vec::new() };
            if query.is_empty() {
                // Recently opened tabs first.
                for tab in app.tabs.iter().rev() {
                    if let super::Tab::Doc(doc) = tab {
                        let relative = workspace.relative(&doc.path).to_string();
                        push(doc.name().to_string(), relative, t!("open tab").into(), Action::OpenFile(doc.path.clone()), None);
                    }
                }
            }
            let open: std::collections::HashSet<&str> =
                app.tabs.iter().filter_map(|t| if let super::Tab::Doc(d) = t { Some(d.path.as_str()) } else { None }).collect();
            if let Some(files) = &workspace.file_index {
                for relative in files {
                    if query.is_empty() && items.len() > 60 {
                        break;
                    }
                    if query.is_empty() && open.contains(host::join(&workspace.root, relative).as_str()) {
                        continue;
                    }
                    let name = host::file_name(relative);
                    if let Some((score, indices)) = fuzzy.score(relative, query) {
                        // Highlight only what falls inside the file name.
                        let offset = relative.chars().count() - name.chars().count();
                        let matches = indices.iter().filter(|i| **i as usize >= offset).map(|i| i - offset as u32).collect();
                        let bonus = if fuzzy.score(name, query).is_some() { 1000 } else { 0 };
                        items.push((
                            score + bonus,
                            Item {
                                label: name.to_string(),
                                detail: relative.clone(),
                                hint: String::new(),
                                matches,
                                action: Action::OpenFile(host::join(&workspace.root, relative)),
                            },
                        ));
                    }
                }
            }
        }
        Mode::Commands => {
            for command in commands::ALL {
                push(command.label().into(), String::new(), command.shortcut_text().unwrap_or("").into(), Action::Command(*command), None);
            }
        }
        Mode::GotoLine => {
            let mut parts = query.split([':', ',']);
            let line = parts.next().and_then(|l| l.trim().parse::<usize>().ok());
            let col = parts.next().and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(1);
            if let Some(line) = line {
                items.push((
                    0,
                    Item {
                        label: t!("Go to line {line}, column {col}", line = line, col = col),
                        detail: String::new(),
                        hint: String::new(),
                        matches: Vec::new(),
                        action: Action::GotoLine(line.saturating_sub(1), col.saturating_sub(1)),
                    },
                ));
            }
        }
        Mode::Recent => {
            for recent in &app.config.recent {
                let name = host::file_name(&recent.path).to_string();
                let where_ = if recent.host == "local" {
                    recent.path.clone()
                } else {
                    format!("{} — {}", recent.host.trim_start_matches("ssh:"), recent.path)
                };
                let hint = if recent.host == "local" { t!("local").into() } else { "SSH".into() };
                push(
                    name,
                    where_.clone(),
                    hint,
                    Action::OpenRecent(recent.clone()),
                    Some(&format!("{} {where_}", host::file_name(&recent.path))),
                );
            }
        }
        Mode::Tasks => {
            if let Some(workspace) = &app.workspace {
                for task in &workspace.tasks {
                    push(
                        task.label.clone(),
                        task.command.clone(),
                        format!("{} · {}", task.group.label(), task.source),
                        Action::RunTask(task.clone()),
                        None,
                    );
                }
            }
        }
        Mode::Branches { merge } => {
            if let Some(workspace) = &app.workspace {
                for branch in &workspace.git.branches {
                    if branch.current {
                        continue;
                    }
                    let hint = if branch.remote {
                        t!("remote")
                    } else if !branch.worktree.is_empty() {
                        t!("worktree")
                    } else {
                        ""
                    };
                    let action = if *merge { Action::Merge(branch.name.clone()) } else { Action::Checkout(branch.clone()) };
                    push(
                        branch.name.clone(),
                        format!("{} {}", branch.commit, branch.subject),
                        format!("{hint} {}", branch.date),
                        action,
                        None,
                    );
                }
                if !*merge && !query.trim().is_empty() && !workspace.git.branches.iter().any(|b| b.name == query.trim()) {
                    let name = query.trim().replace(' ', "-");
                    items.push((
                        u32::MAX,
                        Item {
                            label: t!("+ Create branch “{name}”", name = name),
                            detail: String::new(),
                            hint: String::new(),
                            matches: Vec::new(),
                            action: Action::CreateBranch(name),
                        },
                    ));
                }
            }
        }
    }
    if !query.is_empty() {
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.detail.len().cmp(&b.1.detail.len())));
    }
    items.truncate(300);
    items.into_iter().map(|(_, item)| item).collect()
}

pub fn show(ctx: &egui::Context, app: &mut App) {
    let Some(mut palette) = app.palette.take() else { return };
    // Prefix switching like VS Code.
    if palette.mode == Mode::Files {
        if let Some(rest) = palette.query.strip_prefix('>') {
            palette = Palette::new(Mode::Commands, rest);
        } else if let Some(rest) = palette.query.strip_prefix(':') {
            palette = Palette::new(Mode::GotoLine, rest);
        }
    }

    let index_len = app.workspace.as_ref().and_then(|w| w.file_index.as_ref()).map(|f| f.len()).unwrap_or(0);
    let stale = palette.cache.as_ref().is_none_or(|(q, len, _)| *q != palette.query || *len != index_len);
    if stale {
        let items = build_items(app, &palette.mode, &palette.query);
        palette.cache = Some((palette.query.clone(), index_len, items));
        palette.selected = 0;
    }
    let items = palette.cache.as_ref().map(|(_, _, items)| items.clone()).unwrap_or_default();

    let mut chosen: Option<Action> = None;
    let mut close = false;
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
            palette.selected = (palette.selected + 1).min(items.len().saturating_sub(1));
        }
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
            palette.selected = palette.selected.saturating_sub(1);
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
            chosen = items.get(palette.selected).map(|item| item.action.clone());
            close = true;
        }
    });

    let screen = ctx.content_rect();
    let width = (screen.width() * 0.5).clamp(420.0, 720.0);
    let area = egui::Area::new(egui::Id::new("palette"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos2(screen.center().x - width / 2.0, screen.top() + 46.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(width);
                let input = ui.add(
                    egui::TextEdit::singleline(&mut palette.query)
                        .hint_text(placeholder(&palette.mode))
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Body)
                        .margin(egui::Margin::symmetric(8, 6)),
                );
                if palette.focus {
                    input.request_focus();
                    palette.focus = false;
                }
                if palette.mode == Mode::Files && app.workspace.as_ref().is_some_and(|w| w.file_index_loading) {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(t!("Indexing files…")).small().color(theme::MUTED_TEXT));
                    });
                }
                ui.add_space(4.0);
                let row_h = 40.0;
                let compact = matches!(palette.mode, Mode::Commands | Mode::GotoLine);
                let row_h = if compact { 26.0 } else { row_h };
                egui::ScrollArea::vertical().max_height(420.0).auto_shrink([false, true]).show_rows(ui, row_h, items.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for index in range {
                        let item = &items[index];
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        let selected = index == palette.selected;
                        if selected {
                            ui.painter().rect_filled(rect, 4.0, theme::ACCENT_DIM);
                        } else if response.hovered() {
                            ui.painter().rect_filled(rect, 4.0, theme::SURFACE_HOVER);
                        }
                        let painter = ui.painter_at(rect);
                        let mut x = rect.left() + 10.0;
                        if matches!(palette.mode, Mode::Files) {
                            let path = if item.detail.is_empty() { item.label.clone() } else { format!("/{}", item.detail) };
                            super::icons::file(&painter, pos2(x + 6.0, rect.center().y), &path);
                            x += 20.0;
                        }
                        let job = util::highlighted_label(&item.label, &item.matches, theme::TEXT, theme::ACCENT, 13.5);
                        let galley = painter.layout_job(job);
                        if compact || item.detail.is_empty() {
                            painter.galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, theme::TEXT);
                        } else {
                            painter.galley(pos2(x, rect.top() + 4.0), galley, theme::TEXT);
                            painter.text(
                                pos2(x, rect.bottom() - 5.0),
                                Align2::LEFT_BOTTOM,
                                util::truncate_middle(&item.detail, 90),
                                FontId::proportional(11.5),
                                theme::MUTED_TEXT,
                            );
                        }
                        if !item.hint.is_empty() {
                            painter.text(
                                rect.right_center() - vec2(10.0, 0.0),
                                Align2::RIGHT_CENTER,
                                &item.hint,
                                FontId::proportional(11.5),
                                theme::FAINT_TEXT,
                            );
                        }
                        if selected {
                            response.scroll_to_me(None);
                        }
                        if response.clicked() {
                            chosen = Some(item.action.clone());
                            close = true;
                        }
                    }
                });
                if items.is_empty() && !palette.query.is_empty() {
                    ui.label(RichText::new(t!("Nothing found")).color(theme::MUTED_TEXT));
                }
            });
        });

    // Clicking outside closes.
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if outside {
        close = true;
    }

    if !close {
        app.palette = Some(palette);
    } else if let Some(doc) = app.active_doc() {
        doc.view.focus_request = true;
    }
    if let Some(action) = chosen {
        match action {
            Action::OpenFile(path) => app.open_file(path, None),
            Action::Command(command) => app.run_command(command),
            Action::GotoLine(line, col) => {
                if let Some(doc) = app.active_doc() {
                    let at = doc.buffer.char_at(line, col);
                    doc.buffer.set_cursor(at, false);
                    doc.view.reveal = true;
                    doc.view.reveal_center = true;
                    doc.view.focus_request = true;
                }
            }
            Action::OpenRecent(recent) => super::welcome::open_recent(app, recent),
            Action::RunTask(task) => app.run_task(task),
            Action::Checkout(branch) => super::scm::checkout(app, branch),
            Action::Merge(name) => app.git_op("merge", move |h, r| git::merge(h, r, &name)),
            Action::CreateBranch(name) => app.git_op("switch -c", move |h, r| git::create_branch(h, r, &name, true)),
        }
    }
}
