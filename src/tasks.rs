//! Run/build/test tasks: detected from project files, plus user tasks from
//! `.lumen/tasks.toml`.

use serde::Deserialize;

use crate::host::{Host, join};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Group {
    Run,
    Build,
    Test,
    #[default]
    Other,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Run => t!("Run"),
            Group::Build => t!("Build"),
            Group::Test => t!("Tests"),
            Group::Other => t!("Other"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Task {
    pub label: String,
    pub command: String,
    /// Relative to the project root.
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub group: Group,
    #[serde(default)]
    pub default: bool,
    #[serde(skip)]
    pub source: String,
}

#[derive(Deserialize)]
struct TasksFile {
    #[serde(default, rename = "task")]
    tasks: Vec<Task>,
}

pub const TASKS_FILE: &str = ".lumen/tasks.toml";
const LEGACY_TASKS_FILE: &str = ".codium/tasks.toml";

/// Starter `.lumen/tasks.toml`, commented in the interface language.
pub fn tasks_template() -> &'static str {
    t_file!("tasks.toml")
}

fn task(label: &str, command: &str, group: Group, source: &str) -> Task {
    Task { label: label.into(), command: command.into(), cwd: None, group, default: false, source: source.into() }
}

pub fn detect(host: &Host, root: &str) -> (Vec<Task>, Option<String>) {
    let mut tasks = Vec::new();
    let mut error = None;
    let names: Vec<String> = host.read_dir(root).map(|e| e.into_iter().map(|e| e.name).collect()).unwrap_or_default();
    let has = |name: &str| names.iter().any(|n| n == name);

    // `.codium/tasks.toml` is the location used before the rename.
    if let Ok(data) = host.read_file(&join(root, TASKS_FILE)).or_else(|_| host.read_file(&join(root, LEGACY_TASKS_FILE))) {
        match toml::from_str::<TasksFile>(&String::from_utf8_lossy(&data)) {
            Ok(file) => tasks.extend(file.tasks.into_iter().map(|mut t| {
                t.source = "tasks.toml".into();
                t
            })),
            Err(e) => error = Some(format!("{TASKS_FILE}: {e}")),
        }
    }

    if has("Cargo.toml") {
        tasks.push(task("cargo run", "cargo run", Group::Run, "cargo"));
        tasks.push(task("cargo run --release", "cargo run --release", Group::Run, "cargo"));
        tasks.push(task("cargo build", "cargo build", Group::Build, "cargo"));
        tasks.push(task("cargo build --release", "cargo build --release", Group::Build, "cargo"));
        tasks.push(task("cargo check", "cargo check", Group::Build, "cargo"));
        tasks.push(task("cargo clippy", "cargo clippy", Group::Build, "cargo"));
        tasks.push(task("cargo test", "cargo test", Group::Test, "cargo"));
    }
    if has("go.mod") {
        tasks.push(task("go run .", "go run .", Group::Run, "go"));
        tasks.push(task("go build ./...", "go build ./...", Group::Build, "go"));
        tasks.push(task("go test ./...", "go test ./...", Group::Test, "go"));
        tasks.push(task("go vet ./...", "go vet ./...", Group::Build, "go"));
    }
    if has("CMakeLists.txt") {
        tasks.push(task(
            "cmake build",
            "cmake -S . -B build -DCMAKE_EXPORT_COMPILE_COMMANDS=ON && cmake --build build -j",
            Group::Build,
            "cmake",
        ));
        tasks.push(task("ctest", "ctest --test-dir build --output-on-failure", Group::Test, "cmake"));
    }
    if has("meson.build") {
        tasks.push(task("meson build", "meson setup build 2>/dev/null; meson compile -C build", Group::Build, "meson"));
        tasks.push(task("meson test", "meson test -C build", Group::Test, "meson"));
    }
    for makefile in ["Makefile", "makefile", "GNUmakefile"] {
        if has(makefile) {
            tasks.push(task("make", "make", Group::Build, "make"));
            if let Ok(data) = host.read_file(&join(root, makefile)) {
                for target in make_targets(&String::from_utf8_lossy(&data)) {
                    let group = match target.as_str() {
                        "run" => Group::Run,
                        "test" | "check" => Group::Test,
                        "build" | "all" => Group::Build,
                        _ => Group::Other,
                    };
                    tasks.push(task(&format!("make {target}"), &format!("make {target}"), group, "make"));
                }
            }
            break;
        }
    }
    if has("justfile") || has("Justfile") {
        let file = if has("justfile") { "justfile" } else { "Justfile" };
        if let Ok(data) = host.read_file(&join(root, file)) {
            for recipe in make_targets(&String::from_utf8_lossy(&data)) {
                tasks.push(task(&format!("just {recipe}"), &format!("just {recipe}"), Group::Other, "just"));
            }
        }
    }
    let python = if has("uv.lock") { "uv run" } else { "python3" };
    if has("manage.py") {
        tasks.push(task("manage.py runserver", &format!("{python} manage.py runserver"), Group::Run, "python"));
    }
    for entry in ["main.py", "app.py", "__main__.py"] {
        if has(entry) {
            tasks.push(task(&format!("python {entry}"), &format!("{python} {entry}"), Group::Run, "python"));
        }
    }
    if has("pyproject.toml") || has("setup.py") || has("pytest.ini") || has("tests") && names.iter().any(|n| n.ends_with(".py")) {
        let pytest = if has("uv.lock") { "uv run pytest" } else { "python3 -m pytest" };
        tasks.push(task("pytest", pytest, Group::Test, "python"));
    }
    if names.iter().any(|n| n.ends_with(".csproj") || n.ends_with(".sln") || n.ends_with(".fsproj")) {
        tasks.push(task("dotnet run", "dotnet run", Group::Run, "dotnet"));
        tasks.push(task("dotnet build", "dotnet build", Group::Build, "dotnet"));
        tasks.push(task("dotnet test", "dotnet test", Group::Test, "dotnet"));
    }
    if has("package.json")
        && let Ok(data) = host.read_file(&join(root, "package.json"))
        && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&data)
        && let Some(scripts) = json.get("scripts").and_then(|s| s.as_object())
    {
        let runner = if has("bun.lockb") || has("bun.lock") {
            "bun run"
        } else if has("pnpm-lock.yaml") {
            "pnpm run"
        } else if has("yarn.lock") {
            "yarn"
        } else {
            "npm run"
        };
        for name in scripts.keys() {
            let group = match name.as_str() {
                "dev" | "start" | "serve" => Group::Run,
                "build" => Group::Build,
                "test" => Group::Test,
                _ => Group::Other,
            };
            tasks.push(task(&format!("{runner} {name}"), &format!("{runner} {name}"), group, "package.json"));
        }
    }
    (tasks, error)
}

fn make_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in text.lines() {
        if line.starts_with(['\t', ' ', '#', '.']) || line.contains(":=") || line.contains("?=") {
            continue;
        }
        if let Some((name, _)) = line.split_once(':') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || "-_".contains(c)) && !targets.contains(&name.to_string()) {
                targets.push(name.to_string());
            }
        }
    }
    targets
}

pub fn default_task(tasks: &[Task], group: Group) -> Option<&Task> {
    tasks.iter().find(|t| t.default && t.group == group).or_else(|| tasks.iter().find(|t| t.group == group))
}
