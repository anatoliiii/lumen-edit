//! Reads host aliases from `~/.ssh/config` (including `Include`d files) to
//! offer them in the connection dialog.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct SshHostEntry {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
}

pub fn read_hosts() -> Vec<SshHostEntry> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let mut hosts = Vec::new();
    parse_file(&home.join(".ssh/config"), &home, &mut hosts, 0);
    hosts.dedup_by(|a, b| a.alias == b.alias);
    hosts
}

fn parse_file(path: &Path, home: &Path, hosts: &mut Vec<SshHostEntry>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(text) = std::fs::read_to_string(path) else { return };
    let mut current: Vec<usize> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = match line.split_once(|c: char| c.is_whitespace() || c == '=') {
            Some((key, value)) => (key.to_ascii_lowercase(), value.trim().trim_start_matches('=').trim()),
            None => continue,
        };
        match key.as_str() {
            "host" => {
                current.clear();
                for alias in value.split_whitespace() {
                    if alias.contains(['*', '?', '!']) {
                        continue;
                    }
                    hosts.push(SshHostEntry { alias: alias.to_string(), hostname: None, user: None });
                    current.push(hosts.len() - 1);
                }
            }
            "match" => current.clear(),
            "hostname" => {
                for &index in &current {
                    hosts[index].hostname = Some(value.to_string());
                }
            }
            "user" => {
                for &index in &current {
                    hosts[index].user = Some(value.to_string());
                }
            }
            "include" => {
                for pattern in value.split_whitespace() {
                    for file in expand_include(pattern, home) {
                        parse_file(&file, home, hosts, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
}

fn expand_include(pattern: &str, home: &Path) -> Vec<PathBuf> {
    let path = if let Some(rest) = pattern.strip_prefix("~/") {
        home.join(rest)
    } else if pattern.starts_with('/') {
        PathBuf::from(pattern)
    } else {
        home.join(".ssh").join(pattern)
    };
    let text = path.to_string_lossy();
    if !text.contains('*') {
        return vec![path];
    }
    // Only a trailing `*` in the file name is supported, which covers the
    // usual `config.d/*`.
    let Some(dir) = path.parent() else { return Vec::new() };
    let prefix = path.file_name().map(|n| n.to_string_lossy().replace('*', "")).unwrap_or_default();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|p| p.is_file() && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix)))
        .collect();
    files.sort();
    files
}
