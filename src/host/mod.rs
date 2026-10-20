//! Where a project lives: on this machine or on an SSH host. Every operation
//! here is blocking and meant to be called from worker threads.

mod agent;
mod shell;
pub mod ssh;
pub mod ssh_config;

use std::{
    collections::HashMap,
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use codium_shared::fsops::{self, DirEntry, SearchMatch, SearchQuery, Stat};

pub use ssh::Remote;

/// Extra directories where developer tools usually live; GUI sessions and
/// non-login SSH commands often miss them.
pub const EXTRA_PATH: &[&str] = &[
    ".cargo/bin",
    "go/bin",
    ".local/bin",
    ".dotnet/tools",
    ".bun/bin",
    ".pi/agent/bin",
    ".npm-global/bin",
    "bin",
];

#[derive(Clone)]
pub enum Host {
    Local,
    Remote(Arc<Remote>),
}

impl Host {
    /// Stable key used in recent-project lists and caches.
    pub fn key(&self) -> String {
        match self {
            Host::Local => "local".into(),
            Host::Remote(remote) => format!("ssh:{}", remote.target),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Host::Local => "Локально".into(),
            Host::Remote(remote) => format!("SSH: {}", remote.target),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Host::Remote(_))
    }

    pub fn same(&self, other: &Host) -> bool {
        self.key() == other.key()
    }

    pub fn home(&self) -> String {
        match self {
            Host::Local => dirs::home_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into()),
            Host::Remote(remote) => remote.home.clone(),
        }
    }

    pub fn stat(&self, path: &str) -> Result<Stat, String> {
        match self {
            Host::Local => fsops::stat(path).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.stat(path),
        }
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        match self {
            Host::Local => fsops::read_dir(path).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.read_dir(path),
        }
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        match self {
            Host::Local => std::fs::read(path).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.read_file(path),
        }
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        match self {
            Host::Local => fsops::write_atomic(path, data).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.write_file(path, data),
        }
    }

    pub fn create_dir(&self, path: &str) -> Result<(), String> {
        match self {
            Host::Local => std::fs::create_dir_all(path).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.create_dir(path),
        }
    }

    pub fn remove(&self, path: &str) -> Result<(), String> {
        match self {
            Host::Local => fsops::remove(path).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.remove(path),
        }
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        match self {
            Host::Local => std::fs::rename(from, to).map_err(|e| e.to_string()),
            Host::Remote(remote) => remote.rename(from, to),
        }
    }

    pub fn list_files(&self, root: &str, max: usize) -> Result<Vec<String>, String> {
        match self {
            Host::Local => Ok(fsops::list_files(root, max)),
            Host::Remote(remote) => remote.list_files(root, max),
        }
    }

    pub fn search(&self, root: &str, query: &SearchQuery) -> Result<(Vec<SearchMatch>, bool), String> {
        match self {
            Host::Local => fsops::search(root, query),
            Host::Remote(remote) => remote.search(root, query),
        }
    }

    /// A command that runs `program args` in `cwd` on this host. The caller
    /// sets up stdio and spawns it.
    pub fn command(&self, cwd: &str, program: &str, args: &[String]) -> Command {
        match self {
            Host::Local => {
                let mut command = Command::new(program);
                command.args(args).env("PATH", local_path());
                if Path::new(cwd).is_dir() {
                    command.current_dir(cwd);
                }
                command
            }
            Host::Remote(remote) => {
                let mut line = shell_quote(program);
                for arg in args {
                    line.push(' ');
                    line.push_str(&shell_quote(arg));
                }
                remote.ssh_command(&remote_script(cwd, &format!("exec {line}")), false)
            }
        }
    }

    /// Runs a POSIX shell script on this host.
    pub fn shell(&self, cwd: &str, script: &str) -> Command {
        match self {
            Host::Local => {
                let mut command = Command::new("sh");
                command.arg("-c").arg(script).env("PATH", local_path());
                if Path::new(cwd).is_dir() {
                    command.current_dir(cwd);
                }
                command
            }
            Host::Remote(remote) => remote.ssh_command(&remote_script(cwd, script), false),
        }
    }

    /// Runs a command to completion and returns stdout, or stderr as error.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn output(&self, cwd: &str, program: &str, args: &[String], stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut command = self.command(cwd, program, args);
        run_with_input(&mut command, stdin)
    }

    /// Whether `program` can be found in PATH on this host. Cached.
    pub fn which(&self, program: &str) -> bool {
        static CACHE: Mutex<Option<HashMap<(String, String), bool>>> = Mutex::new(None);
        let key = (self.key(), program.to_string());
        if let Some(found) = CACHE.lock().unwrap().get_or_insert_with(HashMap::new).get(&key) {
            return *found;
        }
        let found = match self {
            Host::Local => which_local(program),
            Host::Remote(_) => self
                .shell("/", &format!("command -v {} >/dev/null 2>&1", shell_quote(program)))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false),
        };
        CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(key, found);
        found
    }

    /// Command for an interactive terminal (or a task) in a local PTY.
    pub fn pty_command(&self, cwd: &str, task: Option<&str>) -> portable_pty::CommandBuilder {
        let finish = "printf '\\n\\033[2m[процесс завершён с кодом %s]\\033[0m\\n' $?";
        match self {
            Host::Local => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                let mut command = match task {
                    None => portable_pty::CommandBuilder::new(&shell),
                    Some(script) => {
                        let mut command = portable_pty::CommandBuilder::new("sh");
                        command.arg("-c");
                        command.arg(format!("{script}\n{finish}\nexec {}", shell_quote(&shell)));
                        command
                    }
                };
                command.env("PATH", local_path());
                command.env("TERM", "xterm-256color");
                command.env("COLORTERM", "truecolor");
                if Path::new(cwd).is_dir() {
                    command.cwd(cwd);
                }
                command
            }
            Host::Remote(remote) => {
                let body = match task {
                    None => "exec \"${SHELL:-/bin/sh}\" -l".to_string(),
                    Some(script) => format!("{script}\n{finish}\nexec \"${{SHELL:-/bin/sh}}\" -l"),
                };
                let args = remote.ssh_args(&remote_script(cwd, &body), true);
                let mut command = portable_pty::CommandBuilder::new("ssh");
                command.args(args);
                command.env("TERM", "xterm-256color");
                command
            }
        }
    }
}

pub fn run_with_input(command: &mut Command, stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    command
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    if let Some(data) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

/// Wraps a script so it runs in `cwd` with a developer-friendly PATH under
/// `sh`, whatever the remote login shell is.
pub fn remote_script(cwd: &str, body: &str) -> String {
    let extra: Vec<String> = EXTRA_PATH.iter().map(|dir| format!("$HOME/{dir}")).collect();
    let inner = format!(
        "[ -r \"$HOME/.profile\" ] && . \"$HOME/.profile\" >/dev/null 2>&1 </dev/null\n\
         export PATH=\"{}:/usr/local/go/bin:$PATH\"\n\
         cd {} 2>/dev/null || cd\n{body}",
        extra.join(":"),
        shell_quote(cwd)
    );
    format!("exec sh -c {}", shell_quote(&inner))
}

pub fn shell_quote(value: &str) -> String {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+,".contains(&b)) {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// PATH from the user's login shell, so tools installed via shell profiles
/// (pi, cargo, go, ...) are found even when the editor starts from the
/// desktop menu. Resolved once in the background at start-up.
static SHELL_PATH: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

pub fn init_shell_path() {
    std::thread::spawn(|| {
        let _ = SHELL_PATH.get_or_init(read_shell_path);
    });
}

fn read_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", "printf '__CODIUM_PATH__%s__END__' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("CODIUM_SHELL_PROBE", "1")
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
    let start = output.find("__CODIUM_PATH__")? + "__CODIUM_PATH__".len();
    let end = output[start..].find("__END__")? + start;
    Some(output[start..end].to_string()).filter(|p| !p.is_empty())
}

pub fn local_path() -> String {
    let inherited = std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into());
    let shell = SHELL_PATH.get().cloned().flatten();
    let home = dirs::home_dir().unwrap_or_default();
    let mut parts: Vec<String> = Vec::new();
    let mut push = |dir: String| {
        if !dir.is_empty() && !parts.contains(&dir) {
            parts.push(dir);
        }
    };
    for dir in shell.iter().flat_map(|p| p.split(':')) {
        push(dir.to_string());
    }
    for dir in EXTRA_PATH {
        push(home.join(dir).to_string_lossy().into_owned());
    }
    for dir in inherited.split(':') {
        push(dir.to_string());
    }
    parts.join(":")
}

fn which_local(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).exists();
    }
    local_path().split(':').any(|dir| {
        let candidate = Path::new(dir).join(program);
        candidate.is_file()
    })
}

/// Joins a child name onto a host path (always `/`-separated on Linux).
pub fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('/') { format!("{dir}{name}") } else { format!("{dir}/{name}") }
}

pub fn parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) => "/".into(),
        Some((parent, _)) => parent.into(),
        None => ".".into(),
    }
}

pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn use_fake_ssh() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fake-ssh-dir");
        let path = std::env::var("PATH").unwrap_or_default();
        if !path.starts_with(dir) {
            // SAFETY: tests touching PATH only add the fake ssh directory.
            unsafe { std::env::set_var("PATH", format!("{dir}:{path}")) };
        }
    }

    fn exercise(host: &Host, dir: &str) {
        let file = join(dir, "hello.txt");
        host.create_dir(&join(dir, "sub")).unwrap();
        host.write_file(&file, b"hello\nworld\n").unwrap();
        assert_eq!(host.read_file(&file).unwrap(), b"hello\nworld\n");
        let entries = host.read_dir(dir).unwrap();
        assert_eq!(entries.iter().map(|e| (e.name.as_str(), e.is_dir)).collect::<Vec<_>>(), vec![("sub", true), ("hello.txt", false)]);
        assert!(!host.stat(&file).unwrap().is_dir);
        let query = codium_shared::fsops::SearchQuery { pattern: "wor".into(), max_results: 10, ..Default::default() };
        let (matches, _) = host.search(dir, &query).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!((matches[0].line, matches[0].start, matches[0].end), (1, 0, 3));
        assert!(host.list_files(dir, 100).unwrap().contains(&"hello.txt".to_string()));
        let renamed = join(dir, "renamed.txt");
        host.rename(&file, &renamed).unwrap();
        host.remove(&renamed).unwrap();
        assert!(host.stat(&renamed).is_err());
        let out = host.output(dir, "echo", &["a b".to_string()], None).unwrap();
        assert_eq!(String::from_utf8_lossy(&out).trim(), "a b");
    }

    #[test]
    fn remote_over_fake_ssh() {
        use_fake_ssh();
        let base = std::env::temp_dir().join(format!("codium-remote-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        for (use_agent, name) in [(true, "agent"), (false, "shell")] {
            let dir = base.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            let target = format!("fake-{name}-{}", std::process::id());
            // Master "connection" in the background, as the UI does in a PTY.
            let mut master = std::process::Command::new("ssh").args(ssh::master_args(&target)).stdout(Stdio::null()).spawn().unwrap();
            let remote = ssh::wait_and_establish(target, ssh::alive_flag(), use_agent).unwrap();
            assert_eq!(remote.has_agent(), use_agent, "{}", remote.agent_status.lock().unwrap());
            assert_eq!(remote.arch, std::env::consts::ARCH);
            exercise(&Host::Remote(remote.clone()), &dir.to_string_lossy());
            remote.disconnect();
            let _ = master.kill();
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn local_host() {
        let dir = std::env::temp_dir().join(format!("codium-local-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        exercise(&Host::Local, &dir.to_string_lossy());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
