//! Windows.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

pub use uds_windows::{UnixListener as ControlListener, UnixStream as ControlStream};

use super::FontCandidates;

/// Separator of directories in PATH.
pub const PATH_SEPARATOR: char = ';';

/// PATH when none is inherited.
pub const DEFAULT_PATH: &str = r"C:\Windows\System32;C:\Windows";

/// Windows has no symlinks without extra rights, so the "current editor"
/// link is a file holding the socket path.
pub fn mark_current(socket: &Path, link: &Path) {
    let _ = std::fs::write(link, socket.to_string_lossy().as_bytes());
}

pub fn current_target(link: &Path) -> Option<PathBuf> {
    std::fs::read_to_string(link).ok().map(|s| PathBuf::from(s.trim()))
}

pub fn current_socket(link: &Path) -> Option<PathBuf> {
    current_target(link)
}

/// `sh` from Git for Windows: internal scripts (transfers, `exec`, process
/// control) are POSIX shell, and Git brings sh with tar, du, rm and awk.
fn git_sh() -> Option<&'static Path> {
    static SH: OnceLock<Option<PathBuf>> = OnceLock::new();
    SH.get_or_init(|| {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path).find_map(|dir| {
            let git = dir.join("git.exe");
            if !git.is_file() {
                return None;
            }
            // <Git>\cmd\git.exe or <Git>\bin\git.exe -> <Git>\bin\sh.exe
            let root = dir.parent()?;
            [root.join(r"bin\sh.exe"), root.join(r"usr\bin\sh.exe")].into_iter().find(|sh| sh.is_file())
        })
    })
    .as_deref()
}

/// Runs a POSIX shell script with Git's sh, or with cmd when Git is missing.
pub fn shell_command(script: &str) -> Command {
    match git_sh() {
        Some(sh) => {
            let mut command = Command::new(sh);
            command.arg("-c").arg(script);
            command
        }
        None => {
            let mut command = Command::new("cmd");
            command.arg("/C").arg(script);
            command
        }
    }
}

/// Whether `dir` contains the program `name` (with any executable extension).
pub fn program_in(dir: &Path, name: &str) -> bool {
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    dir.join(name).is_file()
        || extensions.split(';').any(|ext| !ext.is_empty() && dir.join(format!("{name}{}", ext.to_lowercase())).is_file())
}

/// PowerShell 7 when installed, otherwise the built-in Windows PowerShell.
fn powershell() -> &'static str {
    static SHELL: OnceLock<&'static str> = OnceLock::new();
    SHELL.get_or_init(|| {
        let path = std::env::var_os("PATH").unwrap_or_default();
        if std::env::split_paths(&path).any(|dir| dir.join("pwsh.exe").is_file()) { "pwsh.exe" } else { "powershell.exe" }
    })
}

/// Command for an interactive terminal (or a task) in a local PTY: PowerShell.
pub fn terminal_command(task: Option<&str>, _finish: &str) -> portable_pty::CommandBuilder {
    let mut command = portable_pty::CommandBuilder::new(powershell());
    command.arg("-NoLogo");
    if let Some(script) = task {
        let finish = t!("[process exited with code {code}]", code = "$LASTEXITCODE");
        command.args(["-NoExit", "-Command", &format!("{script}\nWrite-Host \"`n{finish}\"")]);
    }
    command
}

/// Programs started from the Start menu already see the user's PATH.
pub fn login_shell_path() -> Option<String> {
    None
}

/// A local path as the editor writes it: forward slashes, no `\\?\`.
pub fn host_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.trim_start_matches(r"\\?\").replace('\\', "/")
}

/// Whether a path starts with a drive letter (`C:/…`).
pub fn is_drive_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\')
}

/// `C:` is "the current folder of drive C"; the root is `C:/`.
pub fn drive_root(parent: &str) -> Option<String> {
    let bytes = parent.as_bytes();
    (bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()).then(|| format!("{parent}/"))
}

/// `file://` URI of a host path. Local paths are `C:/…`; paths of a Linux
/// project over SSH stay `/…`, which `Url::from_file_path` rejects here.
#[allow(clippy::expect_used, reason = "a constant URL that always parses")]
pub fn path_to_uri(path: &str) -> String {
    let mut url = url::Url::parse("file:///").expect("file:/// parses");
    let path = if is_drive_path(path) { format!("/{}", path.replace('\\', "/")) } else { path.to_string() };
    url.set_path(&path);
    url.to_string()
}

pub fn uri_to_path(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    if url.scheme() != "file" {
        return None;
    }
    let decoded = percent_decode(url.path());
    // `/C:/x` -> `C:/x`; `/home/x` stays.
    Some(match decoded.strip_prefix('/') {
        Some(rest) if is_drive_path(rest) => rest.to_string(),
        _ => decoded,
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
            && let Ok(value) = u8::from_str_radix(&text[i + 1..i + 3], 16)
        {
            out.push(value);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub const FONTS: FontCandidates = FontCandidates {
    ui: &["C:/Windows/Fonts/segoeui.ttf", "C:/Windows/Fonts/arial.ttf"],
    mono: &["C:/Windows/Fonts/CascadiaMono.ttf", "C:/Windows/Fonts/CascadiaCode.ttf", "C:/Windows/Fonts/consola.ttf"],
    nerd: &[],
    symbols: &["C:/Windows/Fonts/tahoma.ttf", "C:/Windows/Fonts/seguisym.ttf", "C:/Windows/Fonts/segoeui.ttf"],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_uris() {
        assert_eq!(host_path(Path::new(r"\\?\C:\Users\me\proj")), "C:/Users/me/proj");
        assert_eq!(drive_root("C:").as_deref(), Some("C:/"));
        assert_eq!(drive_root("/home"), None);
        assert!(is_drive_path("D:/x") && !is_drive_path("/x"));
        assert_eq!(path_to_uri("C:/Users/me/a b.rs"), "file:///C:/Users/me/a%20b.rs");
        assert_eq!(uri_to_path("file:///C:/Users/me/a%20b.rs").as_deref(), Some("C:/Users/me/a b.rs"));
        assert_eq!(uri_to_path(&path_to_uri("/home/user/naïve café/main.rs")).as_deref(), Some("/home/user/naïve café/main.rs"));
    }
}
