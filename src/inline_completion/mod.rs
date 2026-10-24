//! Inline completion: editor identity and permissions stay outside the provider.
pub mod next_edit;
pub mod provider;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMode {
    #[default]
    MercuryEdit2,
    OpenRouterChat,
}
/// One output limit for every FIM provider, the payload and the settings UI.
pub const MAX_TOKENS: u32 = 512;

/// Named output budgets for FIM; `max_tokens` remains the stored value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FimProfile {
    Short,
    MultiLine,
}
impl FimProfile {
    pub const ALL: [FimProfile; 2] = [FimProfile::Short, FimProfile::MultiLine];
    pub fn max_tokens(self) -> u32 {
        match self {
            FimProfile::Short => 32,
            FimProfile::MultiLine => 128,
        }
    }
    pub fn of(max_tokens: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.max_tokens() == max_tokens)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub projects: Vec<String>,
    #[serde(default)]
    pub limits_version: u8,
    pub paused: bool,
    pub provider: ProviderMode,
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
    pub timeout_ms: u64,
    pub debounce_ms: u64,
    pub requests_per_minute: usize,
    pub requests_per_session: u64,
    pub session_budget_usd: f64,
    /// Conservative reservation for requests whose provider omits usage.cost.
    pub reserve_per_request_usd: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            limits_version: 1,
            paused: false,
            provider: ProviderMode::MercuryEdit2,
            model: "mistralai/codestral-2508".into(),
            temperature: 0.1,
            max_tokens: 32,
            timeout_ms: 6000,
            debounce_ms: 180,
            requests_per_minute: 60,
            requests_per_session: 5000,
            session_budget_usd: 1.0,
            reserve_per_request_usd: 0.01,
        }
    }
}
impl Settings {
    pub fn migrate_limits(&mut self) {
        if self.limits_version == 0 {
            if self.requests_per_minute == 12 && self.requests_per_session == 300 {
                self.requests_per_minute = 60;
                self.requests_per_session = 5000;
            }
            self.limits_version = 1;
        }
    }
}
#[derive(Default, Clone, Debug)]
pub struct Context {
    pub language: String,
    pub semantic: String,
    pub prefix: String,
    pub suffix: String,
}
impl Context {
    pub fn prompt(&self) -> String {
        let before: String =
            self.prefix.rsplit('\n').next().unwrap_or("").chars().rev().take(160).collect::<Vec<_>>().into_iter().rev().collect();
        let after: String = self.suffix.lines().next().unwrap_or("").chars().take(160).collect();
        let cursor_line = format!("{before}<CURSOR>{after}");
        serde_json::json!({
            "language":self.language,
            "semantic_context":self.semantic,
            "insertion_line":cursor_line,
            "code_prefix":self.prefix,
            "code_suffix":self.suffix,
            "task":"Return only the missing insertion between code_prefix and code_suffix. Preserve the existing suffix. Do not continue another function or append code after the suffix."
        }).to_string()
    }
}

mod budget;
mod controller;
mod credentials;
pub mod renderer;
pub mod semantic;
mod validation;
pub use controller::{Controller, Ghost, Identity};

/// Observe all buffer versions (including edits made through agents/control/LSP).
/// This runs before keyboard input and again after input, never holding a rope
/// lock or waiting for a worker.
pub fn tick(app: &mut crate::app::App, ctx: &eframe::egui::Context) {
    use crate::app::Tab;
    use std::hash::{Hash, Hasher};
    let fingerprint = serde_json::to_string(&app.config.inline_completion).unwrap_or_default();
    if app.inline_completion.settings_fingerprint != fingerprint {
        app.inline_completion.invalidate();
        app.inline_completion.settings_fingerprint = fingerprint;
        for tab in &mut app.tabs {
            if let Tab::Doc(doc) = tab {
                doc.view.ghost = None;
            }
        }
    }
    let mut epoch = std::collections::hash_map::DefaultHasher::new();
    for tab in &app.tabs {
        if let Tab::Doc(doc) = tab {
            doc.id.hash(&mut epoch);
            doc.buffer.version.hash(&mut epoch);
        }
    }
    app.inline_completion.revision.hash(&mut epoch);
    let epoch = epoch.finish();
    let workspace = app.workspace.as_ref().filter(|w| !w.host.is_remote());
    let project = workspace
        .map(|w| w.session_key())
        .filter(|key| !app.config.inline_completion.paused && app.config.inline_completion.projects.contains(key));
    let identity = if !app.agent_manager.open && app.palette.is_none() && app.dialog.is_none() && !app.settings_open {
        project.as_ref().and_then(|key| {
            app.active_tab.and_then(|i| app.tabs.get(i)).and_then(|tab| {
                let Tab::Doc(doc) = tab else { return None };
                if doc.large
                    || doc.preview
                    || doc.buffer.len_chars() > 256 * 1024
                    || doc.buffer.selections.len() != 1
                    || !doc.buffer.primary().is_empty()
                {
                    return None;
                }
                Some(Identity { project: key.clone(), doc: doc.id, version: doc.buffer.version, cursor: doc.buffer.primary().head, epoch })
            })
        })
    } else {
        None
    };
    let edited = app.inline_completion.changed_version(identity.as_ref());
    app.inline_completion.observe(identity.clone(), edited);
    for tab in &mut app.tabs {
        if let Tab::Doc(doc) = tab {
            if doc.view.ghost.as_ref().is_some_and(|g| identity.as_ref() != Some(&g.identity) || !g.valid(doc.id, &doc.buffer)) {
                doc.view.ghost = None;
            }
            app.inline_completion.metrics.accepted += std::mem::take(&mut doc.view.ai_accepted);
            if std::mem::take(&mut doc.view.ai_rejected) {
                app.inline_completion.reject();
            }
        }
    }
    if let Some(ghost) = app.inline_completion.poll(&app.config.inline_completion)
        && let Some(Tab::Doc(doc)) = app.active_tab.and_then(|i| app.tabs.get_mut(i))
        && doc.view.completion.is_none()
        && ghost.valid(doc.id, &doc.buffer)
    {
        doc.view.ghost = Some(ghost);
    }
    let manual = app.active_tab.and_then(|i| app.tabs.get_mut(i)).is_some_and(|tab| match tab {
        Tab::Doc(doc) => std::mem::take(&mut doc.view.ai_manual),
        _ => false,
    });
    if let Some(identity) = identity {
        if let Some(delay) = app.inline_completion.wake_after(&app.config.inline_completion) {
            ctx.request_repaint_after(delay);
        }
        if app.inline_completion.ready(&app.config.inline_completion, manual) && !app.next_edit.busy {
            let Some(Tab::Doc(doc)) = app.active_tab.and_then(|i| app.tabs.get_mut(i)) else { return };
            if doc.view.next_edit_preview.is_some()
                || doc.view.next_edit_requested
                || doc.view.completion.is_some()
                || doc.view.rename.is_some()
                || doc.view.code_actions.is_some()
                || doc.view.find.is_some()
            {
                return;
            }
            app.lsp.sync(doc);
            let mut buffer = crate::text::Buffer::new("");
            buffer.rope = doc.buffer.rope.clone();
            buffer.version = doc.buffer.version;
            buffer.selections = doc.buffer.selections.clone();
            let clients = app.lsp.inline_clients(doc);
            let lang = doc.lang;
            let path = doc.path.clone();
            let related = app
                .tabs
                .iter()
                .filter_map(|tab| match tab {
                    Tab::Doc(d) if d.buffer.rope.len_bytes() <= 65536 => Some((d.path.clone(), d.buffer.rope.clone())),
                    _ => None,
                })
                .take(24)
                .collect();
            let snapshot = semantic::Snapshot {
                buffer,
                lang,
                cursor: identity.cursor,
                path,
                root: workspace.unwrap().root.clone().into(),
                clients,
                related,
            };
            app.inline_completion.launch(snapshot, app.config.inline_completion.clone(), ctx.clone(), manual);
        }
    }
}

impl next_edit::Controller {
    // Keeps preview identity tied to the foreground document and serializes
    // explicit Next Edit calls with the existing FIM worker.
    pub fn tick(&mut self, app: &mut crate::app::App, ctx: &eframe::egui::Context) {
        use crate::app::Tab;
        let active = app.active_tab.and_then(|i| app.tabs.get(i));
        let identity =
            active.and_then(|t| if let Tab::Doc(d) = t { Some((d.id, d.buffer.version, d.buffer.primary().head)) } else { None });
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        if self.busy && self.pending_identity != identity {
            self.invalidate();
            self.status = "Next Edit cancelled: active document or cursor changed".into();
        }
        self.settle_abandoned(&mut app.inline_completion.ledger);
        if let Some(preview) = self.poll(identity, &mut app.inline_completion.ledger) {
            if let Some(Tab::Doc(doc)) = app.active_tab.and_then(|i| app.tabs.get_mut(i)) {
                doc.view.next_edit_preview = Some(preview);
            }
            ctx.request_repaint();
        }
        for (index, tab) in app.tabs.iter_mut().enumerate() {
            if let Tab::Doc(doc) = tab {
                if doc.view.next_edit_preview.as_ref().is_some_and(|p| !p.is_current(doc.id, &doc.buffer) || Some(index) != app.active_tab)
                {
                    doc.view.next_edit_preview = None;
                }
                if Some(index) != app.active_tab && doc.view.next_edit_requested {
                    doc.view.next_edit_requested = false;
                }
            }
        }
        let Some(index) = app.active_tab else {
            return;
        };
        let Some(Tab::Doc(doc)) = app.tabs.get_mut(index) else {
            return;
        };
        if !doc.view.next_edit_requested {
            return;
        }
        if app.inline_completion.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(40));
            return;
        }
        doc.view.next_edit_requested = false;
        let Some(workspace) = app.workspace.as_ref().filter(|w| !w.host.is_remote()) else {
            self.status = "Next Edit requires a local workspace".into();
            return;
        };
        let key = workspace.session_key();
        if app.config.inline_completion.paused || !app.config.inline_completion.projects.contains(&key) {
            self.status = "Allow cloud completion for this project in Settings first".into();
            return;
        }
        if doc.view.completion.is_some() || doc.view.rename.is_some() || doc.view.code_actions.is_some() || doc.view.find.is_some() {
            self.status = "Close the active editor popup before requesting Next Edit".into();
            return;
        }
        // Same session limits, budget and 429 backoff as FIM.
        let ticket = match app.inline_completion.ledger.reserve(&app.config.inline_completion) {
            Ok(ticket) => ticket,
            Err(refusal) => {
                self.status = refusal.message("Next Edit");
                return;
            }
        };
        doc.view.ghost = None;
        let root = workspace.root.clone();
        let history = doc.next_edit_history.entries().cloned().collect::<Vec<_>>();
        self.launch(
            std::path::Path::new(&root),
            &doc.path,
            doc.id,
            &doc.buffer,
            doc.lang,
            &history,
            app.config.next_edit_region_lines,
            ticket,
        );
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}

pub fn tick_next_edit(app: &mut crate::app::App, ctx: &eframe::egui::Context) {
    let mut controller = std::mem::take(&mut app.next_edit);
    controller.tick(app, ctx);
    app.next_edit = controller;
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    #[test]
    fn migrate_only_original_small_limits() {
        let mut old: Settings = toml::from_str("requests_per_minute=12\nrequests_per_session=300").unwrap();
        old.migrate_limits();
        assert_eq!(old.requests_per_minute, 60);
        assert_eq!(old.requests_per_session, 5000);
        let mut custom: Settings = toml::from_str("requests_per_minute=8\nrequests_per_session=100").unwrap();
        custom.migrate_limits();
        assert_eq!(custom.requests_per_minute, 8);
        assert_eq!(custom.requests_per_session, 100);
        old.requests_per_minute = 12;
        old.requests_per_session = 300;
        old.migrate_limits();
        assert_eq!(old.requests_per_minute, 12);
    }
}
