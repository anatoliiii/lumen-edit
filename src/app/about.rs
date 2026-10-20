//! "About" window: what the product is and who made it.

use eframe::egui::{self, RichText};

use super::App;
use crate::theme;

pub const PRODUCT: &str = "Lumen Edit";
pub const TAGLINE: &str = "Нативный редактор кода без Electron и JavaScript";
pub const AUTHOR: &str = env!("CARGO_PKG_AUTHORS");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("LUMEN_COMMIT");
pub const BUILD_DATE: &str = env!("LUMEN_BUILD_DATE");

const DESCRIPTION: &str = "Lumen Edit — редактор для быстрой работы с проектами на своей машине и на удалённых \
серверах по SSH: открыть проект, поправить код, запустить и закоммитить — без тяжёлого браузерного движка.";

const FEATURES: &[&str] = &[
    "Проекты локально и по SSH через системный OpenSSH, с маленьким агентом на сервере",
    "Языковые серверы (LSP): Rust, C/C++, Go, Python, C#, TS/JS, HTML, CSS, TOML, Bash",
    "Подсветка tree-sitter, несколько курсоров, поиск и замена по проекту",
    "Встроенный терминал, задачи запуска и сборки",
    "Git целиком: индекс по фрагментам, ветки, stash, история с графом, конфликты, remotes",
    "Агент pi во вкладке, в песочнице проекта",
    "Управление из CLI и через MCP для ИИ-агентов",
];

pub fn version_line() -> String {
    format!("{PRODUCT} {VERSION} ({COMMIT}, {BUILD_DATE}, {} {})", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn window(ctx: &egui::Context, app: &mut App) {
    let mut open = app.about_open;
    egui::Window::new("О программе")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(520.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("◆").size(48.0).color(theme::ACCENT));
                ui.label(RichText::new(PRODUCT).size(26.0).strong());
                ui.label(RichText::new(TAGLINE).color(theme::MUTED_TEXT));
                ui.label(RichText::new(format!("Версия {VERSION} · сборка {COMMIT} · {BUILD_DATE}")).small().color(theme::FAINT_TEXT));
            });
            ui.add_space(10.0);
            ui.label(DESCRIPTION);
            ui.add_space(8.0);
            for feature in FEATURES {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("•").color(theme::ACCENT));
                    ui.label(*feature);
                });
            }
            ui.add_space(10.0);
            ui.separator();
            egui::Grid::new("about_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("Автор").color(theme::MUTED_TEXT));
                ui.label(RichText::new(AUTHOR).strong());
                ui.end_row();
                ui.label(RichText::new("Исходный код").color(theme::MUTED_TEXT));
                ui.hyperlink_to(REPOSITORY.trim_start_matches("https://"), REPOSITORY);
                ui.end_row();
                ui.label(RichText::new("Лицензия").color(theme::MUTED_TEXT));
                ui.label("MIT");
                ui.end_row();
                ui.label(RichText::new("Сделано на").color(theme::MUTED_TEXT));
                ui.label("Rust · egui · wgpu · tree-sitter");
                ui.end_row();
                ui.label(RichText::new("Сборка").color(theme::MUTED_TEXT));
                ui.label(RichText::new(format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)).monospace());
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Скопировать версию").clicked() {
                    ui.ctx().copy_text(version_line());
                }
                if ui.button("Закрыть").clicked() {
                    app.about_open = false;
                }
            });
        });
    if !open {
        app.about_open = false;
    }
}
