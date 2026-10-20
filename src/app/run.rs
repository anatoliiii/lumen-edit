//! Run & tasks sidebar.

use eframe::egui::{self, RichText};

use super::{App, Msg};
use crate::{
    host,
    tasks::{self, Group},
    theme,
};

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let Some(workspace) = &app.workspace else { return };
    let tasks_list = workspace.tasks.clone();
    let error = workspace.tasks_error.clone();
    let mut run = None;
    let mut refresh = false;
    let mut edit = false;
    let mut rerun = false;

    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("ЗАПУСК").small().strong().color(theme::MUTED_TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("⚙").on_hover_text("Настроить tasks.toml").clicked() {
                    edit = true;
                }
                if ui.small_button("⟳").on_hover_text("Обновить список").clicked() {
                    refresh = true;
                }
            });
        });
        ui.add_space(6.0);
        let default = tasks::default_task(&tasks_list, Group::Run).cloned();
        ui.horizontal(|ui| {
            let label = match &default {
                Some(task) => format!("⏵  {}", task.label),
                None => "⏵  Запуск".to_string(),
            };
            if ui.add(egui::Button::new(RichText::new(label).strong()).fill(theme::ACCENT_DIM)).on_hover_text("F5").clicked() {
                match default {
                    Some(task) => run = Some(task),
                    None => edit = true,
                }
            }
            if app.last_task.is_some() && ui.button("↻").on_hover_text("Повторить последнюю (Ctrl+F5)").clicked() {
                rerun = true;
            }
        });
        if let Some(error) = &error {
            ui.add_space(4.0);
            ui.label(RichText::new(error).small().color(theme::ERROR));
        }
    });

    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for group in [Group::Run, Group::Build, Group::Test, Group::Other] {
            let items: Vec<_> = tasks_list.iter().filter(|t| t.group == group).collect();
            if items.is_empty() {
                continue;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new(group.label().to_uppercase()).small().color(theme::MUTED_TEXT));
            });
            for task in items {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    let response = ui
                        .add(egui::Button::new(format!("⏵ {}", task.label)).frame(false))
                        .on_hover_text(format!("{}\n({})", task.command, task.source));
                    if response.clicked() {
                        run = Some(task.clone());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&task.source).small().color(theme::FAINT_TEXT));
                    });
                });
            }
        }
        if tasks_list.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                ui.label(RichText::new("Задачи не найдены. Создайте .lumen/tasks.toml.").small().color(theme::MUTED_TEXT));
            });
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Терминалы").small().color(theme::MUTED_TEXT));
        });
        let mut focus = None;
        for (index, terminal) in app.terminals.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(10.0);
                let state = if terminal.is_alive() { "●" } else { "○" };
                let color = if terminal.is_alive() { theme::SUCCESS } else { theme::MUTED_TEXT };
                ui.label(RichText::new(state).color(color));
                if ui.add(egui::Button::new(&terminal.title).frame(false)).clicked() {
                    focus = Some(index);
                }
            });
        }
        if let Some(index) = focus {
            app.active_terminal = index;
            app.panel_open = true;
            app.panel_tab = super::PanelTab::Terminal;
        }
    });

    if let Some(task) = run {
        app.run_task(task);
    }
    if rerun {
        app.run_command(super::Command::RerunLast);
    }
    if refresh {
        app.refresh_tasks();
    }
    if edit {
        edit_tasks_file(app);
    }
}

/// Opens `.lumen/tasks.toml`, creating it from a template if needed.
pub fn edit_tasks_file(app: &mut App) {
    let Some(workspace) = &app.workspace else { return };
    let host = workspace.host.clone();
    let path = host::join(&workspace.root, tasks::TASKS_FILE);
    let dir = host::parent(&path);
    app.bus.spawn(move || {
        if host.stat(&path).is_err()
            && let Err(error) = host.create_dir(&dir).and_then(|_| host.write_file(&path, tasks::TASKS_TEMPLATE.as_bytes()))
        {
            return Msg::Notice(format!("Не удалось создать {path}: {error}"), true);
        }
        Msg::FsOpDone { reload: vec![dir], open: Some(path), renamed: None, error: None }
    });
}
