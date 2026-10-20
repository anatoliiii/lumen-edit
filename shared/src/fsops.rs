use std::{
    fs,
    io::{self, BufRead},
    path::Path,
    time::UNIX_EPOCH,
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct Stat {
    pub is_dir: bool,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SearchQuery {
    pub pattern: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub max_results: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchMatch {
    /// Path relative to the search root.
    pub path: String,
    /// Zero-based line number.
    pub line: u32,
    /// Byte range of the match inside `text`.
    pub start: u32,
    pub end: u32,
    pub text: String,
}

pub fn stat(path: &str) -> io::Result<Stat> {
    let meta = fs::metadata(path)?;
    let mtime =
        meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map(|duration| duration.as_millis() as u64).unwrap_or(0);
    Ok(Stat { is_dir: meta.is_dir(), size: meta.len(), mtime })
}

pub fn read_dir(path: &str) -> io::Result<Vec<DirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Follow symlinks so linked directories can be expanded.
        let is_dir = fs::metadata(entry.path()).map(|meta| meta.is_dir()).unwrap_or(false);
        entries.push(DirEntry { name, is_dir });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

pub fn sort_entries(entries: &mut [DirEntry]) {
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
}

/// Writes through a temporary file and renames it over the target, keeping
/// the original permissions.
pub fn write_atomic(path: &str, data: &[u8]) -> io::Result<()> {
    let target = Path::new(path);
    let file_name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let temp = target.with_file_name(format!(".{file_name}.lumen-tmp"));
    let permissions = fs::metadata(target).ok().map(|meta| meta.permissions());
    // Symlinks: write in place so the link itself is preserved.
    if fs::symlink_metadata(target).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
        return fs::write(target, data);
    }
    if fs::write(&temp, data).is_err() {
        // Directory not writable, but the file may be.
        return fs::write(target, data);
    }
    if let Some(permissions) = permissions {
        let _ = fs::set_permissions(&temp, permissions);
    }
    fs::rename(&temp, target).or_else(|_| {
        let _ = fs::remove_file(&temp);
        fs::write(target, data)
    })
}

pub fn remove(path: &str) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) }
}

fn walker(root: &str) -> ignore::Walk {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .filter_entry(|entry| {
            let name = entry.file_name();
            name != ".git" && name != "node_modules"
        })
        .build()
}

/// Lists files below `root` (relative paths), honoring `.gitignore`.
pub fn list_files(root: &str, max: usize) -> Vec<String> {
    let root_path = Path::new(root);
    let mut files = Vec::new();
    for entry in walker(root).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file() || t.is_symlink()) {
            continue;
        }
        if let Ok(relative) = entry.path().strip_prefix(root_path) {
            files.push(crate::path_string(relative));
            if files.len() >= max {
                break;
            }
        }
    }
    files
}

pub fn build_regex(query: &SearchQuery) -> Result<regex::Regex, String> {
    let mut pattern = if query.regex { query.pattern.clone() } else { regex::escape(&query.pattern) };
    if query.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    regex::RegexBuilder::new(&pattern).case_insensitive(!query.case_sensitive).build().map_err(|e| e.to_string())
}

pub fn search(root: &str, query: &SearchQuery) -> Result<(Vec<SearchMatch>, bool), String> {
    let regex = build_regex(query)?;
    let max = if query.max_results == 0 { 5000 } else { query.max_results };
    let root_path = Path::new(root);
    let mut matches = Vec::new();
    for entry in walker(root).flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(file) = fs::File::open(entry.path()) else { continue };
        if file.metadata().map(|m| m.len() > 8 * 1024 * 1024).unwrap_or(true) {
            continue;
        }
        let relative = crate::path_string(entry.path().strip_prefix(root_path).unwrap_or(entry.path()));
        let reader = io::BufReader::new(file);
        for (line_index, line) in reader.split(b'\n').enumerate() {
            let Ok(line) = line else { break };
            if line.contains(&0) {
                break; // binary file
            }
            let Ok(text) = std::str::from_utf8(&line) else { break };
            let text = text.trim_end_matches('\r');
            for found in regex.find_iter(text) {
                let (display, offset) = clip_line(text, found.start());
                matches.push(SearchMatch {
                    path: relative.clone(),
                    line: line_index as u32,
                    start: (found.start() - offset) as u32,
                    end: (found.end() - offset).min(display.len()) as u32,
                    text: display.to_string(),
                });
                if matches.len() >= max {
                    return Ok((matches, true));
                }
            }
        }
    }
    Ok((matches, false))
}

/// Keeps very long lines readable in the results list.
fn clip_line(text: &str, match_start: usize) -> (&str, usize) {
    const LIMIT: usize = 300;
    if text.len() <= LIMIT {
        return (text, 0);
    }
    let mut start = match_start.saturating_sub(80);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + LIMIT).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[start..end], start)
}
