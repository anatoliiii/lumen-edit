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
                let color = if response.hovered() || app.settings_open { theme::TEXT } else { theme::FAINT_TEXT };
                draw_icon(ui.painter(), Icon::Settings, rect.center(), color);
                if response.on_hover_text(t!("Settings (Ctrl+,)")).clicked() {
                    app.settings_open = !app.settings_open;
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
                            RichText::new(t!("Agent: {n1}", n1 = remote.agent_status.lock().unwrap())).small().color(theme::MUTED_TEXT),
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
                    let label = if busy {
                        "AI …"
                    } else if status.is_empty() {
                        "AI"
                    } else {
                        "AI !"
                    };
                    if status_button(ui, label).on_hover_text(if status.is_empty() { t!("AI Inline Completion") } else { status }).clicked()
                    {
                        app.settings_open = true;
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

pub fn settings_window(ctx: &egui::Context, app: &mut App) {
    let mut open = app.settings_open;
    let mut changed = false;
    let completion_project = app.workspace.as_ref().filter(|w| !w.host.is_remote()).map(|w| w.session_key());
    egui::Window::new(t!("Settings")).open(&mut open).resizable(true).default_width(460.0).vscroll(true).collapsible(false).show(ctx, |ui| {
        let config = &mut app.config;
        egui::Grid::new("settings_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label(t!("Language"));
            let name = |code: &str| crate::i18n::SETTINGS.iter().find(|(c, _)| *c == code).map_or(code.to_string(), |(_, n)| n.to_string());
            let auto = |label: String| if label == "Auto" { t!("Auto (system)").to_string() } else { label };
            egui::ComboBox::from_id_salt("language").selected_text(auto(name(&config.language))).show_ui(ui, |ui| {
                for (code, label) in crate::i18n::SETTINGS {
                    if ui.selectable_label(config.language == *code, auto(label.to_string())).clicked() && config.language != *code {
                        config.language = code.to_string();
                        crate::i18n::set(crate::i18n::from_setting(code));
                        changed = true;
                    }
                }
            });
            ui.end_row();

            ui.label(t!("Editor font"));
            changed |= ui.add(egui::Slider::new(&mut config.editor_font_size, 9.0..=28.0)).changed();
            ui.end_row();
            ui.label(t!("Terminal font"));
            changed |= ui.add(egui::Slider::new(&mut config.terminal_font_size, 9.0..=28.0)).changed();
            ui.end_row();
            ui.label(t!("Interface font"));
            let ui_font = ui.add(egui::Slider::new(&mut config.ui_font_size, 10.0..=20.0));
            if ui_font.drag_stopped() || ui_font.lost_focus() {
                theme::apply(ui.ctx(), config.ui_font_size);
                changed = true;
            }
            ui.end_row();
            ui.label(t!("Tab width"));
            changed |= ui.add(egui::Slider::new(&mut config.tab_size, 1..=8)).changed();
            ui.end_row();
            ui.label(t!("Indent with spaces"));
            changed |= ui.checkbox(&mut config.insert_spaces, "").changed();
            ui.end_row();
            ui.label(t!("Format on save"));
            changed |= ui.checkbox(&mut config.format_on_save, "").changed();
            ui.end_row();
            ui.label(t!("Auto save"));
            changed |= ui.checkbox(&mut config.auto_save, "").changed();
            ui.end_row();
            ui.label(t!("Show whitespace"));
            changed |= ui.checkbox(&mut config.show_whitespace, "").changed();
            ui.end_row();
            ui.label(t!("Line blame"));
            changed |= ui.checkbox(&mut config.inline_blame, "").changed();
            ui.end_row();
            ui.label(t!("Agent on SSH hosts"));
            changed |=
                ui.checkbox(&mut config.use_agent, "").on_hover_text(t!("Fast search and file watching on the remote machine")).changed();
            ui.end_row();
            ui.label(t!("pi agent: command"));
            changed |= ui.add(egui::TextEdit::singleline(&mut config.pi_command).desired_width(200.0)).lost_focus();
            ui.end_row();
            ui.label(t!("Server Pulse"));
            changed |= ui
                .checkbox(&mut config.pulse, "")
                .on_hover_text(t!("CPU, memory and disk of the project host in the status bar"))
                .changed();
            ui.end_row();
            ui.label(t!("pi agent: model"));
            changed |=
                ui.add(egui::TextEdit::singleline(&mut config.agent_model).hint_text("provider/id").desired_width(260.0)).lost_focus();
            ui.end_row();
            ui.label(t!("pi agent: mode at start"));
            changed |= ui.add(egui::TextEdit::singleline(&mut config.agent_mode).hint_text("build").desired_width(120.0)).lost_focus();
            ui.end_row();
            ui.label(t!("pi agent: sandbox (bwrap)"));
            changed |= ui
                .checkbox(&mut config.agent_sandbox, "")
                .on_hover_text(t!("Write only to the project folder, ~/.pi and caches"))
                .changed();
            ui.end_row();
            ui.label("git pull --rebase");
            changed |= ui.checkbox(&mut config.pull_rebase, "").changed();
            ui.end_row();
        });
        ui.separator();
        ui.collapsing(t!("AI Inline Completion"), |ui| {
            changed |= ui.checkbox(&mut config.inline_completion.paused, t!("Pause AI completion in all projects")).changed();
            ui.label(t!("Code context is sent to OpenRouter only for allowed projects."));
            if let Some(key) = &completion_project {
                let mut enabled = config.inline_completion.projects.contains(key);
                if ui.checkbox(&mut enabled, t!("Allow cloud completion for this project")).changed() {
                    config.inline_completion.projects.retain(|k| k != key);
                    if enabled {config.inline_completion.projects.push(key.clone());}
                    changed = true;
                }
            } else {ui.label(t!("Open a local project to allow cloud completion."));}
            ui.label(t!("OpenRouter key: Pi credentials, then OPENROUTER_API_KEY. No key is copied."));
            egui::Grid::new("inline_completion_settings").num_columns(2).show(ui, |ui| {
                ui.label(t!("Model"));changed |= ui.text_edit_singleline(&mut config.inline_completion.model).changed();ui.end_row();
                ui.label("Temperature");changed |= ui.add(egui::Slider::new(&mut config.inline_completion.temperature,0.0..=1.0)).changed();ui.end_row();
                ui.label("Max tokens");changed |= ui.add(egui::Slider::new(&mut config.inline_completion.max_tokens,16..=512)).changed();ui.end_row();
                ui.label("Requests / minute");changed |= ui.add(egui::Slider::new(&mut config.inline_completion.requests_per_minute,1..=60)).changed();ui.end_row();
                ui.label("Session budget, USD");changed |= ui.add(egui::DragValue::new(&mut config.inline_completion.session_budget_usd).range(0.0..=100.0).speed(0.1)).changed();ui.end_row();
            });
            ui.label(t!("Tab accepts; Esc dismisses; Ctrl+Alt+Space requests AI completion."));
            let m=&app.inline_completion.metrics;
            ui.label(format!("Requests {} · success {} · errors {} · cache {} · latency {} ms\nAccepted {} · rejected {} · tokens {} / {}\nCost ${:.4} · reserved ${:.4}",m.requests,m.success,m.errors,m.cache_hits,m.latency_ms,m.accepted,m.rejected,m.input_tokens,m.output_tokens,m.cost_usd,m.reserved_usd));
            if !app.inline_completion.status.is_empty() {ui.label(&app.inline_completion.status);}
        });
        ui.separator();
        ui.label(RichText::new(t!("Language servers")).strong());
        ui.label(
            RichText::new(t!("Disabled servers are not started. Add your own servers in config.toml ([[lsp]])."))
                .small()
                .color(theme::MUTED_TEXT),
        );
        egui::Grid::new("servers_grid").num_columns(3).spacing([12.0, 4.0]).show(ui, |ui| {
            for spec in crate::lsp::default_servers().into_iter().chain(config.lsp.clone()) {
                let mut enabled = !config.disabled_servers.contains(&spec.name);
                if ui.checkbox(&mut enabled, &spec.name).changed() {
                    if enabled {
                        config.disabled_servers.retain(|n| n != &spec.name);
                    } else {
                        config.disabled_servers.push(spec.name.clone());
                    }
                    changed = true;
                }
                ui.label(RichText::new(format!("{} {}", spec.command, spec.args.join(" "))).monospace().small());
                ui.label(RichText::new(spec.languages.join(", ")).small().color(theme::MUTED_TEXT));
                ui.end_row();
            }
        });
        ui.separator();
        if ui.button(t!("Keyboard Shortcuts…")).clicked() {
            super::keybindings::open(app);
        }
        ui.separator();
        ui.label(RichText::new(t!("Settings file: {n1}", n1 = crate::config::config_path().display())).small().color(theme::FAINT_TEXT));
    });
    app.settings_open = open;
    if changed {
        app.config.save();
    }
}
