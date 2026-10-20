#![allow(clippy::type_complexity, clippy::enum_variant_names)]

/// Debug logging to stderr, enabled with `CODIUM_LOG=1`.
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var_os("CODIUM_LOG").is_some() {
            eprintln!("[codium] {}", format!($($arg)*));
        }
    };
}
pub(crate) use debug_log;

mod app;
mod cli;
mod control;
mod config;
mod git;
mod host;
mod lsp;
mod syntax;
mod tasks;
mod terminal;
mod text;
mod theme;

/// Positional arguments for the GUI (project path, file or ssh:// URL).
static GUI_ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

pub fn gui_args() -> &'static [String] {
    GUI_ARGS.get().map(Vec::as_slice).unwrap_or(&[])
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = cli::run(&args) {
        return code;
    }
    host::init_shell_path();
    let gui: Vec<String> = args.into_iter().filter(|a| a != "--new-window" && a != "gui").collect();
    let _ = GUI_ARGS.set(gui);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Codium RS")
            .with_app_id("codium-rs")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let result = eframe::run_native("Codium RS", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))));
    control::cleanup();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("codium-rs: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
