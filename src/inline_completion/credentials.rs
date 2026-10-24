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
}
