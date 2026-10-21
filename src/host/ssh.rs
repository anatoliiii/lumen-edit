//! SSH hosts through the system OpenSSH client. One master connection
//! (ControlMaster) is started in a terminal so password / 2FA prompts work,
//! and every later command reuses it without authenticating again.

use std::{
    path::PathBuf,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use lumen_shared::{
    fsops::{DirEntry, SearchMatch, SearchQuery, Stat},
    proto::{Op, Reply},
};

use super::{agent::AgentClient, shell};

pub struct Remote {
    pub target: String,
    pub control_path: PathBuf,
    pub home: String,
    /// `linux`, `macos` or `windows` (Rust's names), from `uname -s`.
    pub os: String,
    /// `x86_64` or `aarch64` (Rust's names), from `uname -m`.
    pub arch: String,
    agent: Mutex<Option<Arc<AgentClient>>>,
    pub agent_status: Mutex<String>,
    pub events: Arc<Mutex<Option<Sender<Vec<String>>>>>,
    /// Whether the master connection answered the last health check.
    pub connected: AtomicBool,
    /// Set once the agent ran, so a dead agent is brought back.
    agent_wanted: AtomicBool,
    watch_root: Mutex<Option<String>>,
}

pub fn control_path(target: &str) -> PathBuf {
    let base = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir).join("lumen-edit");
    let _ = std::fs::create_dir_all(&base);
    let hash = target.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    base.join(format!("cm-{hash:016x}"))
}

/// Arguments for the master connection; run inside a PTY so the user can
/// answer prompts.
pub fn master_args(target: &str) -> Vec<String> {
    let control = control_path(target);
    let _ = std::fs::remove_file(&control);
    vec![
        "-M".into(),
        "-S".into(),
        control.to_string_lossy().into_owned(),
        "-o".into(),
        "ControlPersist=no".into(),
        "-o".into(),
        "ServerAliveInterval=20".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-N".into(),
        target.into(),
    ]
}

pub fn master_ready(target: &str) -> bool {
    Command::new("ssh")
        .args(["-S", &control_path(target).to_string_lossy(), "-O", "check", target])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Waits for the master (running in a terminal) to come up, then probes the
/// host and tries to start the agent.
pub fn wait_and_establish(target: String, master_alive: Arc<AtomicBool>, use_agent: bool) -> Result<Arc<Remote>, String> {
    let started = Instant::now();
    loop {
        if master_ready(&target) {
            break;
        }
        if !master_alive.load(Ordering::Relaxed) {
            return Err(t!("The SSH connection closed. Details are in the connection window.").into());
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err(t!("Timed out waiting for the connection").into());
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let mut remote = Remote {
        control_path: control_path(&target),
        target,
        home: String::new(),
        os: String::new(),
        arch: String::new(),
        agent: Mutex::new(None),
        agent_status: Mutex::new(t!("agent not used").into()),
        events: Arc::new(Mutex::new(None)),
        connected: AtomicBool::new(true),
        agent_wanted: AtomicBool::new(false),
        watch_root: Mutex::new(None),
    };

    // On Windows the SSH shell must be a POSIX one (Git Bash); its home is
    // turned into C:/... so native programs understand the paths.
    let probe = super::run_with_input(
        &mut remote.ssh_command("uname -s; uname -m; cygpath -m \"$HOME\" 2>/dev/null || printf '%s\\n' \"$HOME\"", false),
        None,
    )
    .map_err(|e| t!("the host did not answer to uname (on Windows set Git Bash as the OpenSSH shell): {e}", e = e))?;
    let probe = String::from_utf8_lossy(&probe);
    let mut lines = probe.lines();
    let (os, arch) = platform(lines.next().unwrap_or(""), lines.next().unwrap_or(""));
    remote.os = os;
    remote.arch = arch;
    remote.home = lines.next().unwrap_or("/").trim().to_string();

    let remote = Arc::new(remote);
    if use_agent {
        let _ = remote.restart_agent();
    }
    spawn_health_check(Arc::downgrade(&remote));
    Ok(remote)
}

/// Rust's OS and arch names for `uname -s` and `uname -m`.
pub fn platform(system: &str, machine: &str) -> (String, String) {
    let system = system.trim();
    let os = match system {
        "Linux" => "linux",
        "Darwin" => "macos",
        "FreeBSD" => "freebsd",
        _ if ["MINGW", "MSYS", "CYGWIN"].iter().any(|p| system.starts_with(p)) => "windows",
        _ => system,
    };
    let arch = match machine.trim() {
        "arm64" | "aarch64" | "armv8l" => "aarch64",
        "amd64" | "x86_64" | "x64" => "x86_64",
        other => other,
    };
    (os.to_lowercase(), arch.to_string())
}

impl Remote {
    /// `<os>-<arch>`, as in agent file names.
    pub fn platform(&self) -> String {
        format!("{}-{}", self.os, self.arch)
    }
}

/// Every few seconds: notes whether the connection is up and brings back an
/// agent that died (dropped link, killed process) once the host answers.
fn spawn_health_check(remote: std::sync::Weak<Remote>) {
    std::thread::spawn(move || {
        let mut last_restart: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(5));
            let Some(remote) = remote.upgrade() else { break };
            let up = master_ready(&remote.target);
            remote.connected.store(up, Ordering::Relaxed);
            if up
                && remote.agent_wanted.load(Ordering::Relaxed)
                && remote.agent().is_none()
                && last_restart.is_none_or(|t| t.elapsed() > Duration::from_secs(30))
            {
                last_restart = Some(Instant::now());
                let _ = remote.restart_agent();
            }
        }
    });
}

impl Remote {
    pub fn ssh_args(&self, script: &str, tty: bool) -> Vec<String> {
        vec![
            "-S".into(),
            self.control_path.to_string_lossy().into_owned(),
            "-o".into(),
            "ControlMaster=no".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "LogLevel=error".into(),
            if tty { "-tt".into() } else { "-T".into() },
            self.target.clone(),
            script.into(),
        ]
    }

    pub fn ssh_command(&self, script: &str, tty: bool) -> Command {
        let mut command = Command::new("ssh");
        command.args(self.ssh_args(script, tty));
        command
    }

    /// (Re)starts the agent on the host and resumes watching.
    pub fn restart_agent(self: &Arc<Self>) -> Result<(), String> {
        if let Some(old) = self.agent.lock().unwrap().take() {
            old.shutdown();
        }
        match super::agent::install_and_start(self) {
            Ok(client) => {
                *self.agent.lock().unwrap() = Some(Arc::new(client));
                *self.agent_status.lock().unwrap() = t!("agent running").into();
                self.agent_wanted.store(true, Ordering::Relaxed);
                let root = self.watch_root.lock().unwrap().clone();
                if let Some(root) = root {
                    let _ = self.call(Op::Watch { root }, &[]);
                }
                Ok(())
            }
            Err(error) => {
                *self.agent_status.lock().unwrap() = t!("no agent: {error}", error = error);
                Err(error)
            }
        }
    }

    #[cfg_attr(not(all(test, unix)), allow(dead_code))]
    pub fn has_agent(&self) -> bool {
        self.agent().is_some()
    }

    fn agent(&self) -> Option<Arc<AgentClient>> {
        let mut guard = self.agent.lock().unwrap();
        if guard.as_ref().is_some_and(|agent| !agent.is_alive()) {
            *guard = None;
            *self.agent_status.lock().unwrap() = t!("agent stopped, using the shell; will restart it when the host answers").into();
        }
        guard.clone()
    }

    fn call(&self, op: Op, payload: &[u8]) -> Option<Result<(Reply, Vec<u8>), String>> {
        let agent = self.agent()?;
        match agent.call(op, payload) {
            Err(error) if !agent.is_alive() => {
                log_error(&error);
                None
            }
            other => Some(other),
        }
    }

    pub fn stat(&self, path: &str) -> Result<Stat, String> {
        match self.call(Op::Stat { path: path.into() }, &[]) {
            Some(Ok((Reply::Stat(stat), _))) => Ok(stat),
            Some(Ok(_)) => Err("unexpected reply".into()),
            Some(Err(error)) => Err(error),
            None => shell::stat(self, path),
        }
    }

    pub fn read_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        match self.call(Op::ReadDir { path: path.into() }, &[]) {
            Some(Ok((Reply::Entries(entries), _))) => Ok(entries),
            Some(Ok(_)) => Err("unexpected reply".into()),
            Some(Err(error)) => Err(error),
            None => shell::read_dir(self, path),
        }
    }

    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        match self.call(Op::ReadFile { path: path.into() }, &[]) {
            Some(Ok((_, data))) => Ok(data),
            Some(Err(error)) => Err(error),
            None => shell::read_file(self, path),
        }
    }

    pub fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        match self.call(Op::WriteFile { path: path.into() }, data) {
            Some(result) => result.map(|_| ()),
            None => shell::write_file(self, path, data),
        }
    }

    pub fn create_dir(&self, path: &str) -> Result<(), String> {
        match self.call(Op::CreateDir { path: path.into() }, &[]) {
            Some(result) => result.map(|_| ()),
            None => shell::simple(self, &format!("mkdir -p -- {}", super::shell_quote(path))),
        }
    }

    pub fn remove(&self, path: &str) -> Result<(), String> {
        match self.call(Op::Remove { path: path.into() }, &[]) {
            Some(result) => result.map(|_| ()),
            None => shell::simple(self, &format!("rm -rf -- {}", super::shell_quote(path))),
        }
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), String> {
        match self.call(Op::Rename { from: from.into(), to: to.into() }, &[]) {
            Some(result) => result.map(|_| ()),
            None => shell::simple(self, &format!("mv -- {} {}", super::shell_quote(from), super::shell_quote(to))),
        }
    }

    pub fn list_files(&self, root: &str, max: usize) -> Result<Vec<String>, String> {
        match self.call(Op::ListFiles { root: root.into(), max }, &[]) {
            Some(Ok((Reply::Files(files), _))) => Ok(files),
            Some(Ok(_)) => Err("unexpected reply".into()),
            Some(Err(error)) => Err(error),
            None => shell::list_files(self, root, max),
        }
    }

    pub fn search(&self, root: &str, query: &SearchQuery) -> Result<(Vec<SearchMatch>, bool), String> {
        match self.call(Op::Search { root: root.into(), query: query.clone() }, &[]) {
            Some(Ok((Reply::Matches { matches, truncated }, _))) => Ok((matches, truncated)),
            Some(Ok(_)) => Err("unexpected reply".into()),
            Some(Err(error)) => Err(error),
            None => shell::search(self, root, query),
        }
    }

    /// Starts watching `root` for changes; only possible with the agent.
    pub fn watch(&self, root: &str, sink: Sender<Vec<String>>) -> bool {
        *self.events.lock().unwrap() = Some(sink);
        *self.watch_root.lock().unwrap() = Some(root.to_string());
        matches!(self.call(Op::Watch { root: root.into() }, &[]), Some(Ok(_)))
    }

    pub fn disconnect(&self) {
        if let Some(agent) = self.agent.lock().unwrap().take() {
            agent.shutdown();
        }
        let _ = Command::new("ssh")
            .args(["-S", &self.control_path.to_string_lossy(), "-O", "exit", &self.target])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

fn log_error(error: &str) {
    eprintln!("lumen-edit: agent error: {error}");
}

/// Shared flag helper for callers that track a master terminal.
#[cfg_attr(not(all(test, unix)), allow(dead_code))]
pub fn alive_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_platforms() {
        let p = |s, m| platform(s, m);
        assert_eq!(p("Linux\n", "x86_64\n"), ("linux".into(), "x86_64".into()));
        assert_eq!(p("Darwin", "arm64"), ("macos".into(), "aarch64".into()));
        assert_eq!(p("MINGW64_NT-10.0-26100", "x86_64"), ("windows".into(), "x86_64".into()));
        assert_eq!(super::super::agent::agent_name("windows", "x86_64"), "lumen-agent-windows-x86_64.exe");
        assert_eq!(super::super::agent::agent_name("macos", "aarch64"), "lumen-agent-macos-aarch64");
    }
}
