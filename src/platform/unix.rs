//! Linux and macOS.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub use std::os::unix::net::{UnixListener as ControlListener, UnixStream as ControlStream};

use super::FontCandidates;

/// Separator of directories in PATH.
pub const PATH_SEPARATOR: char = ':';

/// PATH when none is inherited.
pub const DEFAULT_PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Points the "current editor" link at this editor's socket.
pub fn mark_current(socket: &Path, link: &Path) {
    let _ = std::fs::remove_file(link);
    let _ = std::os::unix::fs::symlink(socket, link);
}

/// The socket the "current editor" link points at.
pub fn current_target(link: &Path) -> Option<PathBuf> {
    std::fs::read_link(link).ok()
}

/// What to connect to for the "current editor": the link itself works.
pub fn current_socket(link: &Path) -> Option<PathBuf> {
    Some(link.to_path_buf())
}

/// Runs a POSIX shell script on this machine.
pub fn shell_command(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(script);
    command
}

/// Whether `dir` contains the program `name`.
pub fn program_in(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file()
}

/// Command for an interactive terminal (or a task) in a local PTY.
/// `finish` is a shell line printing the exit code after a task.
pub fn terminal_command(task: Option<&str>, finish: &str) -> portable_pty::CommandBuilder {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    match task {
        None => portable_pty::CommandBuilder::new(&shell),
        Some(script) => {
            let mut command = portable_pty::CommandBuilder::new("sh");
            command.arg("-c");
            command.arg(format!("{script}\n{finish}\nexec {}", crate::host::shell_quote(&shell)));
            command
        }
    }
}

/// PATH from the user's login shell, so tools installed via shell profiles
/// (pi, cargo, go, ...) are found even when the editor starts from the
/// desktop menu.
pub fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", "printf '__LUMEN_PATH__%s__END__' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("LUMEN_SHELL_PROBE", "1")
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(5) => std::thread::sleep(std::time::Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
    let mut output = String::new();
    use std::io::Read;
    child.stdout.take()?.read_to_string(&mut output).ok()?;
    let start = output.find("__LUMEN_PATH__")? + "__LUMEN_PATH__".len();
    let end = output[start..].find("__END__")? + start;
    Some(output[start..end].to_string()).filter(|p| !p.is_empty())
}

/// A local path as the editor writes it.
pub fn host_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whether a path starts with a drive letter (`C:/…`); never on Unix.
pub fn is_drive_path(_path: &str) -> bool {
    false
}

/// A parent path that needs fixing up (only Windows drive roots do).
pub fn drive_root(_parent: &str) -> Option<String> {
    None
}

/// `file://` URI of a host path for language servers.
pub fn path_to_uri(path: &str) -> String {
    url::Url::from_file_path(path).map(|u| u.to_string()).unwrap_or_else(|_| format!("file://{path}"))
}

pub fn uri_to_path(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    url.to_file_path().ok().map(|p| p.to_string_lossy().into_owned())
}

pub const FONTS: FontCandidates = FontCandidates {
    ui: &[
        "/usr/share/fonts/inter/Inter.ttc",
        "/usr/share/fonts/TTF/Inter-Regular.ttf",
        "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        "/usr/share/fonts/cantarell/Cantarell-VF.otf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ],
    mono: &[
        "/usr/share/fonts/TTF/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/jetbrains-mono/JetBrainsMono-Regular.ttf",
        "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf",
    ],
    nerd: &["/usr/share/fonts/TTF/SymbolsNerdFont-Regular.ttf", "/usr/share/fonts/TTF/SymbolsNerdFontMono-Regular.ttf"],
    symbols: &[
        "/usr/share/fonts/noto/NotoSansSymbols2-Regular.ttf",
        "/usr/share/fonts/noto/NotoSansSymbols-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/noto/NotoSansMath-Regular.ttf",
    ],
};
