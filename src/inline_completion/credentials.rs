//! Read-only reuse of Pi's user credentials. Never reads project .pi files,
//! writes credentials, executes key commands, or includes keys in diagnostics.
use serde_json::Value;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

fn read_json(path: &Path) -> Option<Value> {
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(512 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > 512 * 1024 {
        return None;
    }
    serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).ok()
}
fn resolve(raw: &str, entry: &Value, env: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    if raw.starts_with('!') {
        if let Some(key) = env("OPENROUTER_API_KEY").filter(|key| !key.trim().is_empty()) {
            return Ok(key);
        }
        return Err("Pi uses a credential command; configure OPENROUTER_API_KEY for inline completion".into());
    }
    let key = entry
        .get("env")
        .and_then(|e| e.get(raw))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| env(raw))
        .unwrap_or_else(|| raw.to_string());
    if key.trim().is_empty() { Err("OpenRouter API key is empty".into()) } else { Ok(key) }
}
fn from_directory(dir: &Path, env: impl Fn(&str) -> Option<String>) -> Result<String, String> {
    // Same persistent precedence as Pi: auth.json, models.json, environment.
    if let Some(auth) = read_json(&dir.join("auth.json"))
        && let Some(entry) = auth.get("openrouter")
        && entry.get("type").and_then(Value::as_str) == Some("api_key")
        && let Some(key) = entry.get("key").and_then(Value::as_str)
    {
        return resolve(key, entry, &env);
    }
    if let Some(models) = read_json(&dir.join("models.json"))
        && let Some(entry) = models.pointer("/providers/openrouter")
        && let Some(key) = entry.get("apiKey").and_then(Value::as_str)
    {
        return resolve(key, entry, &env);
    }
    env("OPENROUTER_API_KEY")
        .filter(|key| !key.trim().is_empty())
        .ok_or_else(|| "OpenRouter key missing: log in to Pi or set OPENROUTER_API_KEY".into())
}
pub fn openrouter_key() -> Result<String, String> {
    let home = dirs::home_dir().unwrap_or_default();
    let dir = std::env::var("PI_CODING_AGENT_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|dir| dir.strip_prefix("~/").map(|suffix| home.join(suffix)).unwrap_or_else(|| PathBuf::from(dir)))
        .unwrap_or_else(|| home.join(".pi/agent"));
    from_directory(&dir, |name| std::env::var(name).ok())
}

/// Resolve Mercury FIM settings without copying them into editor config.
/// Environment wins per-variable; missing values are read from project-root
/// `.env-fim`. Values are returned only to the request worker.
pub fn inception_settings(project_root: &Path) -> Result<(String, String), String> {
    let file = std::fs::read_to_string(project_root.join(".env-fim")).unwrap_or_default();
    inception_settings_from(&file, |name| std::env::var(name).ok())
}

/// Resolve the shared Inception base and append a specialized API route.
/// This keeps the existing FIM URL semantics while avoiding duplicated `/v1`.
pub fn inception_route(project_root: &Path, route: &str) -> Result<(String, String), String> {
    let (fim, key) = inception_settings(project_root)?;
    Ok((route_from_fim_endpoint(&fim, route), key))
}

fn route_from_fim_endpoint(fim: &str, route: &str) -> String {
    let base = fim.strip_suffix("/v1/fim/completions").unwrap_or(fim);
    let route = format!("/{}/", route.trim_matches('/'));
    let route = route.trim_end_matches('/');
    format!("{base}{route}")
}

fn inception_settings_from(file: &str, env: impl Fn(&str) -> Option<String>) -> Result<(String, String), String> {
    let mut values = std::collections::HashMap::new();
    for line in file.lines() {
        let Some((name, value)) = line.split_once('=') else { continue };
        let name = name.trim();
        if !matches!(name, "FIM_PROVIDER" | "INCEPTION_API_KEY") {
            continue;
        }
        let mut value = value.trim();
        if value.len() >= 2 && ((value.starts_with('"') && value.ends_with('"')) || (value.starts_with('\'') && value.ends_with('\''))) {
            value = &value[1..value.len() - 1];
        }
        values.insert(name.to_string(), value.to_string());
    }
    let base = env("FIM_PROVIDER")
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| values.get("FIM_PROVIDER").cloned())
        .ok_or("FIM_PROVIDER is missing")?;
    let key = env("INCEPTION_API_KEY")
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .or_else(|| values.get("INCEPTION_API_KEY").cloned())
        .ok_or("INCEPTION_API_KEY is missing")?;
    Ok((fim_endpoint(&base)?, key))
}

fn fim_endpoint(base: &str) -> Result<String, String> {
    let mut base = base.trim().trim_end_matches('/').to_string();
    if base.ends_with("/v1/fim/completions") {
        base.truncate(base.len() - "/v1/fim/completions".len());
    }
    if base.ends_with("/v1") {
        base.truncate(base.len() - 3);
    }
    let url = format!("{}/v1/fim/completions", base.trim_end_matches('/'));
    let parsed = reqwest::Url::parse(&url).map_err(|_| "FIM_PROVIDER is not a valid URL")?;
    if parsed.scheme() != "https" && parsed.host_str() != Some("127.0.0.1") && parsed.host_str() != Some("localhost") {
        return Err("FIM_PROVIDER must use HTTPS".into());
    }
    Ok(url)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuse_pi_key_without_executing_commands_or_mutation() {
        let dir = std::env::temp_dir().join(format!("lumen-inline-credentials-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let auth = dir.join("auth.json");
        let original = r#"{"openrouter":{"type":"api_key","key":"stored-fixture-key"}}"#;
        std::fs::write(&auth, original).unwrap();
        assert_eq!(
            from_directory(&dir, |name| (name == "OPENROUTER_API_KEY").then(|| "environment-fixture".into())).unwrap(),
            "stored-fixture-key"
        );
        assert_eq!(std::fs::read_to_string(&auth).unwrap(), original);
        std::fs::write(&auth, r#"{"openrouter":{"type":"api_key","key":"KEY_NAME","env":{"KEY_NAME":"provider-fixture"}}}"#).unwrap();
        assert_eq!(from_directory(&dir, |_| None).unwrap(), "provider-fixture");
        std::fs::write(&auth, r#"{"openrouter":{"type":"api_key","key":"!touch should-never-exist"}}"#).unwrap();
        assert!(from_directory(&dir, |_| None).is_err());
        assert!(!dir.join("should-never-exist").exists());
        std::fs::remove_file(auth).unwrap();
        assert!(from_directory(&dir, |_| None).is_err());
        assert_eq!(
            from_directory(&dir, |name| (name == "OPENROUTER_API_KEY").then(|| "environment-fixture".into())).unwrap(),
            "environment-fixture"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn fim_endpoint_avoids_duplicate_v1() {
        assert_eq!(fim_endpoint("https://api.inceptionlabs.ai").unwrap(), "https://api.inceptionlabs.ai/v1/fim/completions");
        assert_eq!(fim_endpoint("https://api.inceptionlabs.ai/v1/").unwrap(), "https://api.inceptionlabs.ai/v1/fim/completions");
        assert_eq!(
            fim_endpoint("https://api.inceptionlabs.ai/v1/fim/completions").unwrap(),
            "https://api.inceptionlabs.ai/v1/fim/completions"
        );
        assert!(fim_endpoint("http://example.com").is_err());
    }
    #[test]
    fn fim_environment_overrides_env_file_without_mutating_it() {
        let file = "FIM_PROVIDER=https://file.example/v1\nINCEPTION_API_KEY=file-secret-fixture\n";
        let resolved = inception_settings_from(file, |name| match name {
            "FIM_PROVIDER" => Some("https://env.example".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(resolved.0, "https://env.example/v1/fim/completions");
        assert_eq!(resolved.1, "file-secret-fixture");
        assert_eq!(file, "FIM_PROVIDER=https://file.example/v1\nINCEPTION_API_KEY=file-secret-fixture\n");
    }

    #[test]
    fn edit_endpoint_reuses_provider_base_without_duplicate_v1() {
        for base in ["https://api.inceptionlabs.ai", "https://api.inceptionlabs.ai/v1/"] {
            let fim = fim_endpoint(base).unwrap();
            assert_eq!(route_from_fim_endpoint(&fim, "/v1/edit/completions"), "https://api.inceptionlabs.ai/v1/edit/completions");
        }
    }
}
