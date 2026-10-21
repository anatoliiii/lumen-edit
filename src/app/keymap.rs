//! Keyboard shortcuts: the defaults, the user's overrides from
//! `[keybindings]` in config.toml, matching key presses and searching.
//!
//! A chord is written `Ctrl+Shift+P`. `Ctrl` is the platform's primary
//! modifier, so the same config means Cmd+Shift+P on macOS.

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use eframe::egui::{Event, InputState, Key, Modifiers};

use super::commands::{ALL, Command, Scope};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// Ctrl on Linux and Windows, Cmd on macOS.
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Chord {
    pub fn from_event(key: Key, modifiers: Modifiers) -> Chord {
        Chord { ctrl: modifiers.command, alt: modifiers.alt, shift: modifiers.shift, key }
    }

    /// Parses `Ctrl+Shift+P`, `cmd+alt+t`, `Shift+F12`, `Ctrl++`.
    pub fn parse(text: &str) -> Option<Chord> {
        let text = text.trim();
        let (modifiers, key) = if text == "+" {
            ("", "+")
        } else if let Some(rest) = text.strip_suffix("++") {
            (rest, "+")
        } else {
            text.rsplit_once('+').unwrap_or(("", text))
        };
        let mut chord = Chord { ctrl: false, alt: false, shift: false, key: key_from_name(key.trim())? };
        for part in modifiers.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match modifier_alias(&part.to_lowercase()) {
                Some("ctrl") => chord.ctrl = true,
                Some("alt") => chord.alt = true,
                Some("shift") => chord.shift = true,
                _ => return None,
            }
        }
        Some(chord)
    }

    /// The portable form stored in config.toml: `Ctrl+Alt+Shift+Key`.
    pub fn to_config(self) -> String {
        self.join(["Ctrl", "Alt", "Shift"])
    }

    /// How the chord is shown: `Cmd+Opt+Shift+P` on macOS.
    pub fn label(self) -> String {
        if cfg!(target_os = "macos") { self.join(["Cmd", "Opt", "Shift"]) } else { self.to_config() }
    }

    fn join(self, names: [&str; 3]) -> String {
        let mut out = String::new();
        for (on, name) in [(self.ctrl, names[0]), (self.alt, names[1]), (self.shift, names[2])] {
            if on {
                out.push_str(name);
                out.push('+');
            }
        }
        out.push_str(key_text(self.key));
        out
    }

    /// Keys that type text need Ctrl or Alt, otherwise the shortcut would
    /// eat ordinary typing.
    pub fn types_text(self) -> bool {
        !self.ctrl && !self.alt && !is_special(self.key)
    }

    /// 0 = exact match, 1 = matched ignoring Shift (needed to type some
    /// symbols, e.g. `+` on a US keyboard), `None` = no match.
    fn match_level(self, key: Key, modifiers: Modifiers) -> Option<u8> {
        if key != self.key || modifiers.command != self.ctrl || modifiers.alt != self.alt {
            return None;
        }
        // On macOS the physical Control key is not Cmd: leave it to the
        // terminal and text fields.
        if cfg!(target_os = "macos") && modifiers.ctrl {
            return None;
        }
        if modifiers.shift == self.shift {
            Some(0)
        } else if modifiers.shift && !self.shift && is_symbol(self.key) {
            Some(1)
        } else {
            None
        }
    }
}

fn modifier_alias(name: &str) -> Option<&'static str> {
    Some(match name {
        "ctrl" | "control" | "cmd" | "command" | "mod" | "super" | "meta" => "ctrl",
        "alt" | "option" | "opt" => "alt",
        "shift" | "⇧" => "shift",
        _ => return None,
    })
}

fn key_from_name(name: &str) -> Option<Key> {
    if name.is_empty() {
        return None;
    }
    Key::from_name(name).or_else(|| Key::ALL.iter().copied().find(|k| k.name().eq_ignore_ascii_case(name)))
}

fn key_text(key: Key) -> &'static str {
    match key {
        Key::Minus => "-",
        Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight => key.name(),
        _ => key.symbol_or_name(),
    }
}

fn is_symbol(key: Key) -> bool {
    matches!(
        key,
        Key::Colon
            | Key::Comma
            | Key::Minus
            | Key::Period
            | Key::Plus
            | Key::Equals
            | Key::Semicolon
            | Key::Backslash
            | Key::Slash
            | Key::Pipe
            | Key::Questionmark
            | Key::Exclamationmark
            | Key::OpenBracket
            | Key::CloseBracket
            | Key::OpenCurlyBracket
            | Key::CloseCurlyBracket
            | Key::Backtick
            | Key::Quote
    )
}

/// Keys that never type text: function keys, navigation, Enter, Escape...
fn is_special(key: Key) -> bool {
    key_text(key).chars().count() > 1 && !matches!(key, Key::Space) && !is_symbol(key)
}

/// Default shortcuts. Editor keys that are not listed here (arrows, Home/End,
/// Backspace, Tab, Escape, copy/paste) are fixed.
pub fn defaults(command: Command) -> &'static [&'static str] {
    match command {
        Command::QuickOpen => &["Ctrl+P"],
        Command::CommandPalette => &["Ctrl+Shift+P"],
        Command::GotoLine => &["Ctrl+G"],
        Command::OpenFolder => &["Ctrl+O"],
        Command::ConnectSsh => &["Ctrl+Alt+O"],
        Command::NewFile => &["Ctrl+N"],
        Command::Teleport => &["Ctrl+Alt+T"],
        Command::Save => &["Ctrl+S"],
        Command::SaveAll => &["Ctrl+Alt+S"],
        Command::CloseTab => &["Ctrl+W"],
        Command::NextTab => &["Ctrl+Tab", "Ctrl+PageDown"],
        Command::PrevTab => &["Ctrl+Shift+Tab", "Ctrl+PageUp"],
        Command::ToggleSidebar => &["Ctrl+B"],
        Command::ShowExplorer => &["Ctrl+Shift+E"],
        Command::ShowSearch => &["Ctrl+Shift+F"],
        Command::ShowGit => &["Ctrl+Shift+G"],
        Command::ShowRun => &["Ctrl+Shift+R"],
        Command::TogglePanel => &["Ctrl+J"],
        Command::ToggleTerminal => &["Ctrl+`"],
        Command::NewTerminal => &["Ctrl+Shift+`"],
        Command::KillTerminal => &["Shift+F5"],
        Command::ShowProblems => &["Ctrl+Shift+M"],
        Command::RunDefault => &["F5"],
        Command::RunTask => &["Ctrl+Alt+R"],
        Command::Build => &["Ctrl+Shift+B"],
        Command::RerunLast => &["Ctrl+F5"],
        Command::MarkdownPreview => &["Ctrl+Alt+P"],
        Command::Agent => &["Ctrl+Shift+A"],
        Command::AgentSendSelection => &["Ctrl+Alt+A"],
        Command::ZoomIn => &["Ctrl+=", "Ctrl++"],
        Command::ZoomOut => &["Ctrl+-"],
        Command::ZoomReset => &["Ctrl+0"],
        Command::Settings => &["Ctrl+,"],
        Command::Keybindings => &["Ctrl+Alt+K"],
        Command::Quit => &["Ctrl+Q"],
        Command::GitCommit => &["Ctrl+Enter"],
        // Editor.
        Command::Find => &["Ctrl+F"],
        Command::Replace => &["Ctrl+H"],
        Command::FindNext => &["F3"],
        Command::FindPrevious => &["Shift+F3"],
        Command::GotoDefinition => &["F12"],
        Command::References => &["Shift+F12"],
        Command::Rename => &["F2"],
        Command::CodeActions => &["Ctrl+."],
        Command::Format => &["Ctrl+Shift+I"],
        Command::TriggerSuggest => &["Ctrl+Space"],
        Command::ShowHover => &["Ctrl+K"],
        Command::Undo => &["Ctrl+Z"],
        Command::Redo => &["Ctrl+Y", "Ctrl+Shift+Z"],
        Command::SelectAll => &["Ctrl+A"],
        Command::SelectLine => &["Ctrl+L"],
        Command::AddNextOccurrence => &["Ctrl+D"],
        Command::AddCursorAbove => &["Ctrl+Alt+Up", "Ctrl+Shift+Up"],
        Command::AddCursorBelow => &["Ctrl+Alt+Down", "Ctrl+Shift+Down"],
        Command::MoveLinesUp => &["Alt+Up"],
        Command::MoveLinesDown => &["Alt+Down"],
        Command::CopyLinesUp => &["Alt+Shift+Up"],
        Command::CopyLinesDown => &["Alt+Shift+Down"],
        Command::DeleteLines => &["Ctrl+Shift+K", "Shift+Delete"],
        Command::InsertLineBelow => &["Ctrl+Enter"],
        Command::InsertLineAbove => &["Ctrl+Shift+Enter"],
        Command::ToggleComment => &["Ctrl+/"],
        Command::Indent => &["Ctrl+]"],
        Command::Outdent => &["Ctrl+["],
        Command::ScrollLineUp => &["Ctrl+Up"],
        Command::ScrollLineDown => &["Ctrl+Down"],
        _ => &[],
    }
}

/// Whether shortcuts of the two scopes can fire on the same key press.
/// Global ones are taken before any widget sees the keys.
pub fn scopes_overlap(a: Scope, b: Scope) -> bool {
    a == b || a == Scope::Global || b == Scope::Global
}

pub struct Keymap {
    /// Chords per command, in the order of `commands::ALL`.
    bindings: Vec<(Command, Vec<Chord>)>,
    /// Entries of `[keybindings]` that could not be used.
    pub problems: Vec<String>,
}

impl Keymap {
    /// Defaults with `overrides` (command id -> chords) on top. An override
    /// replaces all default chords of its command; `[]` unbinds it.
    pub fn new(overrides: &BTreeMap<String, Vec<String>>) -> Keymap {
        let mut problems = Vec::new();
        for id in overrides.keys() {
            if Command::from_id(id).is_none() {
                problems.push(format!("[keybindings] {id}: unknown command"));
            }
        }
        let bindings = ALL
            .iter()
            .map(|&command| {
                let texts: Vec<&str> = match overrides.get(&command.id()) {
                    Some(list) => list.iter().map(String::as_str).collect(),
                    None => defaults(command).to_vec(),
                };
                let chords = texts
                    .into_iter()
                    .filter_map(|text| {
                        let chord = Chord::parse(text);
                        if chord.is_none() {
                            problems.push(format!("[keybindings] {}: cannot read \"{text}\"", command.id()));
                        }
                        chord
                    })
                    .collect();
                (command, chords)
            })
            .collect();
        Keymap { bindings, problems }
    }

    pub fn chords(&self, command: Command) -> &[Chord] {
        self.bindings.iter().find(|(c, _)| *c == command).map_or(&[], |(_, chords)| chords)
    }

    /// The first chord of a command, as shown in menus.
    pub fn label(&self, command: Command) -> Option<String> {
        self.chords(command).first().map(|c| c.label())
    }

    /// The command bound to a key press in `scope`: exact matches first, then
    /// the logical key over the physical one.
    pub fn lookup(&self, scope: Scope, key: Key, physical: Option<Key>, modifiers: Modifiers) -> Option<Command> {
        let keys = std::iter::once(key).chain(physical.filter(|p| *p != key));
        let mut best: Option<(u8, Command)> = None;
        for (rank, key) in keys.enumerate() {
            for (command, chords) in &self.bindings {
                if command.scope() != scope {
                    continue;
                }
                for chord in chords {
                    if let Some(level) = chord.match_level(key, modifiers) {
                        let level = level + 2 * rank as u8;
                        if best.is_none_or(|(b, _)| level < b) {
                            best = Some((level, *command));
                        }
                    }
                }
            }
        }
        best.map(|(_, command)| command)
    }

    /// Other commands that react to `chord` where `command` would.
    pub fn conflicts(&self, command: Command, chord: Chord) -> Vec<Command> {
        self.bindings
            .iter()
            .filter(|(c, chords)| *c != command && scopes_overlap(c.scope(), command.scope()) && chords.contains(&chord))
            .map(|(c, _)| *c)
            .collect()
    }
}

static CURRENT: OnceLock<RwLock<Arc<Keymap>>> = OnceLock::new();

fn cell() -> &'static RwLock<Arc<Keymap>> {
    CURRENT.get_or_init(|| RwLock::new(Arc::new(Keymap::new(&BTreeMap::new()))))
}

/// The keymap in effect.
pub fn current() -> Arc<Keymap> {
    cell().read().map(|k| k.clone()).unwrap_or_else(|e| e.into_inner().clone())
}

pub fn set(keymap: Keymap) {
    for problem in &keymap.problems {
        eprintln!("lumen-edit: {problem}");
    }
    match cell().write() {
        Ok(mut current) => *current = Arc::new(keymap),
        Err(e) => *e.into_inner() = Arc::new(keymap),
    }
}

/// Removes the key presses bound in `scope` from this frame's input and
/// returns their commands.
pub fn consume(input: &mut InputState, scope: Scope) -> Vec<Command> {
    let keymap = current();
    let mut triggered = Vec::new();
    input.events.retain(|event| {
        if let Event::Key { key, physical_key, pressed: true, modifiers, .. } = event
            && let Some(command) = keymap.lookup(scope, *key, *physical_key, *modifiers)
        {
            triggered.push(command);
            return false;
        }
        true
    });
    triggered
}

/// `cmd + Shift+p` -> `ctrl+shift+p`: lower case, modifier aliases resolved,
/// no spaces around `+`.
fn normalize(query: &str) -> String {
    // `ё` is often typed as `е`.
    let mut text = query.trim().to_lowercase().replace('\u{451}', "\u{435}");
    while text.contains(" +") || text.contains("+ ") {
        text = text.replace(" +", "+").replace("+ ", "+");
    }
    text.split(' ')
        .map(|word| {
            if !word.contains('+') || word == "+" {
                return word.to_string();
            }
            let (head, plus) = match word.strip_suffix("++") {
                Some(head) => (head, true),
                None => (word, false),
            };
            let mut parts: Vec<String> = head.split('+').map(|p| modifier_alias(p).unwrap_or(p).to_string()).collect();
            if plus {
                parts.push("+".into());
            }
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `command` matches a search: its name in English or Russian, its
/// id, or its keys (`ctrl+shift+p` finds exactly that chord, `f5` every
/// chord with F5).
pub fn matches_query(keymap: &Keymap, command: Command, query: &str) -> bool {
    let query = normalize(query);
    if query.is_empty() {
        return true;
    }
    let chords = keymap.chords(command);
    if !query.contains(' ')
        && query.len() > 1
        && query.contains('+')
        && let Some(chord) = Chord::parse(&query)
    {
        return chords.contains(&chord);
    }
    let english = command.label_key();
    let mut haystack = format!("{english} {} {} {}", crate::i18n::russian_of(english), command.id(), command.id().replace('_', " "));
    for chord in chords {
        haystack.push(' ');
        haystack.push_str(&chord.to_config());
        haystack.push(' ');
        haystack.push_str(&chord.label());
    }
    let haystack = normalize(&haystack);
    query.split(' ').filter(|w| !w.is_empty()).all(|word| haystack.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_parses_and_round_trips() {
        for &command in ALL {
            for text in defaults(command) {
                let chord = Chord::parse(text).unwrap_or_else(|| panic!("{command:?}: {text}"));
                assert_eq!(Chord::parse(&chord.to_config()), Some(chord), "{text}");
            }
        }
    }

    #[test]
    fn parses_aliases_and_plus() {
        let chord = |ctrl, alt, shift, key| Some(Chord { ctrl, alt, shift, key });
        assert_eq!(Chord::parse("cmd+shift+p"), chord(true, false, true, Key::P));
        assert_eq!(Chord::parse("Option + T"), chord(false, true, false, Key::T));
        assert_eq!(Chord::parse("Ctrl++"), chord(true, false, false, Key::Plus));
        assert_eq!(Chord::parse("shift+f12"), chord(false, false, true, Key::F12));
        assert_eq!(Chord::parse("ctrl+pagedown"), chord(true, false, false, Key::PageDown));
        assert_eq!(Chord::parse("Ctrl+Shift"), None);
        assert_eq!(Chord::parse("Hyper+P"), None);
        assert_eq!(Chord::parse("Ctrl+`").unwrap().to_config(), "Ctrl+`");
        assert_eq!(Chord::parse("ctrl+minus").unwrap().to_config(), "Ctrl+-");
    }

    #[test]
    fn defaults_do_not_conflict() {
        let keymap = Keymap::new(&BTreeMap::new());
        assert!(keymap.problems.is_empty(), "{:?}", keymap.problems);
        for &command in ALL {
            for &chord in keymap.chords(command) {
                assert_eq!(keymap.conflicts(command, chord), vec![], "{command:?} {}", chord.to_config());
            }
        }
    }

    #[test]
    fn lookup_prefers_exact_modifiers() {
        let keymap = Keymap::new(&BTreeMap::new());
        let m = |command, shift| Modifiers {
            command,
            ctrl: command && !cfg!(target_os = "macos"),
            mac_cmd: command && cfg!(target_os = "macos"),
            shift,
            alt: false,
        };
        assert_eq!(keymap.lookup(Scope::Global, Key::P, None, m(true, true)), Some(Command::CommandPalette));
        assert_eq!(keymap.lookup(Scope::Global, Key::P, None, m(true, false)), Some(Command::QuickOpen));
        assert_eq!(keymap.lookup(Scope::Global, Key::Backtick, None, m(true, true)), Some(Command::NewTerminal));
        // `+` needs Shift on a US keyboard.
        assert_eq!(keymap.lookup(Scope::Global, Key::Plus, None, m(true, true)), Some(Command::ZoomIn));
        // Ctrl+Shift+D is not Ctrl+D.
        assert_eq!(keymap.lookup(Scope::Editor, Key::D, None, m(true, true)), None);
        // A non-Latin layout: the logical key is unknown, the physical one is P.
        assert_eq!(keymap.lookup(Scope::Global, Key::Q, Some(Key::P), m(true, false)), Some(Command::Quit));
        assert_eq!(keymap.lookup(Scope::Global, Key::F20, Some(Key::P), m(true, false)), Some(Command::QuickOpen));
        assert_eq!(keymap.lookup(Scope::Editor, Key::Enter, None, m(true, false)), Some(Command::InsertLineBelow));
        assert_eq!(keymap.lookup(Scope::CommitMessage, Key::Enter, None, m(true, false)), Some(Command::GitCommit));
    }

    #[test]
    fn overrides_replace_and_unbind() {
        let overrides = BTreeMap::from([
            ("quick_open".to_string(), vec!["Ctrl+E".to_string()]),
            ("quit".to_string(), vec![]),
            ("nope".to_string(), vec!["Ctrl+X".to_string()]),
            ("save".to_string(), vec!["Ctrl+Bogus".to_string()]),
        ]);
        let keymap = Keymap::new(&overrides);
        assert_eq!(keymap.chords(Command::QuickOpen), [Chord::parse("Ctrl+E").unwrap()]);
        assert!(keymap.chords(Command::Quit).is_empty());
        assert!(keymap.chords(Command::Save).is_empty());
        assert_eq!(keymap.problems.len(), 2, "{:?}", keymap.problems);
        assert_eq!(keymap.chords(Command::CommandPalette), [Chord::parse("Ctrl+Shift+P").unwrap()]);
    }

    #[test]
    fn search_by_name_in_both_languages_id_and_keys() {
        let keymap = Keymap::new(&BTreeMap::new());
        let find = |query: &str| ALL.iter().copied().filter(|c| matches_query(&keymap, *c, query)).collect::<Vec<_>>();
        assert!(find("palette").contains(&Command::CommandPalette));
        let russian = crate::i18n::russian_of("Command Palette");
        assert_ne!(russian, "Command Palette");
        assert!(find(russian).contains(&Command::CommandPalette));
        assert!(find(&russian.to_uppercase()).contains(&Command::CommandPalette));
        assert!(find(russian.split(' ').next().unwrap()).contains(&Command::CommandPalette));
        assert!(find("command_palette").contains(&Command::CommandPalette));
        assert_eq!(find("ctrl+shift+p"), vec![Command::CommandPalette]);
        assert_eq!(find("Cmd + Shift + P"), vec![Command::CommandPalette]);
        assert_eq!(find("ctrl+="), vec![Command::ZoomIn]);
        assert_eq!(find("ctrl++"), vec![Command::ZoomIn]);
        let f5 = find("f5");
        for command in [Command::RunDefault, Command::RerunLast, Command::KillTerminal] {
            assert!(f5.contains(&command), "{command:?} in {f5:?}");
        }
        assert!(find("ctrl+shift").contains(&Command::CommandPalette));
        assert!(find("").len() == ALL.len());
        assert!(find("zzzz").is_empty());
    }
}
