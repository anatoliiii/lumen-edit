//! Uploads (when needed) and talks to `lumen-agent` on the remote host.

use std::{
    collections::HashMap,
    io::{BufReader, BufWriter},
    path::PathBuf,
    process::{Child, ChildStdin, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::{Sender, bounded};
use lumen_shared::proto::{self, Message, Op, Reply, Request};

use super::{Remote, run_with_input, shell_quote};

type Pending = Arc<Mutex<HashMap<u64, Sender<(Result<Reply, String>, Vec<u8>)>>>>;

pub struct AgentClient {
    writer: Mutex<BufWriter<ChildStdin>>,
    pending: Pending,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    child: Mutex<Child>,
}

impl AgentClient {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }

    pub fn call(&self, op: Op, payload: &[u8]) -> Result<(Reply, Vec<u8>), String> {
        if !self.is_alive() {
            return Err("agent is not running".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = bounded(1);
        // Poison recovery is safe here: if a worker panicked, `pending` may carry a
        // stale entry at worst, and the `recv_timeout` below turns that into a
        // recoverable "agent did not answer" error; writing to `writer` returns an
        // error which marks the agent dead instead of corrupting state.
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        {
            let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(error) = proto::write_frame(&mut *writer, &Request { id, op }, payload) {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                self.alive.store(false, Ordering::Relaxed);
                return Err(error.to_string());
            }
        }
        match rx.recv_timeout(Duration::from_secs(180)) {
            Ok((result, data)) => result.map(|reply| (reply, data)),
            Err(_) => {
                self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
                Err("agent did not answer".into())
            }
        }
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    }
}

impl Drop for AgentClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Agent file name for a platform: `lumen-agent-<os>-<arch>[.exe]`.
pub fn agent_name(os: &str, arch: &str) -> String {
    let exe = if os == "windows" { ".exe" } else { "" };
    format!("lumen-agent-{os}-{arch}{exe}")
}

/// Finds an agent binary built for the remote platform next to the editor.
fn local_agent_binary(os: &str, arch: &str) -> Option<PathBuf> {
    // Resolved, so a symlink in ~/.local/bin still finds the app bundle.
    let exe = std::env::current_exe().ok()?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let exe_dir = exe.parent()?.to_path_buf();
    let name = agent_name(os, arch);
    let mut candidates = vec![
        exe_dir.join("agents").join(&name),
        exe_dir.join(&name),
        exe_dir.join("../agents").join(&name),
        exe_dir.join("../../agents").join(&name),
        // Inside a macOS app bundle.
        exe_dir.join("../Resources/agents").join(&name),
    ];
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("lumen-edit/agents").join(&name));
    }
    candidates.push(PathBuf::from("/usr/lib/lumen-edit/agents").join(&name));
    // A native build works on the same OS and arch (and libc on Linux).
    if os == std::env::consts::OS && arch == std::env::consts::ARCH {
        let native = format!("lumen-agent{}", std::env::consts::EXE_SUFFIX);
        candidates.push(exe_dir.join(&native));
        // `cargo test` binaries live in target/<profile>/deps.
        candidates.push(exe_dir.join("..").join(&native));
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub fn install_and_start(remote: &Arc<Remote>) -> Result<AgentClient, String> {
    let binary = local_agent_binary(&remote.os, &remote.arch).ok_or_else(|| t!("no agent build for {n1}", n1 = remote.platform()))?;
    let data = std::fs::read(&binary).map_err(|e| e.to_string())?;
    let hash = data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    let exe = if remote.os == "windows" { ".exe" } else { "" };
    let remote_path = format!("{}/.cache/lumen-edit/agent-{hash:016x}{exe}", remote.home);
    let quoted = shell_quote(&remote_path);

    let check = format!("[ -x {quoted} ] && {quoted} --version");
    if run_with_input(&mut remote.ssh_command(&check, false), None).is_err() {
        let upload = format!(
            "mkdir -p \"$HOME/.cache/lumen-edit\" && cat > {quoted}.tmp && chmod +x {quoted}.tmp && mv -f {quoted}.tmp {quoted} && {quoted} --version"
        );
        run_with_input(&mut remote.ssh_command(&upload, false), Some(&data)).map_err(|e| t!("could not start the agent: {e}", e = e))?;
        // Remove agents left over from older builds.
        let cleanup = format!("for f in \"$HOME\"/.cache/lumen-edit/agent-*; do [ \"$f\" = {quoted} ] || rm -f \"$f\"; done");
        let _ = run_with_input(&mut remote.ssh_command(&cleanup, false), None);
    }
    spawn(remote, &remote_path)
}

fn spawn(remote: &Arc<Remote>, path: &str) -> Result<AgentClient, String> {
    let mut child = remote
        .ssh_command(&format!("exec {}", shell_quote(path)), false)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdin = child.stdin.take().ok_or_else(|| "agent stdin unexpectedly missing".to_string())?;
    let stdout = child.stdout.take().ok_or_else(|| "agent stdout unexpectedly missing".to_string())?;
    let pending: Pending = Arc::default();
    let alive = Arc::new(AtomicBool::new(true));

    {
        let pending = pending.clone();
        let alive = alive.clone();
        let events = remote.events.clone();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Ok((message, payload)) = proto::read_frame::<_, Message>(&mut reader) {
                match message {
                    Message::Response { id, result } => {
                        if let Some(tx) = pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
                            let _ = tx.send((result, payload));
                        }
                    }
                    Message::FsChanged { paths } => {
                        if let Some(sink) = events.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                            let _ = sink.send(paths);
                        }
                    }
                }
            }
            alive.store(false, Ordering::Relaxed);
            for (_, tx) in pending.lock().unwrap_or_else(|e| e.into_inner()).drain() {
                let _ = tx.send((Err("agent exited".into()), Vec::new()));
            }
        });
    }

    let client =
        AgentClient { writer: Mutex::new(BufWriter::new(stdin)), pending, next_id: AtomicU64::new(1), alive, child: Mutex::new(child) };
    match client.call(Op::Hello, &[]) {
        Ok((Reply::Hello { .. }, _)) => Ok(client),
        Ok(_) => Err(t!("unexpected answer from the agent").into()),
        Err(error) => Err(error),
    }
}
