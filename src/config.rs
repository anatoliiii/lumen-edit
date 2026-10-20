//! User settings and recent projects in `~/.config/codium-rs/config.toml`.

use std::{collections::HashMap, path::PathBuf};

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
    /// The pi coding agent: command, extra arguments and sandboxing.
    pub pi_command: String,
    pub pi_args: Vec<String>,
    pub agent_sandbox: bool,
    /// Model for new agent sessions (`provider/id`); empty = pi's default.
    pub agent_model: String,
    /// pi-modes mode applied at start (e.g. build); empty = pi's default.
    pub agent_mode: String,
    /// Extra or replacement language servers (matched by `name`).
    pub lsp: Vec<ServerSpec>,
    pub disabled_servers: Vec<String>,
    pub recent: Vec<RecentProject>,
    pub ssh_hosts: Vec<String>,
    /// Open files per project (`host|root`), restored on reopen.
    pub sessions: HashMap<String, Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
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
            pi_args: Vec::new(),
            agent_sandbox: true,
            agent_model: "openrouter/openai/gpt-6-luna".into(),
            agent_mode: "build".into(),
            lsp: Vec::new(),
            disabled_servers: Vec::new(),
            recent: Vec::new(),
            ssh_hosts: Vec::new(),
            sessions: HashMap::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("codium-rs").join("config.toml")
}

impl Config {
    pub fn load() -> Self {
        match std::fs::read_to_string(config_path()) {
            Ok(text) => toml::from_str(&text).unwrap_or_else(|error| {
                eprintln!("codium-rs: {}: {error}", config_path().display());
                Config::default()
            }),
            Err(_) => Config::default(),
        }
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
