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
        .frame(egui::Frame::new().fill(theme::ACTIVITY_BACKGROUND).inner_margin(egui::Margin::symmetric(6, 0)).stroke(Stroke::new(1.0, theme::BORDER)))
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.label(RichText::new("◆").color(theme::ACCENT));
                ui.menu_button("Файл", |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::OpenFolder, Command::ConnectSsh, Command::OpenRecent] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::NewFile, Command::NewFolder, Command::Save, Command::SaveAll, Command::CloseTab] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    let mut auto_save = app.config.auto_save;
                    if ui.checkbox(&mut auto_save, "Автосохранение").changed() {
                        app.config.auto_save = auto_save;
                        app.config.save();
                    }
                    menu_item(ui, app, Command::Settings);
                    ui.separator();
                    menu_item(ui, app, Command::CloseWorkspace);
                    menu_item(ui, app, Command::Quit);
                });
                ui.menu_button("Правка", |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::Find, Command::Replace, Command::ShowSearch] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::Format, Command::Rename, Command::CodeActions] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Вид", |ui| {
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
                    if ui.checkbox(&mut whitespace, "Показывать пробелы").changed() {
                        app.config.show_whitespace = whitespace;
                        app.config.save();
                    }
                    for command in [Command::ZoomIn, Command::ZoomOut, Command::ZoomReset] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Перейти", |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::QuickOpen, Command::GotoLine, Command::GotoDefinition, Command::References, Command::NextTab, Command::PrevTab] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Запуск", |ui| {
                    ui.set_min_width(300.0);
                    for command in [Command::RunDefault, Command::RunTask, Command::Build, Command::Test, Command::RerunLast, Command::KillTerminal, Command::EditTasks] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Терминал", |ui| {
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
                    for command in [Command::GitCheckout, Command::GitNewBranch, Command::GitMerge, Command::GitStash, Command::GitStashPop] {
                        menu_item(ui, app, command);
                    }
                    ui.separator();
                    for command in [Command::GitLog, Command::GitFileHistory, Command::GitToggleBlame, Command::GitInit] {
                        menu_item(ui, app, command);
                    }
                });
                ui.menu_button("Справка", |ui| {
                    ui.set_min_width(300.0);
                    menu_item(ui, app, Command::CommandPalette);
                    menu_item(ui, app, Command::RestartLsp);
                    ui.label(RichText::new(format!("Codium RS {}", env!("CARGO_PKG_VERSION"))).small().color(theme::FAINT_TEXT));
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
                (Icon::Files, Some(SideView::Explorer), "Проводник (Ctrl+Shift+E)", 0),
                (Icon::Search, Some(SideView::Search), "Поиск (Ctrl+Shift+F)", 0),
                (Icon::Git, Some(SideView::Git), "Git (Ctrl+Shift+G)", changes),
                (Icon::Run, Some(SideView::Run), "Запуск и задачи (Ctrl+Shift+R)", 0),
                (Icon::Terminal, None, "Терминал (Ctrl+`)", 0),
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
                    ui.painter().line_segment([rect.left_top() + vec2(1.0, 6.0), rect.left_bottom() + vec2(1.0, -6.0)], Stroke::new(2.0, theme::ACCENT));
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
            let agent_active = matches!(app.active_tab.and_then(|i| app.tabs.get(i)), Some(Tab::Agent(_)));
            let busy = app.tabs.iter().any(|t| matches!(t, Tab::Agent(a) if a.streaming));
            let color = if agent_active || response.hovered() { theme::TEXT } else { theme::FAINT_TEXT };
            if agent_active {
                ui.painter().line_segment([rect.left_top() + vec2(1.0, 6.0), rect.left_bottom() + vec2(1.0, -6.0)], Stroke::new(2.0, theme::ACCENT));
            }
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "π", FontId::proportional(21.0), color);
            if busy {
                ui.painter().circle_filled(rect.center() + vec2(10.0, 9.0), 4.0, theme::SUCCESS);
            }
            if response.on_hover_text("Агент pi (Ctrl+Shift+A)").clicked() {
                app.run_command(Command::Agent);
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                let (rect, response) = ui.allocate_exact_size(vec2(46.0, 44.0), Sense::click());
                let color = if response.hovered() || app.settings_open { theme::TEXT } else { theme::FAINT_TEXT };
                draw_icon(ui.painter(), Icon::Settings, rect.center(), color);
                if response.on_hover_text("Настройки (Ctrl+,)").clicked() {
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
                let (label, fill) = match app.workspace.as_ref().map(|w| &w.host) {
                    Some(Host::Remote(remote)) => (format!("  ⇄ SSH: {}  ", remote.target), theme::REMOTE_BADGE),
                    _ => ("  ⌂ Локально  ".to_string(), theme::ACCENT_DIM),
                };
                let badge = ui.add(egui::Button::new(RichText::new(label).size(12.0).color(Color32::WHITE)).fill(fill).corner_radius(0.0).min_size(vec2(0.0, 24.0)));
                let popup_id = ui.make_persistent_id("host_menu");
                if badge.clicked() {
                    egui::Popup::toggle_id(ui.ctx(), popup_id);
                }
                egui::Popup::menu(&badge).id(popup_id).show(|ui| {
                    ui.set_min_width(240.0);
                    if ui.button("Подключиться по SSH…").clicked() {
                        app.run_command(Command::ConnectSsh);
                        ui.close();
                    }
                    if ui.button("Открыть папку…").clicked() {
                        app.run_command(Command::OpenFolder);
                        ui.close();
                    }
                    if ui.button("Недавние проекты…").clicked() {
                        app.run_command(Command::OpenRecent);
                        ui.close();
                    }
                    if app.workspace.is_some() && ui.button("Закрыть проект").clicked() {
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
                    if status_button(ui, &format!("⎇ {branch}{dirty}")).on_hover_text("Переключить ветку").clicked() {
                        pending = Some(Command::GitCheckout);
                    }
                    let sync = if workspace.git.busy.is_some() {
                        "⟳ …".to_string()
                    } else if status.upstream.is_none() {
                        "↑".to_string()
                    } else {
                        format!("{}↓ {}↑", status.behind, status.ahead)
                    };
                    if status_button(ui, &sync).on_hover_text("Синхронизировать").clicked() {
                        pending = Some(if status.upstream.is_none() { Command::GitPush } else { Command::GitSync });
                    }
                }
                if let Some(command) = pending {
                    app.run_command(command);
                }

                let (errors, warnings) = panel::status_counts(app);
                ui.add_space(4.0);
                if status_button(ui, &format!("⊗ {errors}  ▲ {warnings}")).on_hover_text("Проблемы (Ctrl+Shift+M)").clicked() {
                    app.run_command(Command::ShowProblems);
                }
                if let Some((server, progress)) = app.lsp.progress.iter().next() {
                    ui.add_space(8.0);
                    ui.label(RichText::new(format!("{server}: {}", util::truncate_middle(progress, 50))).size(11.5).color(theme::MUTED_TEXT));
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
                        let tooltip: Vec<String> = servers.iter().map(|(n, s, e)| format!("{n}: {s}{}", e.as_ref().map(|e| format!(" — {e}")).unwrap_or_default())).collect();
                        if status_button(ui, &text.join("  ")).on_hover_text(tooltip.join("\n")).clicked() {
                            app.run_command(Command::ShowOutput);
                        }
                    }
                    let Some(index) = app.active_tab else { return };
                    let Some(Tab::Doc(doc)) = app.tabs.get(index) else { return };
                    status_label(ui, doc.lang.display_name());
                    status_label(ui, if doc.buffer.crlf { "CRLF" } else { "LF" });
                    status_label(ui, "UTF-8");
                    let indent = if doc.buffer.insert_spaces { format!("Пробелы: {}", doc.buffer.tab_size) } else { format!("Табуляция: {}", doc.buffer.tab_size) };
                    let indent_button = status_button(ui, &indent);
                    let mut toggle_indent = false;
                    if indent_button.clicked() {
                        toggle_indent = true;
                    }
                    let primary = doc.buffer.primary();
                    let (line, col) = doc.buffer.line_col(primary.head);
                    let mut position = format!("Стр {}, Стлб {}", line + 1, col + 1);
                    let selected: usize = doc.buffer.selections.iter().map(|s| s.end() - s.start()).sum();
                    if selected > 0 {
                        position.push_str(&format!(" (выделено {selected})"));
                    }
                    if doc.buffer.selections.len() > 1 {
                        position.push_str(&format!(" · курсоров {}", doc.buffer.selections.len()));
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
    let response = ui.add(egui::Button::new(RichText::new(text).size(12.0).color(theme::MUTED_TEXT)).frame(false).min_size(vec2(0.0, 22.0)));
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
    egui::Window::new("Настройки").open(&mut open).resizable(true).default_width(460.0).collapsible(false).show(ctx, |ui| {
        let config = &mut app.config;
        egui::Grid::new("settings_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.label("Шрифт редактора");
            changed |= ui.add(egui::Slider::new(&mut config.editor_font_size, 9.0..=28.0)).changed();
            ui.end_row();
            ui.label("Шрифт терминала");
            changed |= ui.add(egui::Slider::new(&mut config.terminal_font_size, 9.0..=28.0)).changed();
            ui.end_row();
            ui.label("Шрифт интерфейса");
            let ui_font = ui.add(egui::Slider::new(&mut config.ui_font_size, 10.0..=20.0));
            if ui_font.drag_stopped() || ui_font.lost_focus() {
                theme::apply(ui.ctx(), config.ui_font_size);
                changed = true;
            }
            ui.end_row();
            ui.label("Ширина табуляции");
            changed |= ui.add(egui::Slider::new(&mut config.tab_size, 1..=8)).changed();
            ui.end_row();
            ui.label("Отступ пробелами");
            changed |= ui.checkbox(&mut config.insert_spaces, "").changed();
            ui.end_row();
            ui.label("Форматировать при сохранении");
            changed |= ui.checkbox(&mut config.format_on_save, "").changed();
            ui.end_row();
            ui.label("Автосохранение");
            changed |= ui.checkbox(&mut config.auto_save, "").changed();
            ui.end_row();
            ui.label("Показывать пробелы");
            changed |= ui.checkbox(&mut config.show_whitespace, "").changed();
            ui.end_row();
            ui.label("Авторство строки (blame)");
            changed |= ui.checkbox(&mut config.inline_blame, "").changed();
            ui.end_row();
            ui.label("Агент на SSH-хостах");
            changed |= ui.checkbox(&mut config.use_agent, "").on_hover_text("Быстрый поиск и слежение за файлами на удалённой машине").changed();
            ui.end_row();
            ui.label("Агент pi: команда");
            changed |= ui.add(egui::TextEdit::singleline(&mut config.pi_command).desired_width(200.0)).lost_focus();
            ui.end_row();
            ui.label("Агент pi: модель");
            changed |= ui.add(egui::TextEdit::singleline(&mut config.agent_model).hint_text("provider/id").desired_width(260.0)).lost_focus();
            ui.end_row();
            ui.label("Агент pi: режим при старте");
            changed |= ui.add(egui::TextEdit::singleline(&mut config.agent_mode).hint_text("build").desired_width(120.0)).lost_focus();
            ui.end_row();
            ui.label("Агент pi: песочница (bwrap)");
            changed |= ui.checkbox(&mut config.agent_sandbox, "").on_hover_text("Запись только в папку проекта, ~/.pi и кэши").changed();
            ui.end_row();
            ui.label("git pull --rebase");
            changed |= ui.checkbox(&mut config.pull_rebase, "").changed();
            ui.end_row();
        });
        ui.separator();
        ui.label(RichText::new("Языковые серверы").strong());
        ui.label(RichText::new("Отключённые не запускаются. Свои серверы добавляются в config.toml ([[lsp]]).").small().color(theme::MUTED_TEXT));
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
        ui.label(RichText::new(format!("Файл настроек: {}", crate::config::config_path().display())).small().color(theme::FAINT_TEXT));
    });
    app.settings_open = open;
    if changed {
        app.config.save();
    }
}
