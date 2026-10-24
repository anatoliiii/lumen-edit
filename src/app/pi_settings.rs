//! Visual editor for Pi's user-level `models.json` and `settings.json`.
//! Provider/model names are read from Pi configuration; none are built into
//! Lumen's model picker or provider editor.
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use eframe::egui::{self, RichText};
use serde_json::{Map, Value, json};

use super::App;
use crate::theme;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Section {
    #[default]
    Providers,
    Models,
    PiPreferences,
    LumenAgent,
}

#[derive(Clone, Debug, Default)]
struct ProviderDraft {
    id: String,
    original_id: String,
    name: String,
    base_url: String,
    api: String,
    api_key: String,
    is_new: bool,
}

#[derive(Clone, Debug)]
struct ModelDraft {
    id: String,
    original_id: String,
    name: String,
    api: String,
    context: String,
    max_tokens: String,
    input_cost: String,
    output_cost: String,
    cache_read_cost: String,
    cache_write_cost: String,
    reasoning: bool,
    text_input: bool,
    image_input: bool,
    is_new: bool,
}

impl Default for ModelDraft {
    fn default() -> Self {
        Self {
            id: String::new(),
            original_id: String::new(),
            name: String::new(),
            api: String::new(),
            context: "128000".into(),
            max_tokens: "8192".into(),
            input_cost: "0".into(),
            output_cost: "0".into(),
            cache_read_cost: "0".into(),
            cache_write_cost: "0".into(),
            reasoning: false,
            text_input: true,
            image_input: false,
            is_new: true,
        }
    }
}

#[derive(Clone, Debug)]
struct PiSettingsUi {
    loaded: bool,
    section: Section,
    models: Value,
    settings: Value,
    models_error: Option<String>,
    settings_error: Option<String>,
    selected_provider: String,
    provider: ProviderDraft,
    model_provider: String,
    selected_model: String,
    model: ModelDraft,
    confirm_provider_delete: bool,
    confirm_model_delete: bool,
    pi_default_model: String,
    pi_default_provider_only: String,
    pi_default_thinking: String,
    pi_enabled_models: String,
    pi_hide_thinking: bool,
    pi_quiet_startup: String,
    message: Option<(String, bool)>,
}

impl Default for PiSettingsUi {
    fn default() -> Self {
        Self {
            loaded: false,
            section: Section::Providers,
            models: json!({"providers": {}}),
            settings: json!({}),
            models_error: None,
            settings_error: None,
            selected_provider: String::new(),
            provider: ProviderDraft::default(),
            model_provider: String::new(),
            selected_model: String::new(),
            model: ModelDraft::default(),
            confirm_provider_delete: false,
            confirm_model_delete: false,
            pi_default_model: String::new(),
            pi_default_provider_only: String::new(),
            pi_default_thinking: "medium".into(),
            pi_enabled_models: String::new(),
            pi_hide_thinking: false,
            pi_quiet_startup: "false".into(),
            message: None,
        }
    }
}

fn agent_dir() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_default();
    std::env::var_os("PI_CODING_AGENT_DIR")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| path.strip_prefix("~/").map(|suffix| home.join(suffix)).unwrap_or(path))
        .unwrap_or_else(|| home.join(".pi/agent"))
}

fn models_path() -> PathBuf {
    agent_dir().join("models.json")
}

fn settings_path() -> PathBuf {
    agent_dir().join("settings.json")
}

fn next_backup_path(path: &Path) -> PathBuf {
    let file_name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "config.json".into());
    let first = path.with_file_name(format!("{file_name}.lumen-backup"));
    if !first.exists() {
        return first;
    }
    for index in 1u64.. {
        let candidate = path.with_file_name(format!("{file_name}.lumen-backup-{index}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

fn read_json(path: &Path, default: Value) -> Result<Value, String> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| format!("{}: {error}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(default),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let parent = path.parent().ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    if path.exists() {
        let backup = next_backup_path(path);
        fs::copy(path, &backup).map_err(|error| format!("Could not back up {}: {error}", path.display()))?;
        #[cfg(unix)]
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o600)).map_err(|error| format!("{}: {error}", backup.display()))?;
    }
    let serialized = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(path)
    }
    .map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(not(unix))]
    let mut file =
        OpenOptions::new().write(true).create(true).truncate(true).open(path).map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(serialized.as_bytes()).map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(b"\n").map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(())
}

fn object_field<'a>(value: &'a Value, key: &str) -> &'a Map<String, Value> {
    value.get(key).and_then(Value::as_object).unwrap_or_else(|| empty_object())
}

fn empty_object() -> &'static Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Map::new)
}

fn provider_ids(models: &Value) -> Vec<String> {
    object_field(models, "providers").keys().cloned().collect()
}

fn provider_models(models: &Value, provider: &str) -> Vec<Value> {
    models.pointer(&format!("/providers/{}/models", pointer_escape(provider))).and_then(Value::as_array).cloned().unwrap_or_default()
}

fn pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn model_options(models: &Value, app: &App) -> Vec<(String, String)> {
    let mut options = BTreeMap::new();
    for provider in provider_ids(models) {
        for model in provider_models(models, &provider) {
            add_model_option(&mut options, &provider, &model);
        }
    }
    for tab in &app.tabs {
        if let crate::app::Tab::Agent(agent) = tab {
            for model in agent.available_models() {
                if let Some(provider) = model.get("provider").and_then(Value::as_str) {
                    add_model_option(&mut options, provider, model);
                }
            }
        }
    }
    if let Some(agent) = &app.agent_dock {
        for model in agent.available_models() {
            if let Some(provider) = model.get("provider").and_then(Value::as_str) {
                add_model_option(&mut options, provider, model);
            }
        }
    }
    options.into_iter().collect()
}

#[cfg(test)]
fn model_keys(models: &Value) -> Vec<String> {
    let mut keys = Vec::new();
    for provider in provider_ids(models) {
        for model in provider_models(models, &provider) {
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                keys.push(format!("{provider}/{id}"));
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn add_model_option(options: &mut BTreeMap<String, String>, provider: &str, model: &Value) {
    let Some(id) = model.get("id").and_then(Value::as_str) else { return };
    let key = format!("{provider}/{id}");
    let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
    options.insert(key.clone(), format!("{name}  ·  {key}"));
}

fn provider_options(models: &Value, app: &App) -> Vec<String> {
    let mut providers: std::collections::BTreeSet<String> = provider_ids(models).into_iter().collect();
    for (key, _) in model_options(models, app) {
        if let Some((provider, _)) = key.split_once('/') {
            providers.insert(provider.to_owned());
        }
    }
    providers.into_iter().collect()
}

fn provider_draft(models: &Value, id: &str) -> ProviderDraft {
    let provider = models.pointer(&format!("/providers/{}", pointer_escape(id))).unwrap_or(&Value::Null);
    ProviderDraft {
        id: id.to_owned(),
        original_id: id.to_owned(),
        name: provider.get("name").and_then(Value::as_str).unwrap_or_default().into(),
        base_url: provider.get("baseUrl").and_then(Value::as_str).unwrap_or_default().into(),
        api: provider.get("api").and_then(Value::as_str).unwrap_or_default().into(),
        api_key: provider.get("apiKey").and_then(Value::as_str).unwrap_or_default().into(),
        is_new: false,
    }
}

fn json_number(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_f64).map(|number| number.to_string()).unwrap_or_else(|| "0".into())
}

fn model_draft(value: &Value) -> ModelDraft {
    let cost = value.get("cost").unwrap_or(&Value::Null);
    let input = value.get("input").and_then(Value::as_array).cloned().unwrap_or_default();
    ModelDraft {
        id: value.get("id").and_then(Value::as_str).unwrap_or_default().into(),
        original_id: value.get("id").and_then(Value::as_str).unwrap_or_default().into(),
        name: value.get("name").and_then(Value::as_str).unwrap_or_default().into(),
        api: value.get("api").and_then(Value::as_str).unwrap_or_default().into(),
        context: value.get("contextWindow").map(Value::to_string).unwrap_or_else(|| "128000".into()),
        max_tokens: value.get("maxTokens").map(Value::to_string).unwrap_or_else(|| "8192".into()),
        input_cost: json_number(cost, "input"),
        output_cost: json_number(cost, "output"),
        cache_read_cost: json_number(cost, "cacheRead"),
        cache_write_cost: json_number(cost, "cacheWrite"),
        reasoning: value.get("reasoning").and_then(Value::as_bool).unwrap_or(false),
        text_input: input.iter().any(|item| item.as_str() == Some("text")),
        image_input: input.iter().any(|item| item.as_str() == Some("image")),
        is_new: false,
    }
}

fn load_ui() -> PiSettingsUi {
    let mut state = PiSettingsUi { loaded: true, ..Default::default() };
    match read_json(&models_path(), json!({"providers": {}})) {
        Ok(models) if models.is_object() && models.get("providers").is_none_or(Value::is_object) => state.models = models,
        Ok(_) => state.models_error = Some(t!("models.json must be a JSON object with an object-valued providers field.").into()),
        Err(error) => state.models_error = Some(error),
    }
    match read_json(&settings_path(), json!({})) {
        Ok(settings) if settings.is_object() => state.settings = settings,
        Ok(_) => state.settings_error = Some(t!("settings.json must contain a JSON object.").into()),
        Err(error) => state.settings_error = Some(error),
    }
    let providers = provider_ids(&state.models);
    state.selected_provider = providers.first().cloned().unwrap_or_default();
    state.provider = if state.selected_provider.is_empty() {
        ProviderDraft { id: "my-provider".into(), api: "openai-completions".into(), is_new: true, ..Default::default() }
    } else {
        provider_draft(&state.models, &state.selected_provider)
    };
    state.model_provider = state.selected_provider.clone();
    if let Some(model) = provider_models(&state.models, &state.model_provider).first() {
        state.model = model_draft(model);
        state.selected_model = state.model.id.clone();
    }
    state.pi_default_model =
        match (state.settings.get("defaultProvider").and_then(Value::as_str), state.settings.get("defaultModel").and_then(Value::as_str)) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            (None, Some(model)) => model.into(),
            _ => String::new(),
        };
    if state.pi_default_model.is_empty() {
        state.pi_default_provider_only = state.settings.get("defaultProvider").and_then(Value::as_str).unwrap_or_default().into();
    }
    state.pi_default_thinking = state.settings.get("defaultThinkingLevel").and_then(Value::as_str).unwrap_or("medium").into();
    state.pi_enabled_models = state
        .settings
        .get("enabledModels")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    state.pi_hide_thinking = state.settings.get("hideThinkingBlock").and_then(Value::as_bool).unwrap_or(false);
    state.pi_quiet_startup = match state.settings.get("quietStartup") {
        Some(Value::Bool(true)) => "true".into(),
        Some(Value::String(value)) if value == "header" => "header".into(),
        _ => "false".into(),
    };
    state
}

fn select_provider(state: &mut PiSettingsUi, id: &str) {
    state.selected_provider = id.to_owned();
    state.provider = provider_draft(&state.models, id);
}

fn select_model(state: &mut PiSettingsUi, provider: &str, id: &str) {
    state.model_provider = provider.to_owned();
    state.selected_model = id.to_owned();
    let model = provider_models(&state.models, provider).into_iter().find(|model| model.get("id").and_then(Value::as_str) == Some(id));
    state.model = model.map_or_else(ModelDraft::default, |value| model_draft(&value));
}

fn save_models(state: &mut PiSettingsUi) {
    if let Some(error) = &state.models_error {
        state.message = Some((t!("Cannot save because the current models.json could not be loaded: {error}", error = error), true));
        return;
    }
    match write_json(&models_path(), &state.models) {
        Ok(()) => state.message = Some((t!("Provider and model configuration saved.").into(), false)),
        Err(error) => state.message = Some((t!("Could not save Pi model configuration: {error}", error = error), true)),
    }
}

fn save_settings(state: &mut PiSettingsUi) {
    if let Some(error) = &state.settings_error {
        state.message = Some((t!("Cannot save because the current settings.json could not be loaded: {error}", error = error), true));
        return;
    }
    match write_json(&settings_path(), &state.settings) {
        Ok(()) => state.message = Some((t!("Pi preferences saved.").into(), false)),
        Err(error) => state.message = Some((t!("Could not save Pi preferences: {error}", error = error), true)),
    }
}

fn settings_section(ui: &mut egui::Ui, title: &str) {
    super::settings_layout::section(ui, title);
}

fn draw_error(ui: &mut egui::Ui, error: Option<&String>) {
    if let Some(error) = error {
        ui.colored_label(theme::ERROR, t!("Cannot read Pi configuration: {error}", error = error));
    }
}

fn draw_providers(ui: &mut egui::Ui, state: &mut PiSettingsUi) {
    draw_error(ui, state.models_error.as_ref());
    ui.label(RichText::new(t!("Editing file: {path}", path = models_path().display())).small().color(super::settings_layout::HELP_TEXT));
    ui.label(
        RichText::new(t!("Custom providers from models.json. Provider IDs are used as the prefix in model IDs."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    ui.add_space(6.0);
    super::settings_layout::master_detail(ui, |ui, list| {
        if list {
            ui.heading(t!("Providers"));
            ui.separator();
            for id in provider_ids(&state.models) {
                let count = provider_models(&state.models, &id).len();
                if ui.selectable_label(state.selected_provider == id, format!("{id}  ·  {count}")).clicked() {
                    select_provider(state, &id);
                    state.confirm_provider_delete = false;
                }
            }
            if ui.button(t!("+ Add provider")).clicked() {
                state.provider =
                    ProviderDraft { id: "my-provider".into(), api: "openai-completions".into(), is_new: true, ..Default::default() };
                state.selected_provider.clear();
                state.confirm_provider_delete = false;
            }
        } else {
            ui.heading(if state.provider.is_new { t!("New provider") } else { t!("Edit provider") });
            ui.separator();
            ui.label(t!("Provider ID"));
            ui.add(egui::TextEdit::singleline(&mut state.provider.id).hint_text("my-provider"));
            if !state.provider.is_new {
                ui.label(
                    RichText::new(t!("Changing the ID renames the provider and keeps its models."))
                        .small()
                        .color(super::settings_layout::HELP_TEXT),
                );
            }
            ui.label(t!("Display name"));
            ui.text_edit_singleline(&mut state.provider.name);
            ui.label(t!("API type"));
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut state.provider.api).hint_text("openai-completions"));
                egui::ComboBox::from_id_salt("pi_api_type_suggestions").selected_text(t!("Common API types")).show_ui(ui, |ui| {
                    for api in [
                        "openai-completions",
                        "openai-responses",
                        "anthropic-messages",
                        "google-generative-ai",
                        "google-vertex",
                        "bedrock-converse-stream",
                        "azure-openai-responses",
                        "openai-codex-responses",
                        "mistral-conversations",
                        "pi-messages",
                    ] {
                        if ui.selectable_label(state.provider.api == api, api).clicked() {
                            state.provider.api = api.into();
                        }
                    }
                });
            });
            ui.label(
                RichText::new(t!("Use an API type supported by Pi; custom APIs need a Pi extension."))
                    .small()
                    .color(super::settings_layout::HELP_TEXT),
            );
            ui.label(t!("Base URL"));
            ui.add(egui::TextEdit::singleline(&mut state.provider.base_url).hint_text("https://api.example.com/v1"));
            ui.label(t!("API key or environment reference"));
            ui.add(egui::TextEdit::singleline(&mut state.provider.api_key).password(true).hint_text("$API_KEY or !command"));
            ui.label(
                RichText::new(t!("A literal key is stored in Pi's user config. Prefer $ENV_NAME to keep secrets outside the file."))
                    .small()
                    .color(super::settings_layout::HELP_TEXT),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.button(t!("Save provider")).clicked() {
                    save_provider(state);
                }
                if !state.provider.is_new && !state.confirm_provider_delete && ui.button(t!("Remove provider…")).clicked() {
                    state.confirm_provider_delete = true;
                }
            });
            if state.confirm_provider_delete {
                ui.colored_label(theme::WARNING, t!("This also removes every model under this provider."));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t!("Delete provider and its models")).clicked() {
                        delete_provider(state);
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        state.confirm_provider_delete = false;
                    }
                });
            }
        }
    });
}

fn save_provider(state: &mut PiSettingsUi) {
    let id = state.provider.id.trim().to_owned();
    if id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte)) {
        state.message = Some((t!("Provider ID must contain only letters, digits, dot, underscore or hyphen.").into(), true));
        return;
    }
    if state.provider.api.trim().is_empty() {
        state.message = Some((t!("API type cannot be empty.").into(), true));
        return;
    }
    if state.provider.base_url.trim().is_empty() {
        state.message = Some((t!("Base URL cannot be empty.").into(), true));
        return;
    }
    let providers = state.models["providers"].as_object_mut();
    let Some(providers) = providers else {
        state.message = Some((t!("The providers section in models.json must be an object.").into(), true));
        return;
    };
    if state.provider.is_new && providers.contains_key(&id) {
        state.message = Some((t!("A provider with this ID already exists.").into(), true));
        return;
    }
    let old_id = state.provider.original_id.clone();
    let mut value = if !state.provider.is_new && old_id != id {
        if providers.contains_key(&id) {
            state.message = Some((t!("A provider with this ID already exists.").into(), true));
            return;
        }
        providers.remove(&old_id).unwrap_or_else(|| json!({}))
    } else {
        providers.get(&id).cloned().unwrap_or_else(|| json!({}))
    };
    value["name"] = if state.provider.name.trim().is_empty() { Value::Null } else { json!(state.provider.name.trim()) };
    if value["name"].is_null()
        && let Some(object) = value.as_object_mut()
    {
        object.remove("name");
    }
    value["baseUrl"] = json!(state.provider.base_url.trim());
    value["api"] = json!(state.provider.api.trim());
    if state.provider.api_key.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.remove("apiKey");
        }
    } else {
        value["apiKey"] = json!(state.provider.api_key);
    }
    if !value.get("models").is_some_and(Value::is_array) {
        value["models"] = json!([]);
    }
    providers.insert(id.clone(), value);
    if state.model_provider.is_empty() || state.model_provider == old_id {
        state.model_provider = id.clone();
    }
    state.selected_provider = id.clone();
    state.provider.original_id = id.clone();
    state.provider.id = id.clone();
    state.provider.is_new = false;
    state.confirm_provider_delete = false;
    save_models(state);
}

fn delete_provider(state: &mut PiSettingsUi) {
    if let Some(providers) = state.models.get_mut("providers").and_then(Value::as_object_mut) {
        providers.remove(&state.selected_provider);
    }
    if state.model_provider == state.selected_provider {
        state.model_provider.clear();
    }
    state.selected_provider.clear();
    state.provider = ProviderDraft { id: "my-provider".into(), api: "openai-completions".into(), is_new: true, ..Default::default() };
    state.confirm_provider_delete = false;
    save_models(state);
}

fn draw_models(ui: &mut egui::Ui, state: &mut PiSettingsUi) {
    draw_error(ui, state.models_error.as_ref());
    ui.label(RichText::new(t!("Editing file: {path}", path = models_path().display())).small().color(super::settings_layout::HELP_TEXT));
    ui.label(
        RichText::new(t!("Models are stored under their provider. Select a model to edit its limits, capabilities and pricing."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    ui.add_space(6.0);
    let providers = provider_ids(&state.models);
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("Provider"));
        egui::ComboBox::from_id_salt("pi_models_provider")
            .selected_text(if state.model_provider.is_empty() { t!("Select provider") } else { &state.model_provider })
            .show_ui(ui, |ui| {
                for provider in &providers {
                    if ui.selectable_label(state.model_provider == *provider, provider).clicked() {
                        state.model_provider = provider.clone();
                        state.selected_model.clear();
                        state.model = ModelDraft::default();
                    }
                }
            });
        if !state.model_provider.is_empty() && ui.button(t!("+ Add model")).clicked() {
            if let Some(provider) = state.models["providers"].get_mut(&state.model_provider)
                && !provider.get("models").is_some_and(Value::is_array)
            {
                provider["models"] = json!([]);
            }
            state.model = ModelDraft::default();
            state.selected_model.clear();
            state.model.is_new = true;
            state.confirm_model_delete = false;
        }
    });
    if state.model_provider.is_empty() {
        ui.colored_label(super::settings_layout::HELP_TEXT, t!("Add a provider first, then add models under it."));
        return;
    }
    super::settings_layout::master_detail(ui, |ui, list| {
        if list {
            ui.heading(t!("Models"));
            ui.separator();
            for model in provider_models(&state.models, &state.model_provider) {
                let id = model.get("id").and_then(Value::as_str).unwrap_or("?");
                let name = model.get("name").and_then(Value::as_str).unwrap_or(id);
                if ui.selectable_label(state.selected_model == id, format!("{name}\n{id}")).clicked() {
                    select_model(state, &state.model_provider.clone(), id);
                    state.confirm_model_delete = false;
                }
            }
        } else {
            ui.heading(if state.model.is_new { t!("New model") } else { t!("Edit model") });
            ui.separator();
            ui.label(t!("Model ID"));
            ui.text_edit_singleline(&mut state.model.id);
            ui.label(t!("Display name"));
            ui.text_edit_singleline(&mut state.model.name);
            ui.label(t!("API override (optional)"));
            ui.add(egui::TextEdit::singleline(&mut state.model.api).hint_text(t!("Inherit provider API")));
            super::settings_layout::row(ui, t!("Context window (tokens)"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut state.model.context).desired_width(160.0));
            });
            super::settings_layout::row(ui, t!("Maximum output (tokens)"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut state.model.max_tokens).desired_width(160.0));
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut state.model.reasoning, t!("Reasoning"));
                ui.checkbox(&mut state.model.text_input, t!("Text input"));
                ui.checkbox(&mut state.model.image_input, t!("Image input"));
            });
            egui::CollapsingHeader::new(t!("Pricing per million tokens (USD)")).show(ui, |ui| {
                ui.vertical(|ui| {
                    for (label, value) in [
                        (t!("Input"), &mut state.model.input_cost),
                        (t!("Output"), &mut state.model.output_cost),
                        (t!("Cache read"), &mut state.model.cache_read_cost),
                        (t!("Cache write"), &mut state.model.cache_write_cost),
                    ] {
                        super::settings_layout::row(ui, label, |ui| {
                            ui.add(egui::TextEdit::singleline(value).desired_width(160.0));
                        });
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button(t!("Save model")).clicked() {
                    save_model(state);
                }
                if !state.model.is_new && !state.confirm_model_delete && ui.button(t!("Remove model…")).clicked() {
                    state.confirm_model_delete = true;
                }
            });
            if state.confirm_model_delete {
                ui.colored_label(theme::WARNING, t!("Removing a model may affect saved defaults."));
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t!("Delete model")).clicked() {
                        delete_model(state);
                    }
                    if ui.button(t!("Cancel")).clicked() {
                        state.confirm_model_delete = false;
                    }
                });
            }
        }
    });
}

fn parse_positive_u64(text: &str, label: &str) -> Result<u64, String> {
    text.trim().parse::<u64>().ok().filter(|value| *value > 0).ok_or_else(|| t!("{label} must be a positive integer.", label = label))
}

fn parse_cost(text: &str, label: &str) -> Result<f64, String> {
    let value = text.trim().parse::<f64>().map_err(|_| t!("{label} must be a non-negative number.", label = label))?;
    if !value.is_finite() || value < 0.0 {
        return Err(t!("{label} must be a non-negative number.").into());
    }
    Ok(value)
}

fn save_model(state: &mut PiSettingsUi) {
    let id = state.model.id.trim().to_owned();
    if id.is_empty() {
        state.message = Some((t!("Model ID cannot be empty.").into(), true));
        return;
    }
    if !state.model.text_input && !state.model.image_input {
        state.message = Some((t!("Choose at least one input type.").into(), true));
        return;
    }
    let context = match parse_positive_u64(&state.model.context, t!("Context window")) {
        Ok(value) => value,
        Err(error) => {
            state.message = Some((error, true));
            return;
        }
    };
    let max_tokens = match parse_positive_u64(&state.model.max_tokens, t!("Maximum output")) {
        Ok(value) => value,
        Err(error) => {
            state.message = Some((error, true));
            return;
        }
    };
    let costs = [
        parse_cost(&state.model.input_cost, t!("Input cost")),
        parse_cost(&state.model.output_cost, t!("Output cost")),
        parse_cost(&state.model.cache_read_cost, t!("Cache read cost")),
        parse_cost(&state.model.cache_write_cost, t!("Cache write cost")),
    ];
    let mut parsed_costs = Vec::new();
    for cost in costs {
        match cost {
            Ok(value) => parsed_costs.push(value),
            Err(error) => {
                state.message = Some((error, true));
                return;
            }
        }
    }
    let Some(models) =
        state.models.pointer_mut(&format!("/providers/{}/models", pointer_escape(&state.model_provider))).and_then(Value::as_array_mut)
    else {
        state.message = Some((t!("Select or save a provider before adding models.").into(), true));
        return;
    };
    if models
        .iter()
        .any(|value| value.get("id").and_then(Value::as_str) == Some(id.as_str()) && (state.model.is_new || state.model.original_id != id))
    {
        state.message = Some((t!("A model with this ID already exists under this provider.").into(), true));
        return;
    }
    let existing_index = if state.model.is_new {
        None
    } else {
        models.iter().position(|value| value.get("id").and_then(Value::as_str) == Some(&state.model.original_id))
    };
    let mut value = existing_index.map_or_else(|| json!({}), |index| models.remove(index));
    value["id"] = json!(id);
    set_optional_string(&mut value, "name", &state.model.name);
    set_optional_string(&mut value, "api", &state.model.api);
    value["contextWindow"] = json!(context);
    value["maxTokens"] = json!(max_tokens);
    value["reasoning"] = json!(state.model.reasoning);
    let mut input = Vec::new();
    if state.model.text_input {
        input.push("text");
    }
    if state.model.image_input {
        input.push("image");
    }
    value["input"] = json!(input);
    value["cost"] =
        json!({"input": parsed_costs[0], "output": parsed_costs[1], "cacheRead": parsed_costs[2], "cacheWrite": parsed_costs[3]});
    models.push(value);
    state.selected_model = id.clone();
    state.model.id = id.clone();
    state.model.original_id = id;
    state.model.is_new = false;
    state.confirm_model_delete = false;
    save_models(state);
}

fn set_optional_string(value: &mut Value, key: &str, text: &str) {
    if text.trim().is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.remove(key);
        }
    } else {
        value[key] = json!(text.trim());
    }
}

fn delete_model(state: &mut PiSettingsUi) {
    if let Some(models) =
        state.models.pointer_mut(&format!("/providers/{}/models", pointer_escape(&state.model_provider))).and_then(Value::as_array_mut)
    {
        models.retain(|model| model.get("id").and_then(Value::as_str) != Some(state.selected_model.as_str()));
    }
    state.selected_model.clear();
    state.model = ModelDraft::default();
    state.confirm_model_delete = false;
    save_models(state);
}

fn draw_defaults(ui: &mut egui::Ui, state: &mut PiSettingsUi, app: &App) {
    draw_error(ui, state.settings_error.as_ref());
    ui.label(RichText::new(t!("Editing file: {path}", path = settings_path().display())).small().color(super::settings_layout::HELP_TEXT));
    ui.label(
        RichText::new(t!("Pi preferences are stored separately from provider/model definitions."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    settings_section(ui, t!("Startup model"));
    let available_models = model_options(&state.models, app);
    let current_pi_default =
        available_models.iter().find(|(key, _)| key == &state.pi_default_model).map(|(_, label)| label.as_str()).unwrap_or(
            if state.pi_default_model.is_empty() {
                if state.pi_default_provider_only.is_empty() { t!("Automatic (Pi chooses)") } else { &state.pi_default_provider_only }
            } else {
                &state.pi_default_model
            },
        );
    ui.label(t!("Default model for Pi sessions"));
    egui::ComboBox::from_id_salt("pi_default_model_config").selected_text(current_pi_default).show_ui(ui, |ui| {
        if ui
            .selectable_label(state.pi_default_model.is_empty() && state.pi_default_provider_only.is_empty(), t!("Automatic (Pi chooses)"))
            .clicked()
        {
            state.pi_default_model.clear();
            state.pi_default_provider_only.clear();
        }
        for (key, label) in &available_models {
            if ui.selectable_label(state.pi_default_model == *key, label).clicked() {
                state.pi_default_model = key.clone();
                state.pi_default_provider_only.clear();
            }
        }
    });
    if !state.pi_default_model.is_empty() && !available_models.iter().any(|(key, _)| key == &state.pi_default_model) {
        ui.label(
            RichText::new(t!("This saved model is not in the current list. Reload Pi or add the model under Models."))
                .small()
                .color(theme::WARNING),
        );
    }
    let providers = provider_options(&state.models, app);
    let current_provider = if state.pi_default_provider_only.is_empty() { t!("Any provider") } else { &state.pi_default_provider_only };
    ui.label(t!("Optional provider restriction"));
    egui::ComboBox::from_id_salt("pi_default_provider_config").selected_text(current_provider).show_ui(ui, |ui| {
        if ui.selectable_label(state.pi_default_provider_only.is_empty(), t!("Any provider")).clicked() {
            state.pi_default_provider_only.clear();
        }
        for provider in providers {
            if ui.selectable_label(state.pi_default_provider_only == provider, &provider).clicked() {
                state.pi_default_provider_only = provider;
                state.pi_default_model.clear();
            }
        }
    });
    settings_section(ui, t!("Conversation and display"));
    ui.label(t!("Default thinking level"));
    egui::ComboBox::from_id_salt("pi_default_thinking").selected_text(&state.pi_default_thinking).show_ui(ui, |ui| {
        for level in ["off", "minimal", "low", "medium", "high", "xhigh", "max"] {
            ui.selectable_value(&mut state.pi_default_thinking, level.into(), level);
        }
    });
    egui::CollapsingHeader::new(t!("Advanced model cycling")).show(ui, |ui| {
        ui.label(t!("Optional provider/model patterns, one per line. Empty keeps every model."));
        ui.add(egui::TextEdit::multiline(&mut state.pi_enabled_models).desired_rows(3).hint_text("provider/*"));
    });
    ui.checkbox(&mut state.pi_hide_thinking, t!("Hide reasoning blocks"));
    ui.label(t!("Startup output"));
    egui::ComboBox::from_id_salt("pi_quiet_startup")
        .selected_text(match state.pi_quiet_startup.as_str() {
            "true" => t!("Quiet"),
            "header" => t!("Header only"),
            _ => t!("Normal"),
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut state.pi_quiet_startup, "false".into(), t!("Normal"));
            ui.selectable_value(&mut state.pi_quiet_startup, "header".into(), t!("Header only"));
            ui.selectable_value(&mut state.pi_quiet_startup, "true".into(), t!("Quiet"));
        });
    if ui.button(t!("Save Pi preferences")).clicked() {
        save_pi_preferences(state);
    }
}

fn draw_lumen_agent(ui: &mut egui::Ui, state: &mut PiSettingsUi, app: &mut App) {
    ui.label(
        RichText::new(t!("Lumen's model selection is separate from Pi's settings.json defaults."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    settings_section(ui, t!("Default model for Lumen-hosted Pi sessions"));
    let available_models = model_options(&state.models, app);
    let current_lumen_default = available_models
        .iter()
        .find(|(key, _)| key == &app.config.agent_model)
        .map(|(_, label)| label.as_str())
        .unwrap_or(if app.config.agent_model.is_empty() { t!("Use Pi default") } else { &app.config.agent_model });
    egui::ComboBox::from_id_salt("lumen_agent_model_config").selected_text(current_lumen_default).show_ui(ui, |ui| {
        if ui.selectable_label(app.config.agent_model.is_empty(), t!("Use Pi default")).clicked() {
            app.config.agent_model.clear();
        }
        for (key, label) in &available_models {
            if ui.selectable_label(app.config.agent_model == *key, label).clicked() {
                app.config.agent_model = key.clone();
            }
        }
    });
    if !app.config.agent_model.is_empty() && !available_models.iter().any(|(key, _)| key == &app.config.agent_model) {
        ui.label(
            RichText::new(t!("This saved model is not in the current list. Reload Pi or add the model under Models."))
                .small()
                .color(theme::WARNING),
        );
    }
    ui.label(
        RichText::new(t!("The list combines models.json with models reported by open Pi sessions."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    if ui.button(t!("Save Lumen agent default")).clicked() {
        app.config.save();
        state.message = Some((t!("Lumen's default agent model updated.").into(), false));
    }
}

fn save_pi_preferences(state: &mut PiSettingsUi) {
    if let Err(error) = apply_pi_preferences(state) {
        state.message = Some((error, true));
        return;
    }
    save_settings(state);
}

fn apply_pi_preferences(state: &mut PiSettingsUi) -> Result<(), String> {
    if !state.settings.is_object() {
        return Err(t!("settings.json must contain a JSON object.").into());
    }
    if !state.pi_default_model.trim().is_empty() {
        let model = state.pi_default_model.trim();
        if let Some((provider, model_id)) = model.split_once('/') {
            if provider.trim().is_empty() || model_id.trim().is_empty() {
                return Err(t!("Enter a model as provider/model, or leave it empty.").into());
            }
            state.settings["defaultProvider"] = json!(provider.trim());
            state.settings["defaultModel"] = json!(model_id.trim());
        } else if !state.pi_default_provider_only.trim().is_empty() {
            state.settings["defaultProvider"] = json!(state.pi_default_provider_only.trim());
            state.settings["defaultModel"] = json!(model);
        } else {
            if let Some(object) = state.settings.as_object_mut() {
                object.remove("defaultProvider");
            }
            state.settings["defaultModel"] = json!(model);
        }
    } else if !state.pi_default_provider_only.trim().is_empty() {
        state.settings["defaultProvider"] = json!(state.pi_default_provider_only.trim());
        if let Some(object) = state.settings.as_object_mut() {
            object.remove("defaultModel");
        }
    } else if let Some(object) = state.settings.as_object_mut() {
        object.remove("defaultProvider");
        object.remove("defaultModel");
    }
    state.settings["defaultThinkingLevel"] = json!(state.pi_default_thinking);
    let enabled: Vec<&str> = state.pi_enabled_models.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
    if enabled.is_empty() {
        if let Some(object) = state.settings.as_object_mut() {
            object.remove("enabledModels");
        }
    } else {
        state.settings["enabledModels"] = json!(enabled);
    }
    state.settings["hideThinkingBlock"] = json!(state.pi_hide_thinking);
    state.settings["quietStartup"] = match state.pi_quiet_startup.as_str() {
        "true" => json!(true),
        "header" => json!("header"),
        _ => json!(false),
    };
    Ok(())
}

pub fn show(ui: &mut egui::Ui, app: &mut App) {
    let id = egui::Id::unique("pi_configuration_ui");
    let mut state = ui.ctx().data_mut(|data| data.get_temp::<PiSettingsUi>(id)).unwrap_or_default();
    if !state.loaded {
        state = load_ui();
    }
    ui.horizontal_wrapped(|ui| {
        for (section, label) in [
            (Section::Providers, t!("Providers")),
            (Section::Models, t!("Models")),
            (Section::PiPreferences, t!("Pi preferences")),
            (Section::LumenAgent, t!("Lumen agent")),
        ] {
            ui.selectable_value(&mut state.section, section, label);
        }
        if ui.button(t!("Reload Pi configuration")).clicked() {
            state = load_ui();
        }
    });
    ui.separator();
    ui.label(RichText::new(t!("Pi directory: {path}", path = agent_dir().display())).small().color(super::settings_layout::HELP_TEXT));
    ui.label(
        RichText::new(t!("Saved changes are used by new Pi sessions; restart a running agent to reload its catalog."))
            .small()
            .color(super::settings_layout::HELP_TEXT),
    );
    if let Some((message, error)) = &state.message {
        ui.colored_label(if *error { theme::ERROR } else { theme::SUCCESS }, message);
    }
    egui::ScrollArea::vertical().id_salt(("pi_settings", state.section as u8)).auto_shrink([false, false]).show(ui, |ui| {
        match state.section {
            Section::Providers => draw_providers(ui, &mut state),
            Section::Models => draw_models(ui, &mut state),
            Section::PiPreferences => draw_defaults(ui, &mut state, app),
            Section::LumenAgent => draw_lumen_agent(ui, &mut state, app),
        }
    });
    ui.ctx().data_mut(|data| data.insert_temp(id, state));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn model_list_is_read_from_provider_configuration_not_built_in() {
        let models = json!({
            "providers": {
                "custom": {"models": [{"id": "one"}, {"id": "two"}]},
                "another": {"models": [{"id": "other"}]}
            }
        });
        assert_eq!(model_keys(&models), ["another/other", "custom/one", "custom/two"]);
    }

    #[test]
    fn pointer_escape_handles_provider_ids_from_configuration() {
        let models = json!({"providers": {"example/provider~1": {"models": [{"id": "model"}]}}});
        assert_eq!(provider_models(&models, "example/provider~1").len(), 1);
    }

    #[test]
    fn app_settings_can_be_saved_without_replacing_unrelated_keys() {
        let mut state = PiSettingsUi {
            settings: json!({"customSetting": 42}),
            pi_default_model: "provider-a/model-a".into(),
            pi_default_thinking: "high".into(),
            pi_enabled_models: "provider-a/*".into(),
            pi_hide_thinking: true,
            pi_quiet_startup: "true".into(),
            ..Default::default()
        };
        assert!(apply_pi_preferences(&mut state).is_ok());
        assert_eq!(state.settings["customSetting"], 42);
        assert_eq!(state.settings["defaultProvider"], "provider-a");
        assert_eq!(state.settings["defaultModel"], "model-a");
        assert_eq!(state.settings["defaultThinkingLevel"], "high");
        assert_eq!(state.settings["enabledModels"][0], "provider-a/*");
        assert_eq!(state.settings["hideThinkingBlock"], true);
        assert_eq!(state.settings["quietStartup"], true);
    }

    #[test]
    fn pi_preferences_preserve_provider_only_and_header_startup_modes() {
        let mut state = PiSettingsUi {
            settings: json!({"defaultProvider": "provider-a", "quietStartup": "header"}),
            pi_default_provider_only: "provider-a".into(),
            pi_quiet_startup: "header".into(),
            ..Default::default()
        };
        assert!(apply_pi_preferences(&mut state).is_ok());
        assert_eq!(state.settings["defaultProvider"], "provider-a");
        assert!(state.settings.get("defaultModel").is_none());
        assert_eq!(state.settings["quietStartup"], "header");
    }

    #[test]
    fn saving_json_keeps_backup_and_restricts_file_permissions() {
        let directory = std::env::temp_dir().join(format!("pi-config-ui-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        assert!(fs::create_dir_all(&directory).is_ok());
        let path = directory.join("models.json");
        let original = json!({"providers": {"keep": {"models": []}}});
        assert!(fs::write(&path, serde_json::to_vec(&original).unwrap_or_default()).is_ok());
        let updated = json!({"providers": {"new": {"models": []}}});
        assert!(write_json(&path, &updated).is_ok());
        let backup = path.with_file_name("models.json.lumen-backup");
        let next_backup = path.with_file_name("models.json.lumen-backup-1");
        assert_eq!(read_json(&backup, Value::Null).ok(), Some(original.clone()));
        assert_eq!(read_json(&path, Value::Null).ok(), Some(updated.clone()));
        assert!(write_json(&path, &json!({"providers": {"later": {}}})).is_ok());
        assert_eq!(read_json(&backup, Value::Null).ok(), Some(original));
        assert_eq!(read_json(&next_backup, Value::Null).ok(), Some(updated));
        #[cfg(unix)]
        assert_eq!(fs::metadata(path).map(|metadata| metadata.permissions().mode() & 0o777).ok(), Some(0o600));
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn model_key_parser_keeps_slashes_inside_model_id() {
        assert_eq!("provider-a/vendor/model-a".split_once('/'), Some(("provider-a", "vendor/model-a")));
    }
}
