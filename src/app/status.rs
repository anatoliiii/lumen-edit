//! Menu bar, activity bar, status bar and the settings window.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, vec2};

use super::{App, Command, SideView, Tab, panel, util};
use crate::{host::Host, theme};

fn menu_item(ui: &mut egui::Ui, app: &mut App, command: Command) {
    let mut button = egui::Button::new(command.label());
    if let Some(shortcut) = command.shortcut_text() {
        button = button.shortcut_text(RichText::new(shortcut).color(theme::FAINT_TEXT));
    }
    if ui.add(button).clicked() {
        app.run_command(command);
        ui.close();
    }
}

pub fn menu_bar(ui: &mut egui::Ui, app: &mut App) {
    egui::Panel::top("menu_bar")
        .exact_size(28.0)
        .frame(
            egui::Frame::new()
                .fill(theme::ACTIVITY_BACKGROUND)
                .inner_margin(egui::Margin::symmetric(6, 0))
                .stroke(Stroke::new(1.0, theme::BORDER)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.label(RichText::new("◆").color(theme::ACCENT));
                ui.menu_button(t!("File"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::OpenFolder, Command::ConnectSsh, Command::OpenRecent] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in
                        [Command::NewFile, Command::NewFolder, Command::Teleport, Command::Save, Command::SaveAll, Command::CloseTab]
                    {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    let mut auto_save = app.config.auto_save;
                    if ui.checkbox(&mut auto_save, t!("Auto save")).changed() {
                        app.config.auto_save = auto_save;
                        app.config.save();
                    }
                    menu_item(ui, app, Command::Settings);
                    menu_item(ui, app, Command::Keybindings);
                    ui.separator();
                    menu_item(ui, app, Command::CloseWorkspace);
                    menu_item(ui, app, Command::Quit);
                });
                ui.menu_button(t!("Edit"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::Find, Command::Replace, Command::ShowSearch] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::Format, Command::Rename, Command::CodeActions] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button(t!("View"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [
                        Command::CommandPalette,
                        Command::ShowExplorer,
                        Command::ShowSearch,
                        Command::ShowGit,
                        Command::ShowRun,
                        Command::ToggleSidebar,
                        Command::TogglePanel,
                        Command::ShowProblems,
                        Command::ShowOutput,
                    ] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    let mut whitespace = app.config.show_whitespace;
                    if ui.checkbox(&mut whitespace, t!("Show whitespace")).changed() {
                        app.config.show_whitespace = whitespace;
                        app.config.save();
                    }
                    for command in [Command::ZoomIn, Command::ZoomOut, Command::ZoomReset] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button(t!("Go"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [
                        Command::QuickOpen,
                        Command::GotoLine,
                        Command::GotoDefinition,
                        Command::References,
                        Command::NextTab,
                        Command::PrevTab,
                    ] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button(t!("Run"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [
                        Command::RunDefault,
                        Command::RunTask,
                        Command::Build,
                        Command::Test,
                        Command::RerunLast,
                        Command::KillTerminal,
                        Command::EditTasks,
                    ] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button(t!("Terminal"), |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::NewTerminal, Command::ToggleTerminal, Command::KillTerminal] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Git", |ui| {
                    ui.set_min_width(320.0);
                    for command in [Command::ShowGit, Command::GitCommit, Command::GitCommitAmend, Command::GitStageFile] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::GitPull, Command::GitPush, Command::GitFetch, Command::GitSync] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::GitCheckout, Command::GitNewBranch, Command::GitMerge, Command::GitStash, Command::GitStashPop]
                    {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::GitLog, Command::GitFileHistory, Command::GitToggleBlame, Command::GitInit] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button(t!("Help"), |ui| {
                    ui.set_min_width(300.0);
                    menu_item(ui, app, Command::CommandPalette);
                    menu_item(ui, app, Command::RestartLsp);
                    ui.separator();
                    menu_item(ui, app, Command::About);
                });
            });
        });
}

#[derive(Clone, Copy)]
enum Icon {
    Files,
    Search,
    Git,
    Run,
    Terminal,
    Settings,
}

fn draw_icon(painter: &egui::Painter, icon: Icon, center: Pos2, color: Color32) {
    let stroke = Stroke::new(1.6, color);
    match icon {
        Icon::Files => {
            let back = Rect::from_center_size(center + vec2(3.0, -3.0), vec2(13.0, 16.0));
            let front = Rect::from_center_size(center + vec2(-2.0, 2.0), vec2(13.0, 16.0));
            painter.rect_stroke(back, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            painter.rect_filled(front, CornerRadius::same(2), theme::ACTIVITY_BACKGROUND);
            painter.rect_stroke(front, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
        }
        Icon::Search => {
            painter.circle_stroke(center + vec2(-2.0, -2.0), 6.5, stroke);
            painter.line_segment([center + vec2(3.0, 3.0), center + vec2(8.5, 8.5)], Stroke::new(2.0, color));
        }
        Icon::Git => {
            let a = center + vec2(-5.0, -8.0);
            let b = center + vec2(-5.0, 8.0);
            let c = center + vec2(6.0, -3.0);
            painter.line_segment([a, b], stroke);
            painter.add(Shape::QuadraticBezier(egui::epaint::QuadraticBezierShape::from_points_stroke(
                [c, c + vec2(0.0, 8.0), center + vec2(-5.0, 4.0)],
                false,
                Color32::TRANSPARENT,
                stroke,
            )));
            for p in [a, b, c] {
                painter.circle_filled(p, 2.8, color);
            }
        }
        Icon::Run => {
            let points = vec![center + vec2(-5.0, -8.0), center + vec2(8.0, 0.0), center + vec2(-5.0, 8.0)];
            painter.add(Shape::closed_line(points, stroke));
        }
        Icon::Terminal => {
            let rect = Rect::from_center_size(center, vec2(18.0, 14.0));
            painter.rect_stroke(rect, CornerRadius::same(2), stroke, egui::StrokeKind::Middle);
            painter.line_segment([center + vec2(-5.0, -3.0), center + vec2(-2.0, 0.0)], stroke);
            painter.line_segment([center + vec2(-2.0, 0.0), center + vec2(-5.0, 3.0)], stroke);
            painter.line_segment([center + vec2(0.0, 3.0), center + vec2(5.0, 3.0)], stroke);
        }
        Icon::Settings => {
            painter.circle_stroke(center, 4.0, stroke);
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::TAU / 8.0;
                let dir = vec2(angle.cos(), angle.sin());
                painter.line_segment([center + dir * 6.0, center + dir * 8.5], Stroke::new(2.2, color));
            }
        }
    }
}

pub fn activity_bar(ui: &mut egui::Ui, app: &mut App) {
    let changes = app.workspace.as_ref().and_then(|w| w.git.status.as_ref()).map(|s| s.files.len()).unwrap_or(0);
    egui::Panel::left("activity_bar")
        .exact_size(46.0)
        .resizable(false)
        .frame(egui::Frame::new().fill(theme::ACTIVITY_BACKGROUND).stroke(Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            let items = [
                (Icon::Files, Some(SideView::Explorer), t!("Explorer (Ctrl+Shift+E)"), 0),
                (Icon::Search, Some(SideView::Search), t!("Search (Ctrl+Shift+F)"), 0),
                (Icon::Git, Some(SideView::Git), "Git (Ctrl+Shift+G)", changes),
                (Icon::Run, Some(SideView::Run), t!("Run and tasks (Ctrl+Shift+R)"), 0),
                (Icon::Terminal, None, t!("Terminal (Ctrl+`)"), 0),
            ];
            ui.add_space(4.0);
            for (icon, view, tooltip, badge) in items {
                let (rect, response) = ui.allocate_exact_size(vec2(46.0, 44.0), Sense::click());
                let active = match view {
                    Some(view) => app.sidebar_open && app.side_view == view,
                    None => app.panel_open && app.panel_tab == super::PanelTab::Terminal,
                };
                let color = if active || response.hovered() { theme::TEXT } else { theme::FAINT_TEXT };
                if active && view.is_some() {
                    ui.painter().line_segment(
                        [rect.left_top() + vec2(1.0, 6.0), rect.left_bottom() + vec2(1.0, -6.0)],
                        Stroke::new(2.0, theme::ACCENT),
                    );
                }
                draw_icon(ui.painter(), icon, rect.center(), color);
                if badge > 0 {
                    let center = rect.center() + vec2(10.0, 9.0);
                    ui.painter().circle_filled(center, 7.5, theme::ACCENT);
                    let text = if badge > 99 { "99+".to_string() } else { badge.to_string() };
                    ui.painter().text(center, Align2::CENTER_CENTER, text, FontId::proportional(9.5), Color32::WHITE);
                }
                let response = response.on_hover_text(tooltip);
                if response.clicked() {
                    match view {
                        Some(view) => {
                            if app.sidebar_open && app.side_view == view {
                                app.sidebar_open = false;
                            } else {
                                app.side_view = view;
                                app.sidebar_open = true;
                                if view == SideView::Git {
                                    app.git_refresh();
                                }
                            }
                        }
                        None => app.run_command(Command::ToggleTerminal),
                    }
                }
            }
            // The pi agent.
            let (rect, response) = ui.allocate_exact_size(vec2(46.0, 44.0), Sense::click());
            let agent_active = app.agent_manager.open
                || (app.agent_dock.is_some() && app.agent_dock_open)
                || matches!(app.active_tab.and_then(|i| app.tabs.get(i)), Some(Tab::Agent(_)));
            let busy =
                app.tabs.iter().any(|t| matches!(t, Tab::Agent(a) if a.streaming)) || app.agent_dock.as_ref().is_some_and(|a| a.streaming);
            let color = if agent_active || response.hovered() { theme::TEXT } else { theme::FAINT_TEXT };
            if agent_active {
                ui.painter().line_segment(
                    [rect.left_top() + vec2(1.0, 6.0), rect.left_bottom() + vec2(1.0, -6.0)],
                    Stroke::new(2.0, theme::ACCENT),
                );
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "π", FontId::proportional(21.0), color);
            let waiting = super::agent::manager::waiting_count(app);
            if busy || waiting > 0 {
                ui.painter().circle_filled(rect.center() + vec2(10.0, 9.0), 4.0, if waiting > 0 { theme::WARNING } else { theme::SUCCESS });
            }
            response.context_menu(|ui| {
                if ui.button(t!("Agent sessions")).clicked() {
                    app.run_command(Command::AgentSessions);
                    ui.close();
                }
            });
            if response.on_hover_text(t!("pi agent (Ctrl+Shift+A)")).clicked() {
                app.run_command(Command::AgentSessions);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let (rect, response) = ui.allocate_exact_size(vec2(46.0, 44.0), Sense::click());
                let settings_active = app.active_tab.is_some_and(|i| matches!(app.tabs.get(i), Some(super::Tab::Settings)));
                let color = if response.hovered() || settings_active { theme::TEXT } else { theme::FAINT_TEXT };
                draw_icon(ui.painter(), Icon::Settings, rect.center(), color);
                if response.on_hover_text(t!("Settings (Ctrl+,)")).clicked() {
                    app.toggle_settings();
                }
            });
        });
}

pub fn status_bar(ui: &mut egui::Ui, app: &mut App) {
    egui::Panel::bottom("status_bar")
        .exact_size(24.0)
        .frame(egui::Frame::new().fill(theme::STATUS_BACKGROUND).stroke(Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Host badge.
                let remote = match app.workspace.as_ref().map(|w| &w.host) {
                    Some(Host::Remote(remote)) => Some(remote.clone()),
                    _ => None,
                };
                let (label, fill) = match app.workspace.as_ref().map(|w| &w.host) {
                    Some(Host::Remote(remote)) if !remote.connected.load(std::sync::atomic::Ordering::Relaxed) => {
                        (t!("  ⇄ SSH: {n1} — disconnected  ", n1 = remote.target), theme::ERROR.gamma_multiply(0.75))
                    }
                    Some(Host::Remote(remote)) => (format!("  ⇄ SSH: {}  ", remote.target), theme::REMOTE_BADGE),
                    _ => (t!("  ⌂ Local  ").to_string(), theme::ACCENT_DIM),
                };
                let badge = ui.add(
                    egui::Button::new(RichText::new(label).size(12.0).color(Color32::WHITE))
                        .fill(fill)
                        .corner_radius(0.0)
                        .min_size(vec2(0.0, 24.0)),
                );
                let popup_id = ui.make_persistent_id("host_menu");
                egui::Popup::menu(&badge).id(popup_id).show(|ui| {
                    ui.set_min_width(240.0);
                    if let Some(remote) = &remote {
                        let up = remote.connected.load(std::sync::atomic::Ordering::Relaxed);
                        ui.label(RichText::new(if up { t!("Connected") } else { t!("No connection to the host") }).small().color(if up {
                            theme::MUTED_TEXT
                        } else {
                            theme::ERROR
                        }));
                        ui.label(
                            RichText::new(t!("Agent: {n1}", n1 = remote.agent_status.lock().unwrap_or_else(|e| e.into_inner())))
                                .small()
                                .color(theme::MUTED_TEXT),
                        );
                        if ui.button(t!("Reconnect")).clicked() {
                            let root = app.workspace.as_ref().map(|w| w.root.clone());
                            app.start_connect(remote.target.clone(), root);
                            ui.close();
                        }
                        if ui.button(t!("Restart the agent on the server")).clicked() {
                            let remote = remote.clone();
                            app.bus.spawn(move || match remote.restart_agent() {
                                Ok(()) => super::Msg::Notice(t!("The agent on the server was restarted").into(), false),
                                Err(error) => super::Msg::Notice(t!("The agent did not start: {error}", error = error), true),
                            });
                            ui.close();
                        }
                        ui.separator();
                    }
                    if ui.button(t!("Connect via SSH…")).clicked() {
                        app.run_command(Command::ConnectSsh);
                        ui.close();
                    }
                    if ui.button(t!("Open Folder…")).clicked() {
                        app.run_command(Command::OpenFolder);
                        ui.close();
                    }
                    if ui.button(t!("Recent projects…")).clicked() {
                        app.run_command(Command::OpenRecent);
                        ui.close();
                    }
                    if app.workspace.is_some() && ui.button(t!("Close Project")).clicked() {
                        app.run_command(Command::CloseWorkspace);
                        ui.close();
                    }
                });

                let mut pending: Option<Command> = None;
                if let Some(workspace) = &app.workspace
                    && let Some(status) = &workspace.git.status
                {
                    ui.add_space(8.0);
                    let dirty = if status.files.is_empty() { "" } else { "*" };
                    let branch = if status.detached { "detached".to_string() } else { status.branch.clone() };
                    if status_button(ui, &format!("⎇ {branch}{dirty}")).on_hover_text(t!("Switch branch")).clicked() {
                        pending = Some(Command::GitCheckout);
                    }
                    let sync = if workspace.git.busy.is_some() {
                        "⟳ …".to_string()
                    } else if status.upstream.is_none() {
                        "↑".to_string()
                    } else {
                        format!("{}↓ {}↑", status.behind, status.ahead)
                    };
                    if status_button(ui, &sync).on_hover_text(t!("Sync")).clicked() {
                        pending = Some(if status.upstream.is_none() { Command::GitPush } else { Command::GitSync });
                    }
                }
                if let Some(command) = pending {
                    app.run_command(command);
                }

                let ai_enabled = !app.config.inline_completion.paused
                    && app
                        .workspace
                        .as_ref()
                        .is_some_and(|w| !w.host.is_remote() && app.config.inline_completion.projects.contains(&w.session_key()));
                if ai_enabled {
                    let busy = app.inline_completion.busy();
                    let status = &app.inline_completion.status;
                    let label = if busy || app.inline_completion.waiting() {
                        "AI …"
                    } else if status.is_empty() {
                        "AI"
                    } else {
                        "AI !"
                    };
                    if status_button(ui, label).on_hover_text(if status.is_empty() { t!("AI Inline Completion") } else { status }).clicked()
                    {
                        app.open_settings();
                    }
                }
                let (errors, warnings) = panel::status_counts(app);
                ui.add_space(4.0);
                if status_button(ui, &format!("⊗ {errors}  ▲ {warnings}")).on_hover_text(t!("Problems (Ctrl+Shift+M)")).clicked() {
                    app.run_command(Command::ShowProblems);
                }
                if let Some((text, color)) = super::pulse::status_text(app) {
                    ui.add_space(4.0);
                    let button =
                        ui.add(egui::Button::new(RichText::new(text).size(12.0).color(color)).frame(false).min_size(vec2(0.0, 22.0)));
                    if button.on_hover_text(t!("Server pulse: open details")).clicked() {
                        app.panel_open = true;
                        app.panel_tab = super::PanelTab::Pulse;
                    }
                }
                if let Some((server, progress)) = app.lsp.progress.iter().next() {
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("{server}: {}", util::truncate_middle(progress, 50))).size(11.5).color(theme::MUTED_TEXT),
                    );
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    let servers = app.lsp.status();
                    if !servers.is_empty() {
                        let text: Vec<String> = servers
                            .iter()
                            .map(|(name, state, _)| match *state {
                                "ready" => format!("✓ {name}"),
                                "starting" => format!("… {name}"),
                                _ => format!("✗ {name}"),
                            })
                            .collect();
                        let tooltip: Vec<String> = servers
                            .iter()
                            .map(|(n, s, e)| format!("{n}: {s}{}", e.as_ref().map(|e| format!(" — {e}")).unwrap_or_default()))
                            .collect();
                        if status_button(ui, &text.join("  ")).on_hover_text(tooltip.join("\n")).clicked() {
                            app.run_command(Command::ShowOutput);
                        }
                    }
                    let Some(index) = app.active_tab else { return };
                    let Some(Tab::Doc(doc)) = app.tabs.get(index) else { return };
                    status_label(ui, crate::ci::Ci::detect(&doc.path).map(|ci| ci.label()).unwrap_or(doc.lang.display_name()));
                    status_label(ui, if doc.buffer.crlf { "CRLF" } else { "LF" });
                    status_label(ui, "UTF-8");
                    let indent = if doc.buffer.insert_spaces {
                        t!("Spaces: {n1}", n1 = doc.buffer.tab_size)
                    } else {
                        t!("Tab size: {n1}", n1 = doc.buffer.tab_size)
                    };
                    let indent_button = status_button(ui, &indent);
                    let mut toggle_indent = false;
                    if indent_button.clicked() {
                        toggle_indent = true;
                    }
                    let primary = doc.buffer.primary();
                    let (line, col) = doc.buffer.line_col(primary.head);
                    let mut position = t!("Ln {n1}, Col {n2}", n1 = line + 1, n2 = col + 1);
                    let selected: usize = doc.buffer.selections.iter().map(|s| s.end() - s.start()).sum();
                    if selected > 0 {
                        position.push_str(&t!(" ({selected} selected)", selected = selected));
                    }
                    if doc.buffer.selections.len() > 1 {
                        position.push_str(&t!(" · {n1} cursors", n1 = doc.buffer.selections.len()));
                    }
                    if status_button(ui, &position).clicked() {
                        app.run_command(Command::GotoLine);
                    }
                    if toggle_indent && let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) {
                        // Cycle: spaces 4 -> spaces 2 -> tabs -> spaces 4.
                        match (doc.buffer.insert_spaces, doc.buffer.tab_size) {
                            (true, 4) => doc.buffer.tab_size = 2,
                            (true, _) => {
                                doc.buffer.insert_spaces = false;
                                doc.buffer.tab_size = 4;
                            }
                            (false, _) => {
                                doc.buffer.insert_spaces = true;
                                doc.buffer.tab_size = 4;
                            }
                        }
                    }
                });
            });
        });
}

fn status_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_space(4.0);
    let response =
        ui.add(egui::Button::new(RichText::new(text).size(12.0).color(theme::MUTED_TEXT)).frame(false).min_size(vec2(0.0, 22.0)));
    ui.add_space(4.0);
    response
}

fn status_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(12.0).color(theme::MUTED_TEXT));
    ui.add_space(8.0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsTab {
    Editor,
    Ai,
    Pi,
    Servers,
}

fn tab_index(tab: &SettingsTab) -> usize {
    match tab {
        SettingsTab::Editor => 0,
        SettingsTab::Ai => 1,
        SettingsTab::Pi => 2,
        SettingsTab::Servers => 3,
    }
}

fn settings_section(ui: &mut egui::Ui, title: &str) {
    super::settings_layout::section(ui, title);
}

/// A two-column table row: the label cell explains the setting, the control
/// cell holds the checkbox. Hover hints keep the table readable.
fn settings_checkbox(ui: &mut egui::Ui, value: &mut bool, hint: Option<&str>) -> bool {
    let response = ui.checkbox(value, "");
    if let Some(hint) = hint {
        let _ = response.clone().on_hover_text(hint);
    }
    response.changed()
}

fn editor_settings_ui(ui: &mut egui::Ui, app: &mut App, changed: &mut bool) {
    egui::ScrollArea::vertical().id_salt("settings_editor").auto_shrink([false, false]).show(ui, |ui| {
        let config = &mut app.config;

        settings_section(ui, t!("Appearance"));
        super::settings_layout::row(ui, t!("Language"), |ui| {
            let name = |code: &str| crate::i18n::SETTINGS.iter().find(|(c, _)| *c == code).map_or(code.to_string(), |(_, n)| n.to_string());
            let auto = |label: String| if label == "Auto" { t!("Auto (system)").to_string() } else { label };
            egui::ComboBox::from_id_salt("language").selected_text(auto(name(&config.language))).show_ui(ui, |ui| {
                for (code, label) in crate::i18n::SETTINGS {
                    if ui.selectable_label(config.language == *code, auto(label.to_string())).clicked() && config.language != *code {
                        config.language = code.to_string();
                        crate::i18n::set(crate::i18n::from_setting(code));
                        *changed = true;
                    }
                }
            });
        });
        super::settings_layout::row(ui, t!("Interface font"), |ui| {
            let ui_font = ui.add(egui::Slider::new(&mut config.ui_font_size, 10.0..=20.0));
            if ui_font.drag_stopped() || ui_font.lost_focus() {
                theme::apply(ui.ctx(), config.ui_font_size);
                *changed = true;
            }
        });
        super::settings_layout::row(ui, t!("Editor font"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.editor_font_size, 9.0..=28.0)).changed();
        });
        super::settings_layout::row(ui, t!("Terminal font"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.terminal_font_size, 9.0..=28.0)).changed();
        });

        settings_section(ui, t!("Editing"));
        super::settings_layout::row(ui, t!("Tab width"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.tab_size, 1..=8)).changed();
        });
        super::settings_layout::row(ui, t!("Indent with spaces"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.insert_spaces, None);
        });
        super::settings_layout::row(ui, t!("Format on save"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.format_on_save, None);
        });
        super::settings_layout::row(ui, t!("Auto save"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.auto_save, None);
        });
        super::settings_layout::row(ui, t!("Show whitespace"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.show_whitespace, None);
        });
        super::settings_layout::row(ui, t!("Line blame"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.inline_blame, None);
        });
        super::settings_layout::row(ui, "git pull --rebase", |ui| {
            *changed |= settings_checkbox(ui, &mut config.pull_rebase, None);
        });
    });
}

fn completion_status_ui(
    ui: &mut egui::Ui,
    fim: &crate::inline_completion::Controller,
    next: &crate::inline_completion::next_edit::Controller,
) {
    settings_section(ui, t!("Status"));
    let m = &fim.metrics;
    let ledger = &fim.ledger;
    for (label, value) in [
        (t!("Session requests"), ledger.requests.to_string()),
        (t!("Session spending"), format!("${:.4}", ledger.cost_usd)),
        (t!("Reserved for pending requests"), format!("${:.4}", ledger.reserved_usd)),
        (t!("Successful completions"), m.success.to_string()),
        (t!("Accepted completions"), m.accepted.to_string()),
        (t!("Completion errors"), (m.errors + next.metrics.errors).to_string()),
    ] {
        super::settings_layout::row(ui, label, |ui| {
            ui.label(RichText::new(value).strong());
        });
    }
    if !m.rejection_reason.is_empty() {
        ui.colored_label(theme::WARNING, t!("Last completion: {reason}", reason = m.rejection_reason));
    }
    for (label, status) in [("FIM", &fim.status), ("Next Edit", &next.status)] {
        if !status.is_empty() {
            super::settings_layout::row(ui, label, |ui| {
                ui.label(status);
            });
        }
    }
    egui::CollapsingHeader::new(t!("Completion diagnostics")).show(ui, |ui| {
        let (p50, p95) = m.latency_percentiles();
        let (next_p50, next_p95) = next.metrics.percentiles();
        for (label, value) in [
            (t!("FIM requests / Next Edit requests"), format!("{} / {}", m.requests, next.metrics.requests)),
            (t!("Cache hits / dismissed"), format!("{} / {}", m.cache_hits, m.rejected)),
            (t!("FIM latency: last / p50 / p95 (ms)"), format!("{} / {p50} / {p95}", m.total_latency_ms)),
            (t!("Next Edit latency: last / p50 / p95 (ms)"), format!("{} / {next_p50} / {next_p95}", next.metrics.latest.total_latency_ms)),
            (t!("Debounce / queue / context (ms)"), format!("{} / {} / {}", m.debounce_ms, m.queue_ms, m.context_prepare_ms)),
            (t!("HTTP / display (ms)"), format!("{} / {}", m.http_request_ms, m.display_ms)),
            (t!("FIM tokens: input / output / cached"), format!("{} / {} / {}", m.input_tokens, m.output_tokens, m.cached_tokens)),
            (
                t!("Next Edit tokens: input / output / cached"),
                format!(
                    "{} / {} / {}",
                    next.metrics.latest.prompt_tokens, next.metrics.latest.completion_tokens, next.metrics.latest.cached_tokens
                ),
            ),
            (
                t!("Next Edit context / HTTP / diff (ms)"),
                format!(
                    "{} / {} / {}",
                    next.metrics.latest.context_prepare_ms, next.metrics.latest.http_latency_ms, next.metrics.latest.diff_prepare_ms
                ),
            ),
        ] {
            super::settings_layout::row(ui, label, |ui| {
                ui.label(value);
            });
        }
    });
}

fn ai_settings_ui(ui: &mut egui::Ui, app: &mut App, completion_project: Option<&str>, changed: &mut bool) {
    egui::ScrollArea::vertical().id_salt("settings_ai").auto_shrink([false, false]).show(ui, |ui| {
        completion_status_ui(ui, &app.inline_completion, &app.next_edit);
        let config = &mut app.config;

        settings_section(ui, t!("Provider"));
        super::settings_layout::row(ui, t!("Pause AI completion in all projects"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.inline_completion.paused, None);
        });
        super::settings_layout::row(ui, t!("Allow cloud completion for this project"), |ui| {
            if let Some(key) = completion_project {
                let mut enabled = config.inline_completion.projects.iter().any(|k| k == key);
                if ui.checkbox(&mut enabled, "").changed() {
                    config.inline_completion.projects.retain(|k| k != key);
                    if enabled {
                        config.inline_completion.projects.push(key.to_string());
                    }
                    *changed = true;
                }
            } else {
                ui.label(
                    RichText::new(t!("Open a local project to allow cloud completion.")).small().color(super::settings_layout::HELP_TEXT),
                );
            }
        });
        super::settings_layout::row(ui, t!("Provider"), |ui| {
            use crate::inline_completion::ProviderMode;
            egui::ComboBox::from_id_salt("provider")
                .selected_text(match config.inline_completion.provider {
                    ProviderMode::MercuryEdit2 => t!("Mercury Edit 2 · FIM"),
                    ProviderMode::OpenRouterChat => t!("OpenRouter · Chat fallback"),
                })
                .show_ui(ui, |ui| {
                    *changed |= ui
                        .selectable_value(&mut config.inline_completion.provider, ProviderMode::MercuryEdit2, t!("Mercury Edit 2 · FIM"))
                        .changed();
                    *changed |= ui
                        .selectable_value(
                            &mut config.inline_completion.provider,
                            ProviderMode::OpenRouterChat,
                            t!("OpenRouter · Chat fallback"),
                        )
                        .changed();
                });
        });
        super::settings_layout::row(ui, t!("Model"), |ui| {
            if config.inline_completion.provider == crate::inline_completion::ProviderMode::MercuryEdit2 {
                ui.label(RichText::new("mercury-edit-2").monospace());
                ui.label(
                    RichText::new(t!("FIM endpoint: FIM_PROVIDER + /v1/fim/completions · model mercury-edit-2"))
                        .small()
                        .color(super::settings_layout::HELP_TEXT),
                );
                ui.label(
                    RichText::new(t!("Key: INCEPTION_API_KEY from environment or project .env-fim; it is never copied."))
                        .small()
                        .color(super::settings_layout::HELP_TEXT),
                );
            } else {
                *changed |= ui.add(egui::TextEdit::singleline(&mut config.inline_completion.model).desired_width(340.0)).changed();
                ui.label(
                    RichText::new(t!("OpenRouter key: Pi credentials, then OPENROUTER_API_KEY. No key is copied."))
                        .small()
                        .color(super::settings_layout::HELP_TEXT),
                );
            }
        });
        super::settings_layout::row(ui, t!("Temperature"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.inline_completion.temperature, 0.0..=1.0)).changed();
        });
        super::settings_layout::row(ui, t!("FIM length"), |ui| {
            ui.horizontal_wrapped(|ui| {
                use crate::inline_completion::FimProfile;
                let current = FimProfile::of(config.inline_completion.max_tokens);
                for profile in FimProfile::ALL {
                    let label = match profile {
                        FimProfile::Short => t!("Short hint"),
                        FimProfile::MultiLine => t!("Several lines"),
                    };
                    if ui.selectable_label(current == Some(profile), format!("{label} · {}", profile.max_tokens())).clicked() {
                        config.inline_completion.max_tokens = profile.max_tokens();
                        *changed = true;
                    }
                }
            });
        });
        super::settings_layout::row(ui, t!("Max tokens"), |ui| {
            *changed |=
                ui.add(egui::Slider::new(&mut config.inline_completion.max_tokens, 1..=crate::inline_completion::MAX_TOKENS)).changed();
        });
        super::settings_layout::row(ui, t!("Editable lines"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.next_edit_region_lines, 1..=15)).changed();
        });

        settings_section(ui, t!("Limits"));
        ui.label(
            RichText::new(t!("Session limits below are shared by FIM and Next Edit.")).small().color(super::settings_layout::HELP_TEXT),
        );
        super::settings_layout::row(ui, t!("Requests / minute"), |ui| {
            *changed |= ui.add(egui::Slider::new(&mut config.inline_completion.requests_per_minute, 1..=60)).changed();
        });
        super::settings_layout::row(ui, t!("Requests / session"), |ui| {
            *changed |= ui.add(egui::DragValue::new(&mut config.inline_completion.requests_per_session).range(1..=100_000)).changed();
        });
        super::settings_layout::row(ui, t!("Session budget, USD"), |ui| {
            *changed |=
                ui.add(egui::DragValue::new(&mut config.inline_completion.session_budget_usd).range(0.0..=100.0).speed(0.1)).changed();
        });

        ui.label(t!("Tab accepts; Esc dismisses; Ctrl+Alt+Space requests AI completion."));
        ui.label(t!("Manual request: Ctrl+Alt+E. Results open in a review preview; Tab remains FIM."));
    });
}

fn servers_settings_ui(ui: &mut egui::Ui, app: &mut App, changed: &mut bool) {
    egui::ScrollArea::vertical().id_salt("settings_servers").auto_shrink([false, false]).show(ui, |ui| {
        let config = &mut app.config;

        settings_section(ui, t!("Language servers"));
        ui.label(
            RichText::new(t!("Disabled servers are not started. Add your own servers in config.toml ([[lsp]])."))
                .small()
                .color(super::settings_layout::HELP_TEXT),
        );
        for spec in crate::lsp::default_servers().into_iter().chain(config.lsp.clone()) {
            let mut enabled = !config.disabled_servers.contains(&spec.name);
            if super::settings_layout::server(
                ui,
                &mut enabled,
                &spec.name,
                &format!("{} {}", spec.command, spec.args.join(" ")),
                &spec.languages.join(", "),
            ) {
                if enabled {
                    config.disabled_servers.retain(|n| n != &spec.name);
                } else {
                    config.disabled_servers.push(spec.name.clone());
                }
                *changed = true;
            }
        }

        settings_section(ui, t!("pi agent"));
        super::settings_layout::row(ui, t!("pi agent: command"), |ui| {
            let command_response = ui.add(egui::TextEdit::singleline(&mut config.pi_command).desired_width(200.0));
            if command_response.changed() {
                config.pi_entry.clear();
                config.pi_runtime.clear();
            }
            *changed |= command_response.lost_focus();
        });
        super::settings_layout::row(ui, t!("pi agent: mode at start"), |ui| {
            *changed |= ui.add(egui::TextEdit::singleline(&mut config.agent_mode).hint_text("build").desired_width(120.0)).lost_focus();
        });
        super::settings_layout::row(ui, t!("pi agent: sandbox (bwrap)"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.agent_sandbox, Some(t!("Write only to the project folder, ~/.pi and caches")));
        });

        let install_id = ui.make_persistent_id("pi_user_install_state");
        let install_state =
            ui.ctx().data_mut(|data| data.get_temp::<std::sync::Arc<std::sync::Mutex<crate::pi_install::InstallState>>>(install_id));
        let installing = install_state
            .as_ref()
            .is_some_and(|state| matches!(&*state.lock().unwrap_or_else(|e| e.into_inner()), crate::pi_install::InstallState::Running(_)));
        ui.vertical(|ui| {
            if ui.add_enabled(!installing, egui::Button::new(t!("Install Pi for this user"))).clicked() {
                let state = crate::pi_install::start(ui.ctx().clone());
                ui.ctx().data_mut(|data| data.insert_temp(install_id, state));
            }
            ui.label(
                RichText::new(t!("Pi includes its own private Node.js runtime; no system Node.js or administrator rights are needed."))
                    .small()
                    .color(super::settings_layout::HELP_TEXT),
            );
        });
        if let Some(state) = install_state {
            match state.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                crate::pi_install::InstallState::Running(message) => {
                    ui.label(RichText::new(message).small().color(super::settings_layout::HELP_TEXT));
                }
                crate::pi_install::InstallState::Finished(Ok(installed)) => {
                    if config.pi_command != installed.node.to_string_lossy()
                        || config.pi_entry != installed.entry.to_string_lossy()
                        || config.pi_runtime != installed.runtime_dir.to_string_lossy()
                    {
                        config.pi_command = installed.node.to_string_lossy().into_owned();
                        config.pi_entry = installed.entry.to_string_lossy().into_owned();
                        config.pi_runtime = installed.runtime_dir.to_string_lossy().into_owned();
                        *changed = true;
                    }
                    ui.label(RichText::new(t!("Pi is installed for this user.")).small().color(theme::SUCCESS));
                }
                crate::pi_install::InstallState::Finished(Err(error)) => {
                    ui.label(RichText::new(t!("Pi installation failed: {error}", error = error)).small().color(theme::ERROR));
                }
            }
        }

        settings_section(ui, t!("Remote"));
        super::settings_layout::row(ui, t!("Agent on SSH hosts"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.use_agent, Some(t!("Fast search and file watching on the remote machine")));
        });
        super::settings_layout::row(ui, t!("Server Pulse"), |ui| {
            *changed |= settings_checkbox(ui, &mut config.pulse, Some(t!("CPU, memory and disk of the project host in the status bar")));
        });
    });
}

pub fn settings_tab_ui(ui: &mut egui::Ui, app: &mut App) {
    let mut changed = false;
    let completion_project = app.workspace.as_ref().filter(|w| !w.host.is_remote()).map(|w| w.session_key());
    let tab_id = ui.make_persistent_id("settings_active_tab");
    let mut tab: SettingsTab = match ui.ctx().data_mut(|d| d.get_temp::<u8>(tab_id)) {
        Some(0) => SettingsTab::Editor,
        Some(1) => SettingsTab::Ai,
        Some(2) => SettingsTab::Pi,
        Some(3) => SettingsTab::Servers,
        _ => SettingsTab::Editor,
    };

    egui::Frame::new().inner_margin(24.0).show(ui, |ui| {
        super::settings_layout::apply_style(ui);
        ui.set_width(ui.available_width().min(1400.0));
        let mut selected = tab_index(&tab);
        super::settings_layout::page(ui, &mut selected, |ui, index| match index {
            0 => editor_settings_ui(ui, app, &mut changed),
            1 => ai_settings_ui(ui, app, completion_project.as_deref(), &mut changed),
            2 => super::pi_settings::show(ui, app),
            _ => servers_settings_ui(ui, app, &mut changed),
        });
        tab = match selected {
            0 => SettingsTab::Editor,
            1 => SettingsTab::Ai,
            2 => SettingsTab::Pi,
            _ => SettingsTab::Servers,
        };

        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(t!("Keyboard Shortcuts…")).clicked() {
                super::keybindings::open(app);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(t!("Settings file: {n1}", n1 = crate::config::config_path().display()))
                        .small()
                        .color(super::settings_layout::HELP_TEXT),
                );
            });
        });
    });

    ui.ctx().data_mut(|d| d.insert_temp(tab_id, tab_index(&tab) as u8));
    if changed {
        app.config.save();
    }
}
