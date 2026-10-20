//! Uploads (when needed) and talks to `codium-agent` on the remote host.

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

use codium_shared::proto::{self, Message, Op, Reply, Request};
use crossbeam_channel::{Sender, bounded};

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
        self.pending.lock().unwrap().insert(id, tx);
        {
            let mut writer = self.writer.lock().unwrap();
            if let Err(error) = proto::write_frame(&mut *writer, &Request { id, op }, payload) {
                self.pending.lock().unwrap().remove(&id);
                self.alive.store(false, Ordering::Relaxed);
                return Err(error.to_string());
            }
        }
        match rx.recv_timeout(Duration::from_secs(180)) {
            Ok((result, data)) => result.map(|reply| (reply, data)),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("agent did not answer".into())
            }
        }
    }

    pub fn shutdown(&self) {
        self.alive.store(false, Ordering::Relaxed);
        let _ = self.child.lock().unwrap().kill();
    }
}

impl Drop for AgentClient {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Finds an agent binary built for `arch` next to the editor.
fn local_agent_binary(arch: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?.to_path_buf();
    let name = format!("codium-agent-{arch}");
    let mut candidates = vec![
        exe_dir.join("agents").join(&name),
        exe_dir.join(&name),
        exe_dir.join("../agents").join(&name),
        exe_dir.join("../../agents").join(&name),
    ];
    if let Some(data) = dirs::data_dir() {
        candidates.push(data.join("codium-rs/agents").join(&name));
    }
    candidates.push(PathBuf::from("/usr/lib/codium-rs/agents").join(&name));
    // A native build may work if the remote has the same arch and libc.
    if arch == std::env::consts::ARCH {
        candidates.push(exe_dir.join("codium-agent"));
        // `cargo test` binaries live in target/<profile>/deps.
        candidates.push(exe_dir.join("../codium-agent"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub fn install_and_start(remote: &Arc<Remote>) -> Result<AgentClient, String> {
    let binary = local_agent_binary(&remote.arch)
        .ok_or_else(|| format!("нет сборки агента для {}", remote.arch))?;
    let data = std::fs::read(&binary).map_err(|e| e.to_string())?;
    let hash = data.iter().fold(0xcbf29ce484222325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3));
    let remote_path = format!("{}/.cache/codium-rs/agent-{hash:016x}", remote.home);
    let quoted = shell_quote(&remote_path);

    let check = format!("[ -x {quoted} ] && {quoted} --version");
    if run_with_input(&mut remote.ssh_command(&check, false), None).is_err() {
        let upload = format!(
            "mkdir -p \"$HOME/.cache/codium-rs\" && cat > {quoted}.tmp && chmod +x {quoted}.tmp && mv -f {quoted}.tmp {quoted} && {quoted} --version"
        );
        run_with_input(&mut remote.ssh_command(&upload, false), Some(&data))
            .map_err(|e| format!("не удалось запустить агент: {e}"))?;
        // Remove agents left over from older builds.
        let cleanup = format!(
            "for f in \"$HOME\"/.cache/codium-rs/agent-*; do [ \"$f\" = {quoted} ] || rm -f \"$f\"; done"
        );
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
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
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
                        if let Some(tx) = pending.lock().unwrap().remove(&id) {
                            let _ = tx.send((result, payload));
                        }
                    }
                    Message::FsChanged { paths } => {
                        if let Some(sink) = events.lock().unwrap().as_ref() {
                            let _ = sink.send(paths);
                        }
                    }
                }
            }
            alive.store(false, Ordering::Relaxed);
            for (_, tx) in pending.lock().unwrap().drain() {
                let _ = tx.send((Err("agent exited".into()), Vec::new()));
            }
        });
    }

    let client = AgentClient {
        writer: Mutex::new(BufWriter::new(stdin)),
        pending,
        next_id: AtomicU64::new(1),
        alive,
        child: Mutex::new(child),
    };
    match client.call(Op::Hello, &[]) {
        Ok((Reply::Hello { .. }, _)) => Ok(client),
        Ok(_) => Err("агент ответил неожиданно".into()),
        Err(error) => Err(error),
    }
}
