//! User settings and recent projects in `~/.config/lumen-edit/config.toml`.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

use crate::lsp::ServerSpec;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    /// `local` or `ssh:<target>`.
    pub host: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Interface language: `auto` (system locale), `en` or `ru`.
    pub language: String,
    pub inline_completion: crate::inline_completion::Settings,
    pub next_edit_region_lines: usize,
    pub ui_font_size: f32,
    pub editor_font_size: f32,
    pub terminal_font_size: f32,
    /// Optional TTF/OTF for code and terminal, e.g. JetBrains Mono.
    pub editor_font: Option<String>,
    pub tab_size: usize,
    pub insert_spaces: bool,
    pub format_on_save: bool,
    pub auto_save: bool,
    pub show_whitespace: bool,
    pub inline_blame: bool,
    /// Upload and use the helper agent on SSH hosts.
    pub use_agent: bool,
    pub pull_rebase: bool,
    /// The pi coding agent: command, optional Node entry point, runtime PATH,
    /// extra arguments and sandboxing. `pi_entry` is used by the private
    /// Node.js installation managed from Settings.
    pub pi_command: String,
    pub pi_entry: String,
    pub pi_runtime: String,
    pub pi_args: Vec<String>,
    pub agent_sandbox: bool,
    /// Model for new agent sessions (`provider/id`); empty = pi's default.
    pub agent_model: String,
    /// pi-modes mode applied at start (e.g. build); empty = pi's default.
    pub agent_mode: String,
    /// Show the agent beside the code instead of in a tab.
    pub agent_docked: bool,
    /// Live CPU/memory/disk of the project host in the status bar.
    pub pulse: bool,
    /// Extra or replacement language servers (matched by `name`).
    pub lsp: Vec<ServerSpec>,
    pub disabled_servers: Vec<String>,
    pub recent: Vec<RecentProject>,
    pub ssh_hosts: Vec<String>,
    /// Open files per project (`host|root`), restored on reopen.
    pub sessions: HashMap<String, Vec<String>>,
    /// Open Pi conversations per project; prompts are never replayed on restore.
    pub agent_sessions: HashMap<String, Vec<crate::app::agent::team::SavedAgent>>,
    /// Shortcuts that differ from the defaults: command id -> chords like
    /// `Ctrl+Shift+P` (Ctrl is Cmd on macOS, `MacCtrl` its Control key);
    /// `[]` unbinds the command.
    pub keybindings: BTreeMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            inline_completion: Default::default(),
            next_edit_region_lines: 15,
            ui_font_size: 13.5,
            editor_font_size: 14.0,
            terminal_font_size: 13.5,
            editor_font: None,
            tab_size: 4,
            insert_spaces: true,
            format_on_save: false,
            auto_save: false,
            show_whitespace: false,
            inline_blame: true,
            use_agent: true,
            pull_rebase: false,
            pi_command: "pi".into(),
            pi_entry: String::new(),
            pi_runtime: String::new(),
            pi_args: Vec::new(),
            agent_sandbox: true,
            agent_model: "openrouter/openai/gpt-6-luna".into(),
            agent_mode: "build".into(),
            pulse: true,
            agent_docked: true,
            lsp: Vec::new(),
            disabled_servers: Vec::new(),
            recent: Vec::new(),
            ssh_hosts: Vec::new(),
            sessions: HashMap::new(),
            agent_sessions: HashMap::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("lumen-edit").join("config.toml")
}

impl Config {
    pub fn load() -> Self {
        // Settings of the editor's former name (Codium RS).
        let legacy = dirs::config_dir().unwrap_or_default().join("codium-rs").join("config.toml");
        if !config_path().exists() && legacy.exists() {
            if let Some(dir) = config_path().parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::copy(&legacy, config_path());
        }
        let mut config: Self = match std::fs::read_to_string(config_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|error| {
                eprintln!("lumen-edit: {}: {error}", config_path().display());
                Config::default()
            }),
            Err(_) => Config::default(),
        };
        config.inline_completion.migrate_limits();
        config
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn add_recent(&mut self, host: &str, path: &str) {
        let entry = RecentProject { host: host.into(), path: path.into() };
        self.recent.retain(|r| *r != entry);
        self.recent.insert(0, entry);
        self.recent.truncate(20);
    }

    pub fn servers(&self) -> Vec<ServerSpec> {
        let mut servers = crate::lsp::default_servers();
        for custom in &self.lsp {
            servers.retain(|s| s.name != custom.name);
            servers.push(custom.clone());
        }
        servers.retain(|s| !self.disabled_servers.contains(&s.name));
        servers
    }
}
