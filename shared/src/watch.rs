//! Bounded file watching for project trees.
//!
//! A recursive inotify watch walks the whole tree before it returns, which on
//! a big folder (`/DATA`, a home directory) takes minutes and burns a core.
//! Here folders are watched one by one from a background walk that skips
//! ignored and build directories and stops after [`MAX_DIRS`].

use std::{
    collections::BTreeSet,
    path::Path,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Upper bound on watched folders; the rest of a huge tree goes unwatched.
pub const MAX_DIRS: usize = 20_000;

/// Folders that are never worth watching.
const SKIP: &[&str] = &["target", "node_modules", ".cache", "__pycache__", ".venv", "venv", "lost+found", ".Trash-1000"];

/// Keeps the watch alive; dropping it stops watching.
pub struct TreeWatcher {
    _watcher: Arc<Mutex<RecommendedWatcher>>,
}

/// Watches `root` and calls `sink` with batches of changed paths. Returns at
/// once; folders are added in the background.
pub fn watch_tree(root: &str, sink: impl Fn(Vec<String>) + Send + 'static) -> Result<TreeWatcher, String> {
    let (tx, rx) = mpsc::channel::<notify::Event>();
    let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event
            && !matches!(event.kind, EventKind::Access(_))
        {
            let _ = tx.send(event);
        }
    })
    .map_err(|e| e.to_string())?;
    let watcher = Arc::new(Mutex::new(watcher));
    let count = Arc::new(AtomicUsize::new(0));

    let weak = Arc::downgrade(&watcher);
    let walk_count = count.clone();
    let root = root.to_string();
    thread::spawn(move || {
        let walk = ignore::WalkBuilder::new(&root).hidden(false).filter_entry(|entry| !skipped(entry.path())).build();
        for entry in walk.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_dir()) {
                continue;
            }
            if !add(&weak, &walk_count, entry.path()) {
                break;
            }
        }
    });

    // Batch bursts of events (builds, checkouts) into one call.
    let weak = Arc::downgrade(&watcher);
    thread::spawn(move || {
        while let Ok(first) = rx.recv() {
            let mut paths = BTreeSet::new();
            let mut take = |event: notify::Event| {
                let created = matches!(event.kind, EventKind::Create(_));
                for path in event.paths {
                    if skipped(&path) {
                        continue;
                    }
                    if created && path.is_dir() {
                        add(&weak, &count, &path);
                    }
                    paths.insert(path.to_string_lossy().into_owned());
                }
            };
            take(first);
            while let Ok(event) = rx.recv_timeout(Duration::from_millis(150)) {
                take(event);
            }
            if !paths.is_empty() {
                sink(paths.into_iter().collect());
            }
        }
    });

    Ok(TreeWatcher { _watcher: watcher })
}

/// Watches one folder; false once the watcher is gone or the limit is hit.
fn add(watcher: &Weak<Mutex<RecommendedWatcher>>, count: &AtomicUsize, dir: &Path) -> bool {
    let Some(watcher) = watcher.upgrade() else { return false };
    if count.fetch_add(1, Ordering::Relaxed) >= MAX_DIRS {
        return false;
    }
    if let Ok(mut watcher) = watcher.lock() {
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }
    true
}

fn skipped(path: &Path) -> bool {
    let name = |p: &Path| p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
    // Kernel pseudo filesystems when the project is `/`.
    if matches!(path.to_str(), Some("/proc" | "/sys" | "/dev" | "/run")) {
        return true;
    }
    let file = name(path);
    // `.git` itself stays watched (HEAD, index, refs drive the git panel).
    SKIP.contains(&file.as_str()) || (file == "objects" && path.parent().is_some_and(|p| name(p) == ".git"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_changes_in_nested_and_new_folders() {
        let dir = std::env::temp_dir().join(format!("lumen-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::create_dir_all(dir.join("target/debug")).unwrap();
        let (tx, rx) = mpsc::channel();
        let _watch = watch_tree(dir.to_str().unwrap(), move |paths| {
            let _ = tx.send(paths);
        })
        .unwrap();
        thread::sleep(Duration::from_millis(300));

        let wait = |needle: &str| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                if let Ok(paths) = rx.recv_timeout(left)
                    && paths.iter().any(|p| p.ends_with(needle))
                {
                    return true;
                }
            }
            false
        };
        std::fs::write(dir.join("a/b/file.txt"), "x").unwrap();
        assert!(wait("a/b/file.txt"));
        std::fs::create_dir(dir.join("a/new")).unwrap();
        thread::sleep(Duration::from_millis(400));
        std::fs::write(dir.join("a/new/inner.txt"), "x").unwrap();
        assert!(wait("a/new/inner.txt"));
        std::fs::write(dir.join("target/debug/out"), "x").unwrap();
        assert!(!wait("target/debug/out"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
