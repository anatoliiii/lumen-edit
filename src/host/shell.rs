//! Filesystem operations over plain SSH commands, used when the agent is not
//! available on the remote host.

use lumen_shared::fsops::{self, DirEntry, SearchMatch, SearchQuery, Stat};

use super::{Remote, run_with_input, shell_quote};

fn run(remote: &Remote, script: &str, stdin: Option<&[u8]>) -> Result<Vec<u8>, String> {
    run_with_input(&mut remote.ssh_command(&format!("LC_ALL=C; export LC_ALL; {script}"), false), stdin)
}

pub fn simple(remote: &Remote, script: &str) -> Result<(), String> {
    run(remote, script, None).map(|_| ())
}

pub fn stat(remote: &Remote, path: &str) -> Result<Stat, String> {
    // GNU stat, then BSD stat (macOS): "directory" / "Directory".
    let quoted = shell_quote(path);
    let out = run(remote, &format!("stat -L -c '%F|%s|%Y' -- {quoted} 2>/dev/null || stat -L -f '%HT|%z|%m' -- {quoted}"), None)?;
    let out = String::from_utf8_lossy(&out);
    let mut parts = out.trim().split('|');
    let kind = parts.next().unwrap_or("");
    let size = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let mtime: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok(Stat { is_dir: kind.eq_ignore_ascii_case("directory"), size, mtime: mtime * 1000 })
}

pub fn read_dir(remote: &Remote, path: &str) -> Result<Vec<DirEntry>, String> {
    let out = run(remote, &format!("ls -1ApL -- {}", shell_quote(path)), None)?;
    let mut entries: Vec<DirEntry> = String::from_utf8_lossy(&out)
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| match line.strip_suffix('/') {
            Some(name) => DirEntry { name: name.to_string(), is_dir: true },
            None => DirEntry { name: line.to_string(), is_dir: false },
        })
        .collect();
    fsops::sort_entries(&mut entries);
    Ok(entries)
}

pub fn read_file(remote: &Remote, path: &str) -> Result<Vec<u8>, String> {
    run(remote, &format!("cat -- {}", shell_quote(path)), None)
}

pub fn write_file(remote: &Remote, path: &str, data: &[u8]) -> Result<(), String> {
    run(remote, &format!("cat > {}", shell_quote(path)), Some(data)).map(|_| ())
}

pub fn list_files(remote: &Remote, root: &str, max: usize) -> Result<Vec<String>, String> {
    let script = format!(
        "cd {} && {{ git ls-files -co --exclude-standard 2>/dev/null || find . -type f -not -path '*/.git/*' -not -path '*/node_modules/*' | sed 's|^\\./||'; }} | head -n {max}",
        shell_quote(root)
    );
    let out = run(remote, &script, None)?;
    Ok(String::from_utf8_lossy(&out).lines().map(str::to_string).collect())
}

pub fn search(remote: &Remote, root: &str, query: &SearchQuery) -> Result<(Vec<SearchMatch>, bool), String> {
    let regex = fsops::build_regex(query)?;
    let max = if query.max_results == 0 { 5000 } else { query.max_results };
    let pattern = shell_quote(&query.pattern);
    let mut rg = String::from("rg --no-config -n --no-heading --color=never --hidden -g '!.git'");
    let mut grep = String::from("grep -rnI --exclude-dir=.git --exclude-dir=node_modules");
    if !query.case_sensitive {
        rg.push_str(" -i");
        grep.push_str(" -i");
    }
    if query.whole_word {
        rg.push_str(" -w");
        grep.push_str(" -w");
    }
    if query.regex {
        grep.push_str(" -E");
    } else {
        rg.push_str(" -F");
        grep.push_str(" -F");
    }
    let script = format!(
        "cd {} && if command -v rg >/dev/null 2>&1; then {rg} -e {pattern} .; else {grep} -e {pattern} .; fi | head -n {}",
        shell_quote(root),
        max + 1
    );
    // grep/rg exit with 1 when nothing matched.
    let out = match run(remote, &script, None) {
        Ok(out) => out,
        Err(error) if error.is_empty() => Vec::new(),
        Err(error) => return Err(error),
    };
    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&out).lines() {
        let mut parts = line.splitn(3, ':');
        let (Some(path), Some(number), Some(text)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let Ok(number) = number.parse::<u32>() else { continue };
        let path = path.strip_prefix("./").unwrap_or(path).to_string();
        let (start, end) = regex.find(text).map(|m| (m.start(), m.end())).unwrap_or((0, 0));
        matches.push(SearchMatch {
            path,
            line: number.saturating_sub(1),
            start: start as u32,
            end: end as u32,
            text: text.chars().take(300).collect(),
        });
    }
    let truncated = matches.len() > max;
    matches.truncate(max);
    Ok((matches, truncated))
}
