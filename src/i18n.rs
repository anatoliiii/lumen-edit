//! Interface language. Text in the code is English and doubles as the key;
//! translations live in `locales/<lang>.toml` and are built into the binary.

use std::{
    collections::HashMap,
    fmt::Display,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En = 0,
    Ru = 1,
}

static LANG: AtomicU8 = AtomicU8::new(Lang::En as u8);
static RU: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();

/// Values of the `language` setting.
pub const SETTINGS: &[(&str, &str)] = &[("auto", "Auto"), ("en", "English"), ("ru", "Русский")];

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if LANG.load(Ordering::Relaxed) == Lang::Ru as u8 { Lang::Ru } else { Lang::En }
}

/// `"auto"` follows the system locale (LC_ALL, LC_MESSAGES, LANG).
pub fn from_setting(setting: &str) -> Lang {
    match setting {
        "en" => Lang::En,
        "ru" => Lang::Ru,
        _ => ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
            .map_or(Lang::En, |locale| if locale.starts_with("ru") { Lang::Ru } else { Lang::En }),
    }
}

fn russian() -> &'static HashMap<&'static str, &'static str> {
    RU.get_or_init(|| {
        let table: toml::Table =
            toml::from_str(include_str!("../locales/ru.toml")).unwrap_or_else(|e| panic!("locales/ru.toml is invalid TOML: {e}"));
        table
            .into_iter()
            .filter_map(|(key, value)| {
                let value = value.as_str()?.to_string();
                Some((&*Box::leak(key.into_boxed_str()), &*Box::leak(value.into_boxed_str())))
            })
            .collect()
    })
}

/// The text in the current language.
pub fn tr(text: &'static str) -> &'static str {
    match current() {
        Lang::En => text,
        Lang::Ru => russian().get(text).copied().unwrap_or(text),
    }
}

/// The Russian translation of an English text, whatever the current
/// language (searching commands by either name).
pub fn russian_of(text: &'static str) -> &'static str {
    russian().get(text).copied().unwrap_or(text)
}

/// Substitutes `{name}` placeholders; unknown braces stay as they are.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}').and_then(|close| args.iter().find(|(name, _)| *name == &after[..close]).map(|arg| (close, arg))) {
            Some((close, (_, value))) => {
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `t!("Text")` translates; `t!("Saved {name}", name = value)` also fills in
/// placeholders and returns a `String`.
macro_rules! t {
    ($text:literal) => {
        $crate::i18n::tr($text)
    };
    ($text:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($text), &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

/// `t_key!("Text")` marks an English text for translation without
/// translating it; `i18n::tr` turns it into the current language later.
macro_rules! t_key {
    ($text:literal) => {
        $text
    };
}

/// A long text from `locales/<lang>/<file>` (help pages, templates).
macro_rules! t_file {
    ($file:literal) => {
        match $crate::i18n::current() {
            $crate::i18n::Lang::En => include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales/en/", $file)),
            $crate::i18n::Lang::Ru => include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales/ru/", $file)),
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_known_placeholders_only() {
        let count = 3;
        assert_eq!(fill("{count} files in {dir} {x}", &[("count", &count), ("dir", &"src")]), "3 files in src {x}");
        assert_eq!(fill("{ json }", &[]), "{ json }");
    }

    #[test]
    fn every_translation_keeps_the_placeholders() {
        let placeholders = |s: &str| {
            let mut found: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(name, _)| name.to_string())).collect();
            found.sort();
            found
        };
        for (english, russian) in russian() {
            assert_eq!(placeholders(english), placeholders(russian), "{english:?} -> {russian:?}");
        }
    }

    /// Every `t!("…")` and `t_key!("…")` in the sources has a Russian translation, and no
    /// Cyrillic text is left outside the catalog.
    #[test]
    fn sources_are_fully_translated() {
        let literal =
            regex::Regex::new(r#"\bt(?:_key)?!\(\s*"((?:[^"\\]|\\.)*)""#).unwrap_or_else(|e| panic!("invalid literal regex: {e}"));
        let cyrillic = regex::Regex::new(r#""[^"\n]*[А-Яа-яЁё][^"\n]*""#).unwrap_or_else(|e| panic!("invalid cyrillic regex: {e}"));
        let mut missing = Vec::new();
        let mut untranslated = Vec::new();
        let mut used = std::collections::HashSet::new();
        let mut stack = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read_dir failed: {e}")).flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") || path.ends_with("i18n.rs") {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read failed: {e}"));
                for capture in literal.captures_iter(&source) {
                    let key: String = toml::from_str::<toml::Table>(&format!("k = \"{}\"", &capture[1]))
                        .unwrap_or_else(|e| panic!("parse failed: {e}"))["k"]
                        .as_str()
                        .unwrap_or_else(|| panic!("expected string value"))
                        .into();
                    if !russian().contains_key(key.as_str()) {
                        missing.push(format!("{}: {key}", path.display()));
                    }
                    used.insert(key);
                }
                for (number, line) in source.lines().enumerate() {
                    if !line.trim_start().starts_with("//") && cyrillic.is_match(line) {
                        untranslated.push(format!("{}:{}: {}", path.display(), number + 1, line.trim()));
                    }
                }
            }
        }
        assert!(missing.is_empty(), "no Russian translation for:\n{}", missing.join("\n"));
        assert!(untranslated.is_empty(), "Cyrillic text outside t!():\n{}", untranslated.join("\n"));
        let mut unused: Vec<&str> = russian().keys().copied().filter(|key| !used.contains(*key)).collect();
        unused.sort();
        assert!(unused.is_empty(), "translations no longer used in the code:\n{}", unused.join("\n"));
    }
}
