#![allow(clippy::type_complexity, clippy::enum_variant_names)]

/// Debug logging to stderr, enabled with `LUMEN_LOG=1`.
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var_os("LUMEN_LOG").is_some() {
            eprintln!("[lumen] {}", format!($($arg)*));
        }
    };
}
pub(crate) use debug_log;

#[macro_use]
mod i18n;

mod app;
mod ci;
mod cli;
mod config;
mod control;
mod git;
mod host;
mod inline_completion;
mod lsp;
mod pi_install;
mod platform;
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
    i18n::set(i18n::from_setting(&config::Config::load().language));
    if let Some(code) = cli::run(&args) {
        return code;
    }
    install_crash_log();
    host::init_shell_path();
    let gui: Vec<String> = args.into_iter().filter(|a| a != "--new-window" && a != "gui").collect();
    let _ = GUI_ARGS.set(gui);

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Lumen Edit")
            .with_app_id("lumen-edit")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    let result = eframe::run_native("Lumen Edit", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))));
    control::cleanup();
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lumen-edit: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

/// Desktop launches have no terminal, so panics also go to
/// `~/.local/state/lumen-edit/crash.log` with a backtrace.
fn install_crash_log() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let Some(dir) = dirs::state_dir().or_else(dirs::cache_dir).map(|d| d.join("lumen-edit")) else { return };
        let _ = std::fs::create_dir_all(&dir);
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current().name().unwrap_or("?").to_string();
        let entry = format!("--- {} v{} thread '{thread}'\n{info}\n{backtrace}\n", chrono_like_now(), env!("CARGO_PKG_VERSION"));
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            let _ = file.write_all(entry.as_bytes());
        }
    }));
}

/// Seconds since the epoch; enough to tell crashes apart.
fn chrono_like_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
