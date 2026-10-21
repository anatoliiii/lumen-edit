//! Git through the `git` command line, which works the same locally and
//! over SSH. All functions block; call them from worker threads.

use std::process::Stdio;

use crate::host::{Host, join};

fn git_raw(host: &Host, cwd: &str, args: &[&str], stdin: Option<&[u8]>, ok_codes: &[i32]) -> Result<String, String> {
    use std::io::Write;
    let mut full: Vec<String> = vec!["-c".into(), "color.ui=never".into(), "-c".into(), "core.quotepath=off".into()];
    full.extend(args.iter().map(|s| s.to_string()));
    let mut command = host.command(cwd, "git", &full);
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C.UTF-8")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|e| format!("git: {e}"))?;
    if let Some(data) = stdin {
        let mut pipe = child.stdin.take().unwrap();
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = pipe.write_all(&data);
        });
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let code = output.status.code().unwrap_or(-1);
    if output.status.success() || ok_codes.contains(&code) {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

pub fn git(host: &Host, cwd: &str, args: &[&str]) -> Result<String, String> {
    git_raw(host, cwd, args, None, &[])
}

/// Runs git and returns combined output text for display, even on success.
pub fn git_verbose(host: &Host, cwd: &str, args: &[&str]) -> Result<String, String> {
    let mut full: Vec<String> = vec!["-c".into(), "color.ui=never".into()];
    full.extend(args.iter().map(|s| s.to_string()));
    let output =
        host.command(cwd, "git", &full).env("GIT_TERMINAL_PROMPT", "0").stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)).trim().to_string();
    if output.status.success() { Ok(text) } else { Err(text) }
}

/// Repository top level containing `path`, if any.
pub fn repo_root(host: &Host, path: &str) -> Option<String> {
    git(host, path, &["rev-parse", "--show-toplevel"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RepoState {
    #[default]
    Clean,
    Merging,
    Rebasing,
    CherryPicking,
    Reverting,
}

#[derive(Clone, Debug)]
pub struct FileStatus {
    /// Relative to the repository root.
    pub path: String,
    /// Source path of a rename.
    #[allow(dead_code)]
    pub orig_path: Option<String>,
    /// Porcelain v2 XY codes; '.' means unchanged.
    pub index: char,
    pub worktree: char,
    pub conflict: bool,
    pub untracked: bool,
}

impl FileStatus {
    pub fn staged(&self) -> bool {
        !self.untracked && !self.conflict && self.index != '.'
    }
    pub fn unstaged(&self) -> bool {
        self.untracked || (!self.conflict && self.worktree != '.')
    }
    /// Letter shown in lists, VS Code style.
    pub fn letter(&self, staged: bool) -> char {
        if self.conflict {
            return '!';
        }
        if self.untracked {
            return 'U';
        }
        let code = if staged { self.index } else { self.worktree };
        match code {
            'M' | 'T' => 'M',
            'A' => 'A',
            'D' => 'D',
            'R' => 'R',
            'C' => 'C',
            other => other,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub root: String,
    pub branch: String,
    pub detached: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
    pub state: RepoState,
}

pub fn status(host: &Host, root: &str) -> Result<Status, String> {
    let out = git(host, root, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"])?;
    let mut status = Status { root: root.to_string(), ..Default::default() };
    let mut records = out.split('\0').peekable();
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        if let Some(header) = record.strip_prefix("# ") {
            if let Some(head) = header.strip_prefix("branch.head ") {
                status.detached = head == "(detached)";
                status.branch = head.to_string();
            } else if let Some(upstream) = header.strip_prefix("branch.upstream ") {
                status.upstream = Some(upstream.to_string());
            } else if let Some(ab) = header.strip_prefix("branch.ab ") {
                let mut parts = ab.split(' ');
                status.ahead = parts.next().and_then(|a| a.trim_start_matches('+').parse().ok()).unwrap_or(0);
                status.behind = parts.next().and_then(|b| b.trim_start_matches('-').parse().ok()).unwrap_or(0);
            }
            continue;
        }
        let kind = record.chars().next().unwrap_or(' ');
        match kind {
            '1' | '2' | 'u' => {
                let fields: usize = match kind {
                    '1' => 8,
                    '2' => 9,
                    _ => 10,
                };
                let parts: Vec<&str> = record.splitn(fields + 1, ' ').collect();
                let xy: Vec<char> = parts.get(1).unwrap_or(&"..").chars().collect();
                let path = parts.get(fields).unwrap_or(&"").to_string();
                let orig_path = if kind == '2' { records.next().map(str::to_string) } else { None };
                status.files.push(FileStatus {
                    path,
                    orig_path,
                    index: xy.first().copied().unwrap_or('.'),
                    worktree: xy.get(1).copied().unwrap_or('.'),
                    conflict: kind == 'u',
                    untracked: false,
                });
            }
            '?' => status.files.push(FileStatus {
                path: record[2..].to_string(),
                orig_path: None,
                index: '.',
                worktree: '?',
                conflict: false,
                untracked: true,
            }),
            _ => {}
        }
    }
    if let Ok(git_dir) = git(host, root, &["rev-parse", "--absolute-git-dir"]) {
        let git_dir = git_dir.trim();
        let exists = |name: &str| host.stat(&join(git_dir, name)).is_ok();
        status.state = if exists("rebase-merge") || exists("rebase-apply") {
            RepoState::Rebasing
        } else if exists("MERGE_HEAD") {
            RepoState::Merging
        } else if exists("CHERRY_PICK_HEAD") {
            RepoState::CherryPicking
        } else if exists("REVERT_HEAD") {
            RepoState::Reverting
        } else {
            RepoState::Clean
        };
    }
    Ok(status)
}

fn with_paths<'a>(mut args: Vec<&'a str>, paths: &'a [String]) -> Vec<&'a str> {
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    args
}

pub fn stage(host: &Host, root: &str, paths: &[String]) -> Result<(), String> {
    git(host, root, &with_paths(vec!["add", "-A"], paths)).map(|_| ())
}

pub fn stage_all(host: &Host, root: &str) -> Result<(), String> {
    git(host, root, &["add", "-A"]).map(|_| ())
}

pub fn unstage(host: &Host, root: &str, paths: &[String]) -> Result<(), String> {
    // `restore --staged` needs a HEAD commit; fall back for fresh repos.
    git(host, root, &with_paths(vec!["restore", "--staged"], paths))
        .or_else(|_| git(host, root, &with_paths(vec!["rm", "--cached", "-r", "-q"], paths)))
        .map(|_| ())
}

pub fn unstage_all(host: &Host, root: &str) -> Result<(), String> {
    git(host, root, &["reset", "-q"]).map(|_| ())
}

pub fn discard(host: &Host, root: &str, files: &[FileStatus]) -> Result<(), String> {
    let tracked: Vec<String> = files.iter().filter(|f| !f.untracked).map(|f| f.path.clone()).collect();
    let untracked: Vec<String> = files.iter().filter(|f| f.untracked).map(|f| f.path.clone()).collect();
    if !tracked.is_empty() {
        git(host, root, &with_paths(vec!["restore", "--worktree"], &tracked))?;
    }
    if !untracked.is_empty() {
        git(host, root, &with_paths(vec!["clean", "-f", "-q"], &untracked))?;
    }
    Ok(())
}

pub fn commit(host: &Host, root: &str, message: &str, amend: bool, all: bool) -> Result<String, String> {
    let mut args = vec!["commit", "-F", "-"];
    if amend {
        args.push("--amend");
    }
    if all {
        args.push("-a");
    }
    git_raw(host, root, &args, Some(message.as_bytes()), &[]).map(|s| s.lines().next().unwrap_or("").to_string())
}

pub fn last_commit_message(host: &Host, root: &str) -> Result<String, String> {
    git(host, root, &["log", "-1", "--format=%B"]).map(|s| s.trim_end().to_string())
}

/// `set_upstream`: remote to publish the current branch to (`push -u`).
pub fn push(host: &Host, root: &str, set_upstream: Option<&str>, force: bool) -> Result<String, String> {
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if let Some(remote) = set_upstream {
        args.extend(["-u", remote, "HEAD"]);
    }
    git_verbose(host, root, &args)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remote {
    pub name: String,
    pub url: String,
}

pub fn remotes(host: &Host, root: &str) -> Result<Vec<Remote>, String> {
    let out = git(host, root, &["remote", "-v"])?;
    let mut remotes: Vec<Remote> = Vec::new();
    for line in out.lines().filter(|l| l.ends_with("(fetch)")) {
        let mut parts = line.split_whitespace();
        if let (Some(name), Some(url)) = (parts.next(), parts.next())
            && !remotes.iter().any(|r| r.name == name)
        {
            remotes.push(Remote { name: name.into(), url: url.into() });
        }
    }
    Ok(remotes)
}

pub fn add_remote(host: &Host, root: &str, name: &str, url: &str) -> Result<String, String> {
    git_verbose(host, root, &["remote", "add", name, url])
}

pub fn set_remote_url(host: &Host, root: &str, name: &str, url: &str) -> Result<String, String> {
    git_verbose(host, root, &["remote", "set-url", name, url])
}

pub fn remove_remote(host: &Host, root: &str, name: &str) -> Result<String, String> {
    git_verbose(host, root, &["remote", "remove", name])
}

pub fn pull(host: &Host, root: &str, rebase: bool) -> Result<String, String> {
    git_verbose(host, root, if rebase { &["pull", "--rebase"] } else { &["pull", "--no-rebase"] })
}

pub fn fetch(host: &Host, root: &str) -> Result<String, String> {
    git_verbose(host, root, &["fetch", "--all", "--prune"])
}

#[derive(Clone, Debug, Default)]
pub struct Branch {
    pub name: String,
    pub remote: bool,
    pub current: bool,
    #[allow(dead_code)]
    pub upstream: String,
    pub commit: String,
    pub date: String,
    pub subject: String,
    /// Root of the worktree that has this branch checked out, if any.
    pub worktree: String,
}

pub fn branches(host: &Host, root: &str) -> Result<Vec<Branch>, String> {
    let out = git(
        host,
        root,
        &[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(refname)%00%(refname:short)%00%(HEAD)%00%(upstream:short)%00%(objectname:short)%00%(committerdate:relative)%00%(subject)%00%(worktreepath)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\0').collect();
            if parts.len() < 8 || parts[0].ends_with("/HEAD") {
                return None;
            }
            Some(Branch {
                remote: parts[0].starts_with("refs/remotes/"),
                name: parts[1].to_string(),
                current: parts[2] == "*",
                upstream: parts[3].to_string(),
                commit: parts[4].to_string(),
                date: parts[5].to_string(),
                subject: parts[6].to_string(),
                worktree: parts[7].to_string(),
            })
        })
        .collect())
}

pub fn checkout(host: &Host, root: &str, branch: &Branch) -> Result<String, String> {
    if branch.remote {
        let local = branch.name.split_once('/').map(|(_, rest)| rest).unwrap_or(&branch.name);
        git_verbose(host, root, &["switch", "--track", "-c", local, &branch.name]).or_else(|_| git_verbose(host, root, &["switch", local]))
    } else {
        git_verbose(host, root, &["switch", &branch.name])
    }
}

pub fn create_branch(host: &Host, root: &str, name: &str, checkout: bool) -> Result<String, String> {
    if checkout { git_verbose(host, root, &["switch", "-c", name]) } else { git_verbose(host, root, &["branch", name]) }
}

pub fn delete_branch(host: &Host, root: &str, name: &str, force: bool) -> Result<String, String> {
    git_verbose(host, root, &["branch", if force { "-D" } else { "-d" }, name])
}

pub fn merge(host: &Host, root: &str, name: &str) -> Result<String, String> {
    git_verbose(host, root, &["merge", "--no-edit", name])
}

pub fn rebase(host: &Host, root: &str, name: &str) -> Result<String, String> {
    git_verbose(host, root, &["rebase", name])
}

pub fn abort(host: &Host, root: &str, state: &RepoState) -> Result<String, String> {
    match state {
        RepoState::Merging => git_verbose(host, root, &["merge", "--abort"]),
        RepoState::Rebasing => git_verbose(host, root, &["rebase", "--abort"]),
        RepoState::CherryPicking => git_verbose(host, root, &["cherry-pick", "--abort"]),
        RepoState::Reverting => git_verbose(host, root, &["revert", "--abort"]),
        RepoState::Clean => Ok(String::new()),
    }
}

pub fn continue_op(host: &Host, root: &str, state: &RepoState) -> Result<String, String> {
    let mut command = match state {
        RepoState::Merging => vec!["commit", "--no-edit"],
        RepoState::Rebasing => vec!["rebase", "--continue"],
        RepoState::CherryPicking => vec!["cherry-pick", "--continue"],
        RepoState::Reverting => vec!["revert", "--continue"],
        RepoState::Clean => return Ok(String::new()),
    };
    let mut full = vec!["-c", "core.editor=true"];
    full.append(&mut command);
    git_verbose(host, root, &full)
}

pub fn take_side(host: &Host, root: &str, path: &str, ours: bool) -> Result<(), String> {
    git(host, root, &["checkout", if ours { "--ours" } else { "--theirs" }, "--", path])?;
    git(host, root, &["add", "--", path]).map(|_| ())
}

#[derive(Clone, Debug)]
pub struct StashEntry {
    pub name: String,
    pub message: String,
    pub date: String,
}

pub fn stashes(host: &Host, root: &str) -> Result<Vec<StashEntry>, String> {
    let out = git(host, root, &["stash", "list", "--format=%gd%x00%gs%x00%cr"])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\0').collect();
            (parts.len() >= 3).then(|| StashEntry { name: parts[0].into(), message: parts[1].into(), date: parts[2].into() })
        })
        .collect())
}

pub fn stash_push(host: &Host, root: &str, message: &str, include_untracked: bool) -> Result<String, String> {
    let mut args = vec!["stash", "push"];
    if include_untracked {
        args.push("-u");
    }
    if !message.is_empty() {
        args.extend(["-m", message]);
    }
    git_verbose(host, root, &args)
}

pub fn stash_action(host: &Host, root: &str, action: &str, name: &str) -> Result<String, String> {
    git_verbose(host, root, &["stash", action, name])
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    pub graph: String,
    pub hash: String,
    pub short: String,
    pub refs: String,
    pub subject: String,
    pub author: String,
    pub date: String,
}

pub fn log(host: &Host, root: &str, all: bool, limit: usize, path: Option<&str>) -> Result<Vec<LogEntry>, String> {
    let limit = format!("-n{limit}");
    let mut args = vec!["log", "--graph", "--date-order", "--format=%x1f%H%x1f%h%x1f%D%x1f%s%x1f%an%x1f%cr", &limit];
    if all {
        args.push("--all");
    }
    if let Some(path) = path {
        args.extend(["--follow", "--", path]);
    }
    let out = git(host, root, &args)?;
    Ok(out
        .lines()
        .map(|line| {
            let mut parts = line.split('\x1f');
            let graph = parts.next().unwrap_or("").to_string();
            LogEntry {
                graph,
                hash: parts.next().unwrap_or("").into(),
                short: parts.next().unwrap_or("").into(),
                refs: parts.next().unwrap_or("").into(),
                subject: parts.next().unwrap_or("").into(),
                author: parts.next().unwrap_or("").into(),
                date: parts.next().unwrap_or("").into(),
            }
        })
        .collect())
}

pub fn show_commit(host: &Host, root: &str, hash: &str) -> Result<String, String> {
    git(host, root, &["show", "--format=commit %H%nAuthor: %an <%ae>%nDate:   %ad%n%n%w(0,4,4)%B", "--stat", "--patch", hash])
}

pub enum DiffTarget {
    Unstaged,
    Staged,
    Untracked,
}

pub fn diff_file(host: &Host, root: &str, path: &str, target: DiffTarget) -> Result<String, String> {
    match target {
        DiffTarget::Unstaged => git(host, root, &["diff", "--", path]),
        DiffTarget::Staged => git(host, root, &["diff", "--cached", "--", path]),
        DiffTarget::Untracked => git_raw(host, root, &["diff", "--no-index", "--", "/dev/null", path], None, &[1]),
    }
}

/// Content of `path` at HEAD (relative to the repo root).
pub fn head_content(host: &Host, root: &str, path: &str) -> Option<String> {
    git(host, root, &["show", &format!("HEAD:{path}")]).ok()
}

pub fn apply_patch(host: &Host, root: &str, patch: &str, reverse: bool, cached: bool) -> Result<(), String> {
    let mut args = vec!["apply", "--whitespace=nowarn"];
    if cached {
        args.push("--cached");
    }
    if reverse {
        args.push("-R");
    }
    args.push("-");
    git_raw(host, root, &args, Some(patch.as_bytes()), &[]).map(|_| ())
}

#[derive(Clone, Debug)]
pub struct BlameLine {
    pub hash: String,
    pub author: String,
    pub time: i64,
    pub summary: String,
}

/// Blame for the buffer contents (so unsaved edits line up).
pub fn blame(host: &Host, root: &str, path: &str, contents: &str) -> Result<Vec<BlameLine>, String> {
    let out = git_raw(host, root, &["blame", "--porcelain", "--contents", "-", "--", path], Some(contents.as_bytes()), &[])?;
    let mut commits: std::collections::HashMap<String, BlameLine> = std::collections::HashMap::new();
    let mut lines = Vec::new();
    let mut current: Option<String> = None;
    for line in out.lines() {
        if let Some(_content) = line.strip_prefix('\t') {
            if let Some(hash) = current.take() {
                let info = commits.get(&hash).cloned().unwrap_or(BlameLine {
                    hash: hash.clone(),
                    author: String::new(),
                    time: 0,
                    summary: String::new(),
                });
                lines.push(info);
            }
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let first = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("");
        if first.len() == 40 && first.chars().all(|c| c.is_ascii_hexdigit()) {
            current = Some(first.to_string());
            commits.entry(first.to_string()).or_insert_with(|| BlameLine {
                hash: first.to_string(),
                author: String::new(),
                time: 0,
                summary: String::new(),
            });
        } else if let Some(hash) = &current
            && let Some(entry) = commits.get_mut(hash)
        {
            match first {
                "author" => entry.author = rest.to_string(),
                "author-time" => entry.time = rest.parse().unwrap_or(0),
                "summary" => entry.summary = rest.to_string(),
                _ => {}
            }
        }
    }
    Ok(lines)
}

pub fn init(host: &Host, root: &str) -> Result<String, String> {
    git_verbose(host, root, &["init"])
}

// ------------------------------------------------------------ diff parsing

#[derive(Clone, Debug)]
pub struct DiffFile {
    pub header: Vec<String>,
    pub path: String,
    pub hunks: Vec<Hunk>,
}

#[derive(Clone, Debug)]
pub struct Hunk {
    pub header: String,
    pub lines: Vec<String>,
    pub new_start: usize,
}

pub fn parse_diff(text: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    for line in text.lines() {
        if line.starts_with("diff --git") || (line.starts_with("diff ") && files.is_empty()) {
            files.push(DiffFile { header: vec![line.to_string()], path: String::new(), hunks: Vec::new() });
            continue;
        }
        let Some(file) = files.last_mut() else { continue };
        if line.starts_with("@@") {
            let new_start = line.split('+').nth(1).and_then(|s| s.split([',', ' ']).next()).and_then(|n| n.parse().ok()).unwrap_or(1);
            file.hunks.push(Hunk { header: line.to_string(), lines: Vec::new(), new_start });
        } else if let Some(hunk) = file.hunks.last_mut() {
            hunk.lines.push(line.to_string());
        } else {
            if let Some(path) = line.strip_prefix("+++ ") {
                file.path = path.strip_prefix("b/").unwrap_or(path).to_string();
            } else if let Some(path) = line.strip_prefix("--- ")
                && file.path.is_empty()
                && path != "/dev/null"
            {
                file.path = path.strip_prefix("a/").unwrap_or(path).to_string();
            }
            file.header.push(line.to_string());
        }
    }
    files
}

/// A patch containing only one hunk, for staging/unstaging it.
pub fn hunk_patch(file: &DiffFile, hunk: &Hunk) -> String {
    let mut patch = file.header.join("\n");
    patch.push('\n');
    patch.push_str(&hunk.header);
    patch.push('\n');
    for line in &hunk.lines {
        patch.push_str(line);
        patch.push('\n');
    }
    patch
}

// ------------------------------------------------------------ gutter diff

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineChange {
    Added,
    Modified,
    /// Lines were removed just above this line.
    DeletedAbove,
}

/// Per-line change markers of `current` against `base`.
pub fn line_changes(base: &str, current: &str) -> Vec<(usize, LineChange)> {
    let diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .timeout(std::time::Duration::from_millis(60))
        .diff_lines(base, current);
    let mut changes = Vec::new();
    for op in diff.ops() {
        match *op {
            similar::DiffOp::Insert { new_index, new_len, .. } => {
                changes.extend((new_index..new_index + new_len).map(|l| (l, LineChange::Added)));
            }
            similar::DiffOp::Replace { new_index, new_len, .. } => {
                changes.extend((new_index..new_index + new_len).map(|l| (l, LineChange::Modified)));
            }
            similar::DiffOp::Delete { new_index, .. } => changes.push((new_index, LineChange::DeletedAbove)),
            similar::DiffOp::Equal { .. } => {}
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hunks() {
        let diff =
            "diff --git a/x.rs b/x.rs\nindex 1..2 100644\n--- a/x.rs\n+++ b/x.rs\n@@ -1,2 +1,2 @@\n-a\n+b\n c\n@@ -10 +10 @@\n-x\n+y\n";
        let files = parse_diff(diff);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "x.rs");
        assert_eq!(files[0].hunks.len(), 2);
        assert_eq!(files[0].hunks[1].new_start, 10);
        let patch = hunk_patch(&files[0], &files[0].hunks[1]);
        assert!(patch.starts_with("diff --git") && patch.contains("@@ -10 +10 @@\n-x\n+y\n"));
    }

    #[test]
    fn manages_remotes() {
        let host = Host::Local;
        let dir = std::env::temp_dir().join(format!("lumen-git-remote-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.to_string_lossy().into_owned();
        init(&host, &root).unwrap();
        assert!(remotes(&host, &root).unwrap().is_empty());
        add_remote(&host, &root, "origin", "git@example.com:me/repo.git").unwrap();
        set_remote_url(&host, &root, "origin", "https://example.com/me/repo.git").unwrap();
        assert_eq!(remotes(&host, &root).unwrap(), vec![Remote { name: "origin".into(), url: "https://example.com/me/repo.git".into() }]);
        remove_remote(&host, &root, "origin").unwrap();
        assert!(remotes(&host, &root).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_branches_in_other_worktrees() {
        let host = Host::Local;
        let dir = std::env::temp_dir().join(format!("lumen-git-worktree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.join("main").to_string_lossy().into_owned();
        let other = dir.join("other").to_string_lossy().into_owned();
        std::fs::create_dir_all(&root).unwrap();
        init(&host, &root).unwrap();
        git(&host, &root, &["-c", "user.name=t", "-c", "user.email=t@t", "commit", "--allow-empty", "-m", "init"]).unwrap();
        git(&host, &root, &["worktree", "add", "-b", "side", &other]).unwrap();
        let branches = branches(&host, &root).unwrap();
        let side = branches.iter().find(|b| b.name == "side").unwrap();
        assert!(!side.current);
        assert_eq!(std::path::Path::new(&side.worktree).canonicalize().unwrap(), std::path::Path::new(&other).canonicalize().unwrap());
        assert!(checkout(&host, &root, side).unwrap_err().contains("already used by worktree"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gutter_changes() {
        let changes = line_changes("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(changes, vec![(1, LineChange::Modified), (3, LineChange::Added)]);
    }
}
