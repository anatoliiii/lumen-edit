//! "About" window: what the product is and who made it.

use eframe::egui::{self, RichText};

use super::App;
use crate::theme;

pub const PRODUCT: &str = "Lumen Edit";
pub fn tagline() -> &'static str {
    t!("Native code editor without Electron or JavaScript")
}
/// The author's name as written in the interface language.
pub fn author() -> &'static str {
    t!("Anatolii Kopeikin")
}
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMMIT: &str = env!("LUMEN_COMMIT");
pub const BUILD_DATE: &str = env!("LUMEN_BUILD_DATE");

fn features() -> [&'static str; 7] {
    [
        t!("Projects locally and over SSH via system OpenSSH, with a tiny agent on the server"),
        t!("Language servers (LSP): Rust, C/C++, Go, Python, C#, TS/JS, HTML, CSS, TOML, Bash"),
        t!("tree-sitter highlighting, multiple cursors, project-wide search and replace"),
        t!("Built-in terminal, run and build tasks"),
        t!("Full Git: staging hunks, branches, stash, graph history, conflicts, remotes"),
        t!("pi agent in a tab, sandboxed to the project"),
        t!("Control from the CLI and via MCP for AI agents"),
    ]
}

pub fn version_line() -> String {
    format!("{PRODUCT} {VERSION} ({COMMIT}, {BUILD_DATE}, {} {})", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn window(ctx: &egui::Context, app: &mut App) {
    let mut open = app.about_open;
    egui::Window::new(t!("About"))
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
                ui.label(RichText::new(tagline()).color(theme::MUTED_TEXT));
                ui.label(
                    RichText::new(t!(
                        "Version {VERSION} · build {COMMIT} · {BUILD_DATE}",
                        VERSION = VERSION,
                        COMMIT = COMMIT,
                        BUILD_DATE = BUILD_DATE
                    ))
                    .small()
                    .color(theme::FAINT_TEXT),
                );
            });
            ui.add_space(10.0);
            ui.label(t!("Lumen Edit is an editor for fast work with projects on your machine and on remote servers over SSH: open a project, fix the code, run it and commit — without a heavy browser engine."));
            ui.add_space(8.0);
            for feature in features() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("•").color(theme::ACCENT));
                    ui.label(feature);
                });
            }
            ui.add_space(10.0);
            ui.separator();
            egui::Grid::new("about_grid").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new(t!("Author")).color(theme::MUTED_TEXT));
                ui.label(RichText::new(author()).strong());
                ui.end_row();
                ui.label(RichText::new(t!("Source code")).color(theme::MUTED_TEXT));
                ui.hyperlink_to(REPOSITORY.trim_start_matches("https://"), REPOSITORY);
                ui.end_row();
                ui.label(RichText::new(t!("License")).color(theme::MUTED_TEXT));
                ui.label("MIT");
                ui.end_row();
                ui.label(RichText::new(t!("Built with")).color(theme::MUTED_TEXT));
                ui.label("Rust · egui · wgpu · tree-sitter");
                ui.end_row();
                ui.label(RichText::new(t!("Build")).color(theme::MUTED_TEXT));
                ui.label(RichText::new(format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)).monospace());
                ui.end_row();
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(t!("Copy version")).clicked() {
                    ui.ctx().copy_text(version_line());
                }
                if ui.button(t!("Close")).clicked() {
                    app.about_open = false;
                }
            });
        });
    if !open {
        app.about_open = false;
    }
}
