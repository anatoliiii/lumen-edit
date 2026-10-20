//! Source control sidebar: changes, commit, branches, stashes, merge state.

use eframe::egui::{self, Align2, FontId, RichText, Sense, pos2, vec2};

use super::{App, Dialog, diffview::DiffMode, palette::Palette};
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
    pub remotes: Vec<git::Remote>,
    show_remotes: bool,
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
    AddRemote,
    EditRemote(git::Remote),
    RemoveRemote(String),
    FetchRemote(String),
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
                        if ui.button(t!("New branch…")).clicked() {
                            actions.push(Action::NewBranch);
                            ui.close();
                        }
                        if ui.button(t!("Stash changes")).clicked() {
                            actions.push(Action::Stash);
                            ui.close();
                        }
                        if ui.button(t!("History (graph)")).clicked() {
                            actions.push(Action::Log);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button(t!("Stage all")).clicked() {
                            actions.push(Action::StageAll);
                            ui.close();
                        }
                        if ui.button(t!("Unstage all")).clicked() {
                            actions.push(Action::UnstageAll);
                            ui.close();
                        }
                    });
                    if ui.small_button("⎘").on_hover_text(t!("History")).clicked() {
                        actions.push(Action::Log);
                    }
                    if ui.small_button("⟳").on_hover_text(t!("Refresh")).clicked() {
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
                ui.label(RichText::new(t!("This folder is not a git repository.")).color(theme::MUTED_TEXT));
                ui.add_space(8.0);
                if ui.button(t!("Initialize repository")).clicked() {
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
            if ui.button(format!("⎇ {branch}")).on_hover_text(t!("Switch branch")).clicked() {
                actions.push(Action::Checkout(git::Branch {
                    name: String::new(),
                    remote: false,
                    current: false,
                    upstream: String::new(),
                    commit: String::new(),
                    date: String::new(),
                    subject: String::new(),
                }));
            }
            let sync_label = match (&status.upstream, status.ahead, status.behind) {
                (None, _, _) => t!("↑ Publish").to_string(),
                (Some(_), 0, 0) => "⟳".to_string(),
                (Some(_), ahead, behind) => format!("⟳ {behind}↓ {ahead}↑"),
            };
            let hint = if status.upstream.is_none() { t!("Push the branch to origin") } else { t!("Sync: pull, then push") };
            if ui.button(sync_label).on_hover_text(hint).clicked() {
                actions.push(if status.upstream.is_none() { Action::Push } else { Action::Sync });
            }
        });
    });

    // Merge / rebase in progress.
    if status.state != RepoState::Clean {
        egui::Frame::new()
            .fill(theme::SURFACE)
            .inner_margin(egui::Margin::same(8))
            .outer_margin(egui::Margin::symmetric(8, 6))
            .corner_radius(4.0)
            .show(ui, |ui| {
                let label = match status.state {
                    RepoState::Merging => t!("Merge in progress"),
                    RepoState::Rebasing => t!("Rebase in progress"),
                    RepoState::CherryPicking => t!("Cherry-pick in progress"),
                    RepoState::Reverting => t!("Revert in progress"),
                    RepoState::Clean => "",
                };
                ui.label(RichText::new(label).color(theme::WARNING).strong());
                let conflicts = status.files.iter().filter(|f| f.conflict).count();
                if conflicts > 0 {
                    ui.label(
                        RichText::new(t!("Conflicts: {conflicts}. Resolve them in the editor and stage the files.", conflicts = conflicts))
                            .small(),
                    );
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(conflicts == 0, egui::Button::new(t!("Continue"))).clicked() {
                        actions.push(Action::Continue);
                    }
                    if ui.button(RichText::new(t!("Abort")).color(theme::ERROR)).clicked() {
                        actions.push(Action::Abort);
                    }
                });
            });
    }

    // Commit box.
    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        let hint = if git.amend { t!("New message for the last commit") } else { t!("Commit message (Ctrl+Enter)") };
        let editor =
            ui.add(egui::TextEdit::multiline(&mut git.commit_message).hint_text(hint).desired_rows(3).desired_width(f32::INFINITY));
        if editor.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)) {
            actions.push(Action::Commit);
        }
        ui.horizontal(|ui| {
            let staged = status.files.iter().filter(|f| f.staged()).count();
            let label = if git.amend {
                t!("✓ Amend commit").to_string()
            } else if staged == 0 && !status.files.is_empty() {
                t!("✓ Commit (all)").to_string()
            } else {
                t!("✓ Commit").to_string()
            };
            let enabled = !git.commit_message.trim().is_empty() && (!status.files.is_empty() || git.amend) && git.busy.is_none();
            if ui.add_enabled(enabled, egui::Button::new(RichText::new(label).strong()).fill(theme::ACCENT_DIM)).clicked() {
                actions.push(Action::Commit);
            }
            if ui.checkbox(&mut git.amend, "amend").on_hover_text(t!("Amend the last commit")).changed()
                && git.amend
                && git.commit_message.is_empty()
            {
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
            section_header(ui, t!("CONFLICTS"), conflicts.len(), &mut false, |_| {});
            for file in &conflicts {
                file_row(ui, file, false, &mut actions, true);
            }
        }
        if !staged.is_empty() {
            let mut hide = git.hide_staged;
            section_header(ui, t!("STAGED"), staged.len(), &mut hide, |ui| {
                if ui.small_button("−").on_hover_text(t!("Unstage all")).clicked() {
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
        section_header(ui, t!("CHANGES"), changes.len(), &mut hide, |ui| {
            if !changes.is_empty() {
                if ui.small_button("+").on_hover_text(t!("Stage all")).clicked() {
                    actions.push(Action::StageAll);
                }
                if ui.small_button("↺").on_hover_text(t!("Discard all changes")).clicked() {
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
                    ui.label(RichText::new(t!("Working tree is clean")).small().color(theme::FAINT_TEXT));
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
        section_header(ui, t!("BRANCHES"), local.len(), &mut collapsed, |ui| {
            if ui.small_button("+").on_hover_text(t!("New branch")).clicked() {
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

        // Remotes.
        let mut collapsed = !git.show_remotes;
        section_header(ui, "REMOTES", git.remotes.len(), &mut collapsed, |ui| {
            if ui.small_button("+").on_hover_text(t!("Add remote")).clicked() {
                actions.push(Action::AddRemote);
            }
        });
        git.show_remotes = !collapsed;
        if git.show_remotes {
            if git.remotes.is_empty() {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    if ui.small_button(t!("Add origin…")).clicked() {
                        actions.push(Action::AddRemote);
                    }
                });
            }
            for remote in &git.remotes {
                let response = ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(&remote.name).strong());
                    ui.add(egui::Label::new(RichText::new(&remote.url).small().color(theme::MUTED_TEXT)).truncate());
                });
                response.response.interact(Sense::click()).context_menu(|ui| {
                    if ui.button(t!("Change URL…")).clicked() {
                        actions.push(Action::EditRemote(remote.clone()));
                        ui.close();
                    }
                    if ui.button(t!("Copy URL")).clicked() {
                        ui.ctx().copy_text(remote.url.clone());
                        ui.close();
                    }
                    if ui.button("Fetch").clicked() {
                        actions.push(Action::FetchRemote(remote.name.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(RichText::new(t!("Delete")).color(theme::ERROR)).clicked() {
                        actions.push(Action::RemoveRemote(remote.name.clone()));
                        ui.close();
                    }
                });
            }
        }

        // Stashes.
        let mut collapsed = !git.show_stashes;
        section_header(ui, "STASH", git.stashes.len(), &mut collapsed, |ui| {
            if ui.small_button("+").on_hover_text(t!("Stash changes")).clicked() {
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
                        if ui.small_button("✕").on_hover_text(t!("Delete")).clicked() {
                            actions.push(Action::StashAction("drop", stash.name.clone()));
                        }
                        if ui.small_button("↓").on_hover_text(t!("Apply and drop (pop)")).clicked() {
                            actions.push(Action::StashAction("pop", stash.name.clone()));
                        }
                        if ui.small_button("✓").on_hover_text(t!("Apply")).clicked() {
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
    ui.painter().text(
        rect.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        if *collapsed { "⏵" } else { "⏷" },
        FontId::proportional(10.0),
        theme::MUTED_TEXT,
    );
    ui.painter().text(rect.left_center() + vec2(24.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(11.5), theme::MUTED_TEXT);
    let badge = count.to_string();
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, badge, FontId::proportional(11.5), theme::FAINT_TEXT);
    let mut child = ui
        .new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(30.0, 2.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
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
    let letter = file.letter(staged);
    let color = match letter {
        'A' | 'U' => theme::GIT_ADDED,
        'D' => theme::GIT_DELETED,
        '!' => theme::GIT_CONFLICT,
        _ => theme::GIT_MODIFIED,
    };
    let painter = ui.painter_at(rect);
    super::icons::file(&painter, rect.left_center() + vec2(24.0, 0.0), &format!("/{}", file.path));
    let name_color = if letter == 'D' { theme::MUTED_TEXT } else { theme::TEXT };
    let galley = painter.layout_no_wrap(name.to_string(), FontId::proportional(13.0), name_color);
    let w = galley.size().x;
    painter.galley(rect.left_center() + vec2(34.0, -galley.size().y / 2.0), galley, name_color);
    painter.text(rect.left_center() + vec2(40.0 + w, 0.0), Align2::LEFT_CENTER, dir, FontId::proportional(11.5), theme::FAINT_TEXT);
    painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, letter.to_string(), FontId::monospace(12.0), color);

    if hovered {
        let mut child = ui.new_child(
            egui::UiBuilder::new().max_rect(rect.shrink2(vec2(26.0, 1.0))).layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        let paths = vec![file.path.clone()];
        if conflict {
            if child.small_button("+").on_hover_text(t!("Mark as resolved")).clicked() {
                actions.push(Action::Stage(paths.clone()));
            }
            if child.small_button(t!("theirs")).on_hover_text(t!("Take the incoming version entirely")).clicked() {
                actions.push(Action::TakeSide(file.path.clone(), false));
            }
            if child.small_button(t!("mine")).on_hover_text(t!("Take the current version entirely")).clicked() {
                actions.push(Action::TakeSide(file.path.clone(), true));
            }
        } else if staged {
            if child.small_button("−").on_hover_text(t!("Unstage")).clicked() {
                actions.push(Action::Unstage(paths.clone()));
            }
        } else {
            if child.small_button("+").on_hover_text(t!("Stage")).clicked() {
                actions.push(Action::Stage(paths.clone()));
            }
            if child.small_button("↺").on_hover_text(t!("Discard changes")).clicked() {
                actions.push(Action::Discard(vec![file.clone()]));
            }
        }
        if child.small_button("🗋").on_hover_text(t!("Open File")).clicked() {
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
    let marker = if branch.current {
        "●"
    } else if branch.remote {
        "⇅"
    } else {
        "⎇"
    };
    painter.text(rect.left_center() + vec2(20.0, 0.0), Align2::CENTER_CENTER, marker, FontId::proportional(11.0), color);
    painter.text(rect.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, &branch.name, FontId::proportional(13.0), color);
    painter.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, &branch.date, FontId::proportional(11.0), theme::FAINT_TEXT);
    let response = response.on_hover_text(format!("{} {}", branch.commit, branch.subject));
    if response.double_clicked() && !branch.current {
        actions.push(Action::Checkout(branch.clone()));
    }
    response.context_menu(|ui| {
        if !branch.current && ui.button(t!("Switch")).clicked() {
            actions.push(Action::Checkout(branch.clone()));
            ui.close();
        }
        if !branch.current && ui.button(t!("Merge into current")).clicked() {
            actions.push(Action::MergeBranch(branch.name.clone()));
            ui.close();
        }
        if !branch.current && ui.button(t!("Rebase current onto this")).clicked() {
            actions.push(Action::RebaseOnto(branch.name.clone()));
            ui.close();
        }
        if !branch.current && !branch.remote {
            ui.separator();
            if ui.button(RichText::new(t!("Delete branch")).color(theme::ERROR)).clicked() {
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
                    t!("Discard changes in “{n1}”? They cannot be recovered.", n1 = names[0])
                } else {
                    t!("Discard changes in {n1} files? They cannot be recovered.", n1 = names.len())
                };
                app.dialog = Some(Dialog::Confirm {
                    title: t!("Discard changes").into(),
                    message,
                    action: Box::new(move |app: &mut App| {
                        app.git_op("restore", move |h, r| git::discard(h, r, &files).map(|_| String::new()))
                    }),
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
                    title: t!("Delete branch").into(),
                    message: t!("Delete branch “{name}”?", name = name),
                    action: Box::new(move |app: &mut App| {
                        app.git_op("branch -d", move |h, r| {
                            git::delete_branch(h, r, &name, false)
                                .or_else(|e| if e.contains("not fully merged") { git::delete_branch(h, r, &name, true) } else { Err(e) })
                        })
                    }),
                });
            }
            Action::MergeBranch(name) => app.git_op("merge", move |h, r| git::merge(h, r, &name)),
            Action::RebaseOnto(name) => app.git_op("rebase", move |h, r| git::rebase(h, r, &name)),
            Action::NewBranch => prompt_new_branch(app),
            Action::Stash => {
                app.dialog = Some(Dialog::Input {
                    title: t!("Stash message (optional)").into(),
                    value: "wip".into(),
                    focus: true,
                    action: Box::new(|app: &mut App, message: String| {
                        app.git_op("stash", move |h, r| git::stash_push(h, r, &message, true))
                    }),
                });
            }
            Action::StashAction(action, name) => app.git_op(&format!("stash {action}"), move |h, r| git::stash_action(h, r, action, &name)),
            Action::Log => app.open_log(true),
            Action::Init => app.run_command(super::Command::GitInit),
            Action::AddRemote => prompt_add_remote(app, None),
            Action::EditRemote(remote) => {
                app.dialog = Some(Dialog::Input {
                    title: t!("URL for “{n1}”", n1 = remote.name),
                    value: remote.url.clone(),
                    focus: true,
                    action: Box::new(move |app: &mut App, url: String| {
                        let name = remote.name.clone();
                        app.git_op("remote set-url", move |h, r| git::set_remote_url(h, r, &name, &url));
                    }),
                });
            }
            Action::RemoveRemote(name) => {
                app.dialog = Some(Dialog::Confirm {
                    title: t!("Remove remote").into(),
                    message: t!("Remove remote “{name}”? The remote repository itself is not affected.", name = name),
                    action: Box::new(move |app: &mut App| app.git_op("remote remove", move |h, r| git::remove_remote(h, r, &name))),
                });
            }
            Action::FetchRemote(name) => app.git_op("fetch", move |h, r| git::git_verbose(h, r, &["fetch", "--prune", &name])),
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
        app.notify(t!("Enter a commit message"), true);
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
    let Some(git) = app.workspace.as_ref().map(|w| &w.git) else { return };
    let no_upstream = git.status.as_ref().is_some_and(|s| s.upstream.is_none());
    if git.remotes.is_empty() {
        // Nowhere to push yet: ask for the URL, then publish.
        prompt_add_remote(app, Some(()));
        return;
    }
    let remote = git.remotes.iter().find(|r| r.name == "origin").or(git.remotes.first()).map(|r| r.name.clone());
    let upstream = if no_upstream { remote } else { None };
    app.git_op("push", move |h, r| git::push(h, r, upstream.as_deref(), false));
}

/// Asks for `[name] url`; with `publish`, pushes the current branch to it.
pub fn prompt_add_remote(app: &mut App, publish: Option<()>) {
    let first = app.workspace.as_ref().is_none_or(|w| w.git.remotes.is_empty());
    app.dialog = Some(Dialog::Input {
        title: if publish.is_some() {
            t!("Where to publish: repository URL (git@host:user/repo.git)").into()
        } else {
            t!("New remote: name and URL separated by a space (or just a URL for origin)").into()
        },
        value: if first || publish.is_some() { String::new() } else { "upstream ".into() },
        focus: true,
        action: Box::new(move |app: &mut App, input: String| {
            let mut parts = input.split_whitespace();
            let (name, url) = match (parts.next(), parts.next()) {
                (Some(name), Some(url)) => (name.to_string(), url.to_string()),
                (Some(url), None) => ("origin".to_string(), url.to_string()),
                _ => return,
            };
            app.git_op(if publish.is_some() { "publish" } else { "remote add" }, move |h, r| {
                let added = git::add_remote(h, r, &name, &url)?;
                if publish.is_some() {
                    let pushed = git::push(h, r, Some(&name), false)?;
                    Ok(format!("{added}\n{pushed}"))
                } else {
                    Ok(if added.is_empty() { format!("{name} → {url}") } else { added })
                }
            });
        }),
    });
}

pub fn prompt_new_branch(app: &mut App) {
    app.dialog = Some(Dialog::Input {
        title: t!("New branch name").into(),
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
