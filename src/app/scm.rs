//! Source control sidebar: changes, commit, branches, stashes, merge state.

use eframe::egui::{self, Align2, FontId, RichText, Sense, pos2, vec2};

use super::{App, Dialog, diffview::DiffMode, palette::Palette, util};
use crate::{
    git::{self, FileStatus, RepoState},
    host, theme,
};

#[derive(Default)]
pub struct GitState {
    pub repo_root: Option<String>,
    pub checked: bool,
    pub status: Option<git::Status>,
    pub loading: bool,
    pub refresh_again: bool,
    pub busy: Option<String>,
    pub error: Option<String>,
    pub commit_message: String,
    pub amend: bool,
    pub branches: Vec<git::Branch>,
    pub stashes: Vec<git::StashEntry>,
    hide_staged: bool,
    hide_changes: bool,
    show_branches: bool,
    show_stashes: bool,
}

enum Action {
    Stage(Vec<String>),
    Unstage(Vec<String>),
    StageAll,
    UnstageAll,
    Discard(Vec<FileStatus>),
    Diff(String, DiffMode),
    Open(String),
    TakeSide(String, bool),
    Commit,
    Push,
    Pull,
    Fetch,
    Sync,
    Abort,
    Continue,
    Checkout(git::Branch),
    DeleteBranch(String),
    MergeBranch(String),
    RebaseOnto(String),
    NewBranch,
    Stash,
    StashAction(&'static str, String),
    Log,
    Init,
    Refresh,
    AmendLoad,
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(workspace) = &mut app.workspace else { return };
    let git = &mut workspace.git;
    let mut actions = Vec::new();

    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("GIT").small().strong().color(theme::MUTED_TEXT));
            if git.loading || git.busy.is_some() {
                ui.spinner();
                if let Some(busy) = &git.busy {
                    ui.label(RichText::new(busy).small().color(theme::MUTED_TEXT));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if git.repo_root.is_some() {
                    ui.menu_button("⋯", |ui| {
                        if ui.button("Pull").clicked() {
                            actions.push(Action::Pull);
                            ui.close();
                        }
                        if ui.button("Push").clicked() {
                            actions.push(Action::Push);
                            ui.close();
                        }
                        if ui.button("Fetch").clicked() {
                            actions.push(Action::Fetch);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Новая ветка…").clicked() {
                            actions.push(Action::NewBranch);
                            ui.close();
                        }
                        if ui.button("Спрятать изменения (stash)").clicked() {
                            actions.push(Action::Stash);
                            ui.close();
                        }
                        if ui.button("История (граф)").clicked() {
                            actions.push(Action::Log);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Добавить всё в индекс").clicked() {
                            actions.push(Action::StageAll);
                            ui.close();
                        }
                        if ui.button("Убрать всё из индекса").clicked() {
                            actions.push(Action::UnstageAll);
                            ui.close();
                        }
                    });
                    if ui.small_button("⎘").on_hover_text("История").clicked() {
                        actions.push(Action::Log);
                    }
                    if ui.small_button("⟳").on_hover_text("Обновить").clicked() {
                        actions.push(Action::Refresh);
                    }
                }
            });
        });
    });

    if git.repo_root.is_none() {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            if !git.checked {
                ui.spinner();
            } else {
                ui.label(RichText::new("Папка не является git-репозиторием.").color(theme::MUTED_TEXT));
                ui.add_space(8.0);
                if ui.button("Инициализировать репозиторий").clicked() {
                    actions.push(Action::Init);
                }
            }
        });
        drop_actions(app, actions);
        return;
    }

    let status = git.status.clone().unwrap_or_default();

    // Branch line with sync.
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let branch = if status.detached { "(detached HEAD)".to_string() } else { status.branch.clone() };
            if ui.button(format!("⎇ {branch}")).on_hover_text("Переключить ветку").clicked() {
                actions.push(Action::Checkout(git::Branch { name: String::new(), remote: false, current: false, upstream: String::new(), commit: String::new(), date: String::new(), subject: String::new() }));
            }
            let sync_label = match (&status.upstream, status.ahead, status.behind) {
                (None, _, _) => "↑ Опубликовать".to_string(),
                (Some(_), 0, 0) => "⟳".to_string(),
                (Some(_), ahead, behind) => format!("⟳ {behind}↓ {ahead}↑"),
            };
            let hint = if status.upstream.is_none() { "Отправить ветку на origin" } else { "Синхронизировать: pull, затем push" };
            if ui.button(sync_label).on_hover_text(hint).clicked() {
                actions.push(if status.upstream.is_none() { Action::Push } else { Action::Sync });
            }
        });
    });

    // Merge / rebase in progress.
    if status.state != RepoState::Clean {
        egui::Frame::new().fill(theme::SURFACE).inner_margin(egui::Margin::same(8)).outer_margin(egui::Margin::symmetric(8, 6)).corner_radius(4.0).show(ui, |ui| {
            let label = match status.state {
                RepoState::Merging => "Идёт слияние",
                RepoState::Rebasing => "Идёт rebase",
                RepoState::CherryPicking => "Идёт cherry-pick",
                RepoState::Reverting => "Идёт revert",
                RepoState::Clean => "",
            };
            ui.label(RichText::new(label).color(theme::WARNING).strong());
            let conflicts = status.files.iter().filter(|f| f.conflict).count();
            if conflicts > 0 {
                ui.label(RichText::new(format!("Конфликтов: {conflicts}. Разрешите их в редакторе и добавьте в индекс.")).small());
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(conflicts == 0, egui::Button::new("Продолжить")).clicked() {
                    actions.push(Action::Continue);
                }
                if ui.button(RichText::new("Прервать").color(theme::ERROR)).clicked() {
                    actions.push(Action::Abort);
                }
            });
        });
    }

    // Commit box.
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        let hint = if git.amend { "Новое сообщение для последнего коммита" } else { "Сообщение коммита (Ctrl+Enter)" };
        let editor = ui.add(egui::TextEdit::multiline(&mut git.commit_message).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY));
        if editor.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            actions.push(Action::Commit);
        }
        ui.horizontal(|ui| {
            let staged = status.files.iter().filter(|f| f.staged()).count();
            let label = if git.amend {
                "✓ Исправить коммит".to_string()
            } else if staged == 0 && !status.files.is_empty() {
                "✓ Коммит (всё)".to_string()
            } else {
                "✓ Коммит".to_string()
            };
            let enabled = !git.commit_message.trim().is_empty() && (!status.files.is_empty() || git.amend) && git.busy.is_none();
            if ui.add_enabled(enabled, egui::Button::new(RichText::new(label).strong()).fill(theme::ACCENT_DIM)).clicked() {
                actions.push(Action::Commit);
            }
            if ui.checkbox(&mut git.amend, "amend").on_hover_text("Изменить последний коммит").changed() && git.amend && git.commit_message.is_empty() {
                actions.push(Action::AmendLoad);
            }
        });
        if let Some(error) = &git.error {
            ui.label(RichText::new(error).small().color(theme::ERROR));
        }
    });

    let conflicts: Vec<&FileStatus> = status.files.iter().filter(|f| f.conflict).collect();
    let staged: Vec<&FileStatus> = status.files.iter().filter(|f| f.staged()).collect();
    let changes: Vec<&FileStatus> = status.files.iter().filter(|f| f.unstaged()).collect();

    egui::ScrollArea::vertical().id_salt("scm_scroll").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        if !conflicts.is_empty() {
            section_header(ui, "КОНФЛИКТЫ", conflicts.len(), &mut false, |_| {});
            for file in &conflicts {
                file_row(ui, file, false, &mut actions, true);
            }
        }
        if !staged.is_empty() {
            let mut hide = git.hide_staged;
            section_header(ui, "В ИНДЕКСЕ", staged.len(), &mut hide, |ui| {
                if ui.small_button("−").on_hover_text("Убрать всё из индекса").clicked() {
                    actions.push(Action::UnstageAll);
                }
            });
            git.hide_staged = hide;
            if !hide {
                for file in &staged {
                    file_row(ui, file, true, &mut actions, false);
                }
            }
        }
        let mut hide = git.hide_changes;
        section_header(ui, "ИЗМЕНЕНИЯ", changes.len(), &mut hide, |ui| {
            if !changes.is_empty() {
                if ui.small_button("+").on_hover_text("Добавить всё").clicked() {
                    actions.push(Action::StageAll);
                }
                if ui.small_button("↺").on_hover_text("Отменить все изменения").clicked() {
                    actions.push(Action::Discard(changes.iter().map(|f| (*f).clone()).collect()));
                }
            }
        });
        git.hide_changes = hide;
        if !hide {
            if changes.is_empty() && staged.is_empty() && conflicts.is_empty() {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new("Рабочая копия чистая").small().color(theme::FAINT_TEXT));
                });
            }
            for file in &changes {
                file_row(ui, file, false, &mut actions, false);
            }
        }

        // Branches.
        ui.add_space(6.0);
        let mut show_branches = git.show_branches;
        let local: Vec<&git::Branch> = git.branches.iter().filter(|b| !b.remote).collect();
        let remote: Vec<&git::Branch> = git.branches.iter().filter(|b| b.remote).collect();
        let mut collapsed = !show_branches;
        section_header(ui, "ВЕТКИ", local.len(), &mut collapsed, |ui| {
            if ui.small_button("+").on_hover_text("Новая ветка").clicked() {
                actions.push(Action::NewBranch);
            }
        });
        show_branches = !collapsed;
        git.show_branches = show_branches;
        if show_branches {
            for branch in local.iter().chain(remote.iter()) {
                branch_row(ui, branch, &mut actions);
            }
        }

        // Stashes.
        let mut collapsed = !git.show_stashes;
        section_header(ui, "STASH", git.stashes.len(), &mut collapsed, |ui| {
            if ui.small_button("+").on_hover_text("Спрятать изменения").clicked() {
                actions.push(Action::Stash);
            }
        });
        git.show_stashes = !collapsed;
        if git.show_stashes {
            for stash in &git.stashes {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(&stash.message).small()).on_hover_text(format!("{} — {}", stash.name, stash.date));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(6.0);
                        if ui.small_button("✕").on_hover_text("Удалить").clicked() {
                            actions.push(Action::StashAction("drop", stash.name.clone()));
                        }
                        if ui.small_button("↓").on_hover_text("Применить и удалить (pop)").clicked() {
                            actions.push(Action::StashAction("pop", stash.name.clone()));
                        }
                        if ui.small_button("✓").on_hover_text("Применить (apply)").clicked() {
                            actions.push(Action::StashAction("apply", stash.name.clone()));
                        }
                    });
                });
            }
        }
        ui.add_space(20.0);
    });

    drop_actions(app, actions);
}

fn section_header(ui: &mut egui::Ui, title: &str, count: usize, collapsed: &mut bool, buttons: impl FnOnce(&mut egui::Ui)) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
    }
    ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, if *collapsed { "⏵" } else { "⏷" }, FontId::proportional(10.0), theme::MUTED_TEXT);
    ui.painter().text(rect.left_center() + vec2(24.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(11.5), theme::MUTED_TEXT);
    let badge = count.to_string();
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, badge, FontId::proportional(11.5), theme::FAINT_TEXT);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(30.0, 2.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    buttons(&mut child);
    if response.clicked() {
        *collapsed = !*collapsed;
    }
}

fn file_row(ui: &mut egui::Ui, file: &FileStatus, staged: bool, actions: &mut Vec<Action>, conflict: bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    let hovered = ui.rect_contains_pointer(rect);
    if hovered {
        ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
    }
    let name = host::file_name(&file.path);
    let dir = file.path.strip_suffix(name).unwrap_or("").trim_end_matches('/');
    let (icon, icon_color) = util::file_icon(name);
    let letter = file.letter(staged);
    let color = match letter {
        'A' | 'U' => theme::GIT_ADDED,
        'D' => theme::GIT_DELETED,
        '!' => theme::GIT_CONFLICT,
        _ => theme::GIT_MODIFIED,
    };
    let painter = ui.painter_at(rect);
    painter.text(rect.left_center() + vec2(24.0, 0.0), Align2::CENTER_CENTER, icon, FontId::monospace(11.0), icon_color);
    let name_color = if letter == 'D' { theme::MUTED_TEXT } else { theme::TEXT };
    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(13.0), name_color);
    let w = galley.size().x;
    painter.galley(rect.left_center() + vec2(34.0, -galley.size().y / 2.0), galley, name_color);
    painter.text(rect.left_center() + vec2(40.0 + w, 0.0), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme::FAINT_TEXT);
    painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, letter.to_string(), FontId::monospace(12.0), color);

    if hovered {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(26.0, 1.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        let paths = vec![file.path.clone()];
        if conflict {
            if child.small_button("+").on_hover_text("Отметить как разрешённый").clicked() {
                actions.push(Action::Stage(paths.clone()));
            }
            if child.small_button("их").on_hover_text("Взять входящую версию целиком").clicked() {
                actions.push(Action::TakeSide(file.path.clone(), false));
            }
            if child.small_button("моё").on_hover_text("Взять текущую версию целиком").clicked() {
                actions.push(Action::TakeSide(file.path.clone(), true));
            }
        } else if staged {
            if child.small_button("−").on_hover_text("Убрать из индекса").clicked() {
                actions.push(Action::Unstage(paths.clone()));
            }
        } else {
            if child.small_button("+").on_hover_text("Добавить в индекс").clicked() {
                actions.push(Action::Stage(paths.clone()));
            }
            if child.small_button("↺").on_hover_text("Отменить изменения").clicked() {
                actions.push(Action::Discard(vec![file.clone()]));
            }
        }
        if child.small_button("🗋").on_hover_text("Открыть файл").clicked() {
            actions.push(Action::Open(file.path.clone()));
        }
    }
    if response.clicked() {
        if conflict || file.worktree == 'D' && !staged {
            actions.push(Action::Open(file.path.clone()));
        } else {
            let mode = if staged {
                DiffMode::Staged
            } else if file.untracked {
                DiffMode::Untracked
            } else {
                DiffMode::Unstaged
            };
            actions.push(Action::Diff(file.path.clone(), mode));
        }
    }
    if response.double_clicked() {
        actions.push(Action::Open(file.path.clone()));
    }
}

fn branch_row(ui: &mut egui::Ui, branch: &git::Branch, actions: &mut Vec<Action>) {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, theme::SURFACE);
    }
    let painter = ui.painter_at(rect);
    let color = if branch.current {
        theme::ACCENT
    } else if branch.remote {
        theme::MUTED_TEXT
    } else {
        theme::TEXT
    };
    let marker = if branch.current { "●" } else if branch.remote { "⇅" } else { "⎇" };
    painter.text(rect.left_center() + vec2(20.0, 0.0), Align2::CENTER_CENTER, marker, FontId::proportional(11.0), color);
    painter.text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &branch.name, FontId::proportional(13.0), color);
    painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, &branch.date, FontId::proportional(11.0), theme::FAINT_TEXT);
    let response = response.on_hover_text(format!("{} {}", branch.commit, branch.subject));
    if response.double_clicked() && !branch.current {
        actions.push(Action::Checkout(branch.clone()));
    }
    response.context_menu(|ui| {
        if !branch.current && ui.button("Переключиться").clicked() {
            actions.push(Action::Checkout(branch.clone()));
            ui.close();
        }
        if !branch.current && ui.button("Слить в текущую (merge)").clicked() {
            actions.push(Action::MergeBranch(branch.name.clone()));
            ui.close();
        }
        if !branch.current && ui.button("Rebase текущей на эту").clicked() {
            actions.push(Action::RebaseOnto(branch.name.clone()));
            ui.close();
        }
        if !branch.current && !branch.remote {
            ui.separator();
            if ui.button(RichText::new("Удалить ветку").color(theme::ERROR)).clicked() {
                actions.push(Action::DeleteBranch(branch.name.clone()));
                ui.close();
            }
        }
    });
}

fn drop_actions(app: &mut App, actions: Vec<Action>) {
    let repo = app.workspace.as_ref().and_then(|w| w.git.repo_root.clone()).unwrap_or_default();
    for action in actions {
        match action {
            Action::Stage(paths) => app.git_op("add", move |h, r| git::stage(h, r, &paths).map(|_| String::new())),
            Action::Unstage(paths) => app.git_op("reset", move |h, r| git::unstage(h, r, &paths).map(|_| String::new())),
            Action::StageAll => app.git_op("add -A", |h, r| git::stage_all(h, r).map(|_| String::new())),
            Action::UnstageAll => app.git_op("reset", |h, r| git::unstage_all(h, r).map(|_| String::new())),
            Action::Discard(files) => {
                let names: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
                let message = if names.len() == 1 {
                    format!("Отменить изменения в «{}»? Их нельзя будет вернуть.", names[0])
                } else {
                    format!("Отменить изменения в {} файлах? Их нельзя будет вернуть.", names.len())
                };
                app.dialog = Some(Dialog::Confirm {
                    title: "Отмена изменений".into(),
                    message,
                    action: Box::new(move |app: &mut App| app.git_op("restore", move |h, r| git::discard(h, r, &files).map(|_| String::new()))),
                });
            }
            Action::Diff(path, mode) => app.open_diff(path, mode),
            Action::Open(path) => app.open_file(host::join(&repo, &path), None),
            Action::TakeSide(path, ours) => app.git_op("checkout", move |h, r| git::take_side(h, r, &path, ours).map(|_| String::new())),
            Action::Commit => commit(app, false),
            Action::Push => push(app),
            Action::Pull => app.run_command(super::Command::GitPull),
            Action::Fetch => app.run_command(super::Command::GitFetch),
            Action::Sync => app.run_command(super::Command::GitSync),
            Action::Abort => {
                let state = app.workspace.as_ref().and_then(|w| w.git.status.as_ref()).map(|s| s.state.clone()).unwrap_or_default();
                app.git_op("abort", move |h, r| git::abort(h, r, &state));
            }
            Action::Continue => {
                let state = app.workspace.as_ref().and_then(|w| w.git.status.as_ref()).map(|s| s.state.clone()).unwrap_or_default();
                app.git_op("continue", move |h, r| git::continue_op(h, r, &state));
            }
            Action::Checkout(branch) if branch.name.is_empty() => app.palette = Some(Palette::branches(false)),
            Action::Checkout(branch) => app.git_op("switch", move |h, r| git::checkout(h, r, &branch)),
            Action::DeleteBranch(name) => {
                app.dialog = Some(Dialog::Confirm {
                    title: "Удаление ветки".into(),
                    message: format!("Удалить ветку «{name}»?"),
                    action: Box::new(move |app: &mut App| {
                        app.git_op("branch -d", move |h, r| git::delete_branch(h, r, &name, false).or_else(|e| if e.contains("not fully merged") { git::delete_branch(h, r, &name, true) } else { Err(e) }))
                    }),
                });
            }
            Action::MergeBranch(name) => app.git_op("merge", move |h, r| git::merge(h, r, &name)),
            Action::RebaseOnto(name) => app.git_op("rebase", move |h, r| git::rebase(h, r, &name)),
            Action::NewBranch => prompt_new_branch(app),
            Action::Stash => {
                app.dialog = Some(Dialog::Input {
                    title: "Сообщение для stash (необязательно)".into(),
                    value: "wip".into(),
                    focus: true,
                    action: Box::new(|app: &mut App, message: String| app.git_op("stash", move |h, r| git::stash_push(h, r, &message, true))),
                });
            }
            Action::StashAction(action, name) => app.git_op(&format!("stash {action}"), move |h, r| git::stash_action(h, r, action, &name)),
            Action::Log => app.open_log(true),
            Action::Init => app.run_command(super::Command::GitInit),
            Action::Refresh => app.git_refresh(),
            Action::AmendLoad => {
                let Some(workspace) = &app.workspace else { continue };
                let host = workspace.host.clone();
                let repo = repo.clone();
                if let Ok(message) = git::last_commit_message(&host, &repo)
                    && let Some(workspace) = &mut app.workspace
                {
                    workspace.git.commit_message = message;
                }
            }
        }
    }
}

pub fn commit(app: &mut App, amend_flag: bool) {
    let Some(workspace) = &mut app.workspace else { return };
    let message = workspace.git.commit_message.trim().to_string();
    let amend = workspace.git.amend || amend_flag;
    if message.is_empty() {
        app.side_view = super::SideView::Git;
        app.sidebar_open = true;
        app.notify("Введите сообщение коммита", true);
        return;
    }
    let nothing_staged = workspace.git.status.as_ref().is_some_and(|s| !s.files.iter().any(|f| f.staged()));
    workspace.git.commit_message.clear();
    workspace.git.amend = false;
    app.git_op("commit", move |h, r| {
        if nothing_staged && !amend {
            git::stage_all(h, r)?;
        }
        git::commit(h, r, &message, amend, false)
    });
}

pub fn push(app: &mut App) {
    let no_upstream = app.workspace.as_ref().and_then(|w| w.git.status.as_ref()).is_some_and(|s| s.upstream.is_none());
    app.git_op("push", move |h, r| git::push(h, r, no_upstream, false));
}

pub fn prompt_new_branch(app: &mut App) {
    app.dialog = Some(Dialog::Input {
        title: "Имя новой ветки".into(),
        value: String::new(),
        focus: true,
        action: Box::new(|app: &mut App, name: String| {
            let name = name.replace(' ', "-");
            app.git_op("switch -c", move |h, r| git::create_branch(h, r, &name, true));
        }),
    });
}

#[allow(dead_code)]
fn unused(_: egui::Pos2) -> egui::Pos2 {
    pos2(0.0, 0.0)
}
