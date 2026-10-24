//! Keyboard shortcuts: the defaults, the user's overrides from
//! `[keybindings]` in config.toml, matching key presses and searching.
//!
//! In config.toml a chord is written `Ctrl+Shift+P`. `Ctrl` is the
//! platform's primary modifier, so the same config means Cmd+Shift+P on
//! macOS; the Control key of a Mac is `MacCtrl` (plain Ctrl elsewhere).
//! On screen and in search macOS chords read `Cmd+Shift+P`, `Ctrl+``.

use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

use eframe::egui::{Event, InputState, Key, Modifiers};

use super::commands::{ALL, Command, Scope};

const MAC: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Chord {
    /// The primary modifier: Ctrl on Linux and Windows, Cmd on macOS.
    pub ctrl: bool,
    /// The Control key of a Mac, which is not the primary modifier there.
    pub mac_ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: Key,
}

impl Chord {
    pub const fn plain(key: Key) -> Chord {
        Chord { ctrl: false, mac_ctrl: false, alt: false, shift: false, key }
    }

    pub fn from_event(key: Key, modifiers: Modifiers) -> Chord {
        Chord { ctrl: modifiers.command, mac_ctrl: MAC && modifiers.ctrl, alt: modifiers.alt, shift: modifiers.shift, key }
    }

    /// Parses the config form: `Ctrl+Shift+P`, `MacCtrl+``, `Shift+F12`, `Ctrl++`.
    pub fn parse(text: &str) -> Option<Chord> {
        Self::parse_with(text, false)
    }

    /// Parses a chord as shown on screen: on macOS `Cmd` is the primary
    /// modifier and `Ctrl` the Control key.
    pub fn parse_label(text: &str) -> Option<Chord> {
        Self::parse_with(text, MAC)
    }

    fn parse_with(text: &str, mac_names: bool) -> Option<Chord> {
        let text = text.trim();
        let (modifiers, key) = if text == "+" {
            ("", "+")
        } else if let Some(rest) = text.strip_suffix("++") {
            (rest, "+")
        } else {
            text.rsplit_once('+').unwrap_or(("", text))
        };
        let mut chord = Chord::plain(key_from_name(key.trim()).filter(|k| !is_modifier(*k))?);
        for part in modifiers.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_lowercase().as_str() {
                "cmd" | "command" | "mod" | "super" | "meta" => chord.ctrl = true,
                "ctrl" | "control" if mac_names => chord.mac_ctrl = true,
                "ctrl" | "control" => chord.ctrl = true,
                "macctrl" if MAC => chord.mac_ctrl = true,
                "macctrl" => chord.ctrl = true,
                "alt" | "option" | "opt" => chord.alt = true,
                "shift" => chord.shift = true,
                _ => return None,
            }
        }
        Some(chord)
    }

    /// The portable form stored in config.toml.
    pub fn to_config(self) -> String {
        join(&[(self.ctrl, "Ctrl"), (self.mac_ctrl, "MacCtrl"), (self.alt, "Alt"), (self.shift, "Shift")], self.key)
    }

    /// How the chord is shown: `Cmd+Shift+P` or `Ctrl+Opt+T` on macOS.
    pub fn label(self) -> String {
        let mut text = modifiers_label(self.ctrl, self.mac_ctrl, self.alt, self.shift);
        text.push_str(key_text(self.key));
        text
    }

    /// Keys that type text need a modifier other than Shift, otherwise the
    /// shortcut would eat ordinary typing.
    pub fn types_text(self) -> bool {
        !self.ctrl && !self.mac_ctrl && !self.alt && !is_special(self.key)
    }

    /// 0 = exact match, 1 = matched ignoring Shift (needed to type some
    /// symbols, e.g. `+` on a US keyboard), `None` = no match.
    fn match_level(self, key: Key, modifiers: Modifiers) -> Option<u8> {
        if key != self.key || modifiers.command != self.ctrl || modifiers.alt != self.alt {
            return None;
        }
        if MAC && modifiers.ctrl != self.mac_ctrl {
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

/// Held modifiers as shown before the key: `Cmd+Shift+`.
pub fn held_label(modifiers: Modifiers) -> String {
    modifiers_label(modifiers.command, MAC && modifiers.ctrl, modifiers.alt, modifiers.shift)
}

fn modifiers_label(ctrl: bool, mac_ctrl: bool, alt: bool, shift: bool) -> String {
    let names: &[(bool, &str)] = if MAC {
        &[(mac_ctrl, "Ctrl"), (ctrl, "Cmd"), (alt, "Opt"), (shift, "Shift")]
    } else {
        &[(ctrl, "Ctrl"), (alt, "Alt"), (shift, "Shift")]
    };
    names.iter().filter(|(on, _)| *on).map(|(_, name)| format!("{name}+")).collect()
}

fn join(modifiers: &[(bool, &str)], key: Key) -> String {
    let mut text: String = modifiers.iter().filter(|(on, _)| *on).map(|(_, name)| format!("{name}+")).collect();
    text.push_str(key_text(key));
    text
}

/// Shift, Ctrl, Alt and Cmd themselves: egui reports them as keys too.
pub fn is_modifier(key: Key) -> bool {
    matches!(
        key,
        Key::ShiftLeft
            | Key::ShiftRight
            | Key::ControlLeft
            | Key::ControlRight
            | Key::AltLeft
            | Key::AltRight
            | Key::SuperLeft
            | Key::SuperRight
    )
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
/// Backspace, Tab, Escape) are fixed.
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
        // macOS keeps Cmd+Tab, Cmd+`, Cmd+Space and Cmd+H for itself: the
        // Mac defaults use the Control key there, like VS Code.
        Command::NextTab if MAC => &["MacCtrl+Tab", "Ctrl+PageDown"],
        Command::NextTab => &["Ctrl+Tab", "Ctrl+PageDown"],
        Command::PrevTab if MAC => &["MacCtrl+Shift+Tab", "Ctrl+PageUp"],
        Command::PrevTab => &["Ctrl+Shift+Tab", "Ctrl+PageUp"],
        Command::ToggleSidebar => &["Ctrl+B"],
        Command::ShowExplorer => &["Ctrl+Shift+E"],
        Command::ShowSearch => &["Ctrl+Shift+F"],
        Command::ShowGit => &["Ctrl+Shift+G"],
        Command::ShowRun => &["Ctrl+Shift+R"],
        Command::TogglePanel => &["Ctrl+J"],
        Command::ToggleTerminal if MAC => &["MacCtrl+`"],
        Command::ToggleTerminal => &["Ctrl+`"],
        Command::NewTerminal if MAC => &["MacCtrl+Shift+`"],
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
        Command::Replace if MAC => &["Ctrl+Alt+F"],
        Command::Replace => &["Ctrl+H"],
        Command::FindNext => &["F3"],
        Command::FindPrevious => &["Shift+F3"],
        Command::GotoDefinition => &["F12"],
        Command::References => &["Shift+F12"],
        Command::Rename => &["F2"],
        Command::CodeActions => &["Ctrl+."],
        Command::Format => &["Ctrl+Shift+I"],
        Command::TriggerInlineCompletion => &["Ctrl+Alt+Space"],
        Command::TriggerSuggest if MAC => &["MacCtrl+Space"],
        Command::TriggerSuggest => &["Ctrl+Space"],
        Command::ShowHover => &["Ctrl+K"],
        Command::Undo => &["Ctrl+Z"],
        Command::Redo => &["Ctrl+Y", "Ctrl+Shift+Z"],
        Command::Cut => &["Ctrl+X"],
        Command::Copy => &["Ctrl+C"],
        Command::Paste => &["Ctrl+V"],
        // Ctrl+C copies a selection (else interrupts); with Shift as in
        // other terminals.
        Command::TerminalCopy => &["Ctrl+C", "Ctrl+Shift+C"],
        Command::TerminalPaste => &["Ctrl+V", "Ctrl+Shift+V"],
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
    /// replaces all default chords of its command; `[]` unbinds it. An
    /// override none of whose chords can be read keeps the defaults.
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
                let parse_defaults = || defaults(command).iter().filter_map(|text| Chord::parse(text)).collect();
                let chords = match overrides.get(&command.id()) {
                    Some(list) => {
                        let chords: Vec<Chord> = list
                            .iter()
                            .filter_map(|text| {
                                let chord = Chord::parse(text);
                                if chord.is_none() {
                                    problems.push(format!("[keybindings] {}: cannot read \"{text}\"", command.id()));
                                }
                                chord
                            })
                            .collect();
                        if chords.is_empty() && !list.is_empty() { parse_defaults() } else { chords }
                    }
                    None => parse_defaults(),
                };
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

/// The chord behind a clipboard event: egui turns a plain Ctrl+C / Ctrl+X /
/// Ctrl+V (Cmd on macOS) into Copy / Cut / Paste without a key press.
pub fn clipboard_chord(event: &Event) -> Option<Chord> {
    let key = match event {
        Event::Copy => Key::C,
        Event::Cut => Key::X,
        Event::Paste(_) => Key::V,
        _ => return None,
    };
    Some(Chord { ctrl: true, ..Chord::plain(key) })
}

/// The modifiers egui reports with a clipboard event.
fn clipboard_modifiers() -> Modifiers {
    Modifiers { command: true, mac_cmd: MAC, ctrl: !MAC, ..Modifiers::NONE }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardAction {
    /// Copy, cut or paste as usual.
    Native,
    /// The keys belong to another command.
    Run(Command),
    /// No command has these keys any more.
    Unbound,
}

/// What a clipboard event (Ctrl+C / Ctrl+X / Ctrl+V) does in `scope`, by
/// the keys of the commands that normally copy, cut and paste there.
pub fn clipboard_action(keymap: &Keymap, scope: Scope, event: &Event) -> Option<ClipboardAction> {
    let chord = clipboard_chord(event)?;
    let native = match (scope, chord.key) {
        (Scope::Editor, Key::C) => Some(Command::Copy),
        (Scope::Editor, Key::X) => Some(Command::Cut),
        (Scope::Editor, Key::V) => Some(Command::Paste),
        (Scope::Terminal, Key::C) => Some(Command::TerminalCopy),
        (Scope::Terminal, Key::V) => Some(Command::TerminalPaste),
        _ => None,
    };
    Some(match keymap.lookup(scope, chord.key, None, clipboard_modifiers()) {
        Some(command) if Some(command) == native => ClipboardAction::Native,
        Some(command) => ClipboardAction::Run(command),
        None => ClipboardAction::Unbound,
    })
}

const PASTE_PENDING: &str = "lumen-paste-pending";
const PASTE_NOW: &str = "lumen-paste-now";

/// Asks egui for the clipboard text: it arrives next frame as a paste
/// event, which must then not be taken for Ctrl+V.
pub fn request_paste(ctx: &eframe::egui::Context) {
    ctx.data_mut(|d| d.insert_temp(eframe::egui::Id::unique(PASTE_PENDING), true));
    ctx.send_viewport_cmd(eframe::egui::ViewportCommand::RequestPaste);
    // Make sure the next frame comes, or the text waits for the next input.
    ctx.request_repaint();
}

/// Called first thing every frame: a paste asked for last frame is the one
/// arriving now.
pub fn begin_frame(ctx: &eframe::egui::Context) {
    ctx.data_mut(|d| {
        let pending = d.remove_temp::<bool>(eframe::egui::Id::unique(PASTE_PENDING)).unwrap_or(false);
        d.insert_temp(eframe::egui::Id::unique(PASTE_NOW), pending);
    });
}

/// Whether this frame's paste event was asked for by a command (and so is
/// not Ctrl+V).
pub fn requested_paste(ctx: &eframe::egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(eframe::egui::Id::unique(PASTE_NOW))).unwrap_or(false)
}

/// Removes the key presses bound in `scope` from this frame's input and
/// returns their commands. With `clipboard`, Ctrl+C / Ctrl+X / Ctrl+V bound
/// to a command of `scope` run it instead of copying or pasting.
pub fn consume(input: &mut InputState, scope: Scope, clipboard: bool) -> Vec<Command> {
    consume_with(&current(), input, scope, clipboard)
}

fn consume_with(keymap: &Keymap, input: &mut InputState, scope: Scope, clipboard: bool) -> Vec<Command> {
    let mut triggered = Vec::new();
    input.events.retain(|event| {
        let command = match event {
            Event::Key { key, physical_key, pressed: true, modifiers, .. } => keymap.lookup(scope, *key, *physical_key, *modifiers),
            _ if clipboard => clipboard_chord(event).and_then(|chord| keymap.lookup(scope, chord.key, None, clipboard_modifiers())),
            _ => None,
        };
        match command {
            Some(command) => {
                triggered.push(command);
                false
            }
            None => true,
        }
    });
    triggered
}

/// `Cmd + Shift+P` -> `cmd+shift+p`: lower case, no spaces around `+`,
/// modifier names as `label()` writes them (`option` -> `alt`; `cmd` is
/// `ctrl` off macOS).
fn normalize(query: &str) -> String {
    // `ё` is often typed as `е`.
    let mut text = query.trim().to_lowercase().replace('\u{451}', "\u{435}");
    while text.contains(" +") || text.contains("+ ") {
        text = text.replace(" +", "+").replace("+ ", "+");
    }
    let alias = |part: &str| -> String {
        match part {
            "opt" | "option" => "alt",
            "command" | "super" | "meta" | "mod" if MAC => "cmd",
            "cmd" | "command" | "super" | "meta" | "mod" | "control" if !MAC => "ctrl",
            "control" => "ctrl",
            other => other,
        }
        .to_string()
    };
    text.split(' ')
        .map(|word| {
            if !word.contains('+') || word == "+" {
                return word.to_string();
            }
            let (head, plus) = match word.strip_suffix("++") {
                Some(head) => (head, true),
                None => (word, false),
            };
            let mut parts: Vec<String> = head.split('+').map(alias).collect();
            if plus {
                parts.push("+".into());
            }
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `command` matches a search: its name in English or Russian, its
/// id, or its keys as shown (`cmd+shift+p` finds exactly that chord, `f5`
/// every chord with F5).
pub fn matches_query(keymap: &Keymap, command: Command, query: &str) -> bool {
    let query = normalize(query);
    if query.is_empty() {
        return true;
    }
    let chords = keymap.chords(command);
    if !query.contains(' ')
        && query.len() > 1
        && query.contains('+')
        && let Some(chord) = Chord::parse_label(&query)
    {
        return chords.contains(&chord);
    }
    let english = command.label_key();
    let mut haystack = format!("{english} {} {} {}", crate::i18n::russian_of(english), command.id(), command.id().replace('_', " "));
    for chord in chords {
        haystack.push(' ');
        haystack.push_str(&chord.label());
        haystack.push(' ');
        haystack.push_str(chord.key.name());
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
        let chord = |ctrl, alt, shift, key| Some(Chord { ctrl, alt, shift, ..Chord::plain(key) });
        assert_eq!(Chord::parse("cmd+shift+p"), chord(true, false, true, Key::P));
        assert_eq!(Chord::parse("Option + T"), chord(false, true, false, Key::T));
        assert_eq!(Chord::parse("Ctrl++"), chord(true, false, false, Key::Plus));
        assert_eq!(Chord::parse("shift+f12"), chord(false, false, true, Key::F12));
        assert_eq!(Chord::parse("ctrl+pagedown"), chord(true, false, false, Key::PageDown));
        assert_eq!(Chord::parse("Ctrl+Shift"), None);
        assert_eq!(Chord::parse("Ctrl+ShiftLeft"), None);
        assert_eq!(Chord::parse("Hyper+P"), None);
        assert_eq!(Chord::parse("Ctrl+`").unwrap().to_config(), "Ctrl+`");
        assert_eq!(Chord::parse("ctrl+minus").unwrap().to_config(), "Ctrl+-");
        // The Control key of a Mac: its own modifier there, Ctrl elsewhere.
        let mac_ctrl = Chord::parse("MacCtrl+`").unwrap();
        assert_eq!((mac_ctrl.ctrl, mac_ctrl.mac_ctrl), (!MAC, MAC));
        assert_eq!(Chord::parse(&mac_ctrl.to_config()), Some(mac_ctrl));
        assert_eq!(Chord::parse_label(&mac_ctrl.label()), Some(mac_ctrl));
        assert_eq!(mac_ctrl.label(), "Ctrl+`");
        let primary = Chord::parse("Ctrl+F").unwrap();
        assert_eq!(primary.label(), if MAC { "Cmd+F" } else { "Ctrl+F" });
        assert_eq!(Chord::parse_label(&primary.label()), Some(primary));
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

    /// The modifiers egui reports while `chord` is held.
    fn press(chord: Chord) -> Modifiers {
        Modifiers {
            alt: chord.alt,
            shift: chord.shift,
            command: chord.ctrl,
            mac_cmd: MAC && chord.ctrl,
            ctrl: if MAC { chord.mac_ctrl } else { chord.ctrl },
        }
    }

    #[test]
    fn every_default_triggers_its_command() {
        let keymap = Keymap::new(&BTreeMap::new());
        for &command in ALL {
            for &chord in keymap.chords(command) {
                assert_eq!(keymap.lookup(command.scope(), chord.key, None, press(chord)), Some(command), "{}", chord.label());
                assert_eq!(Chord::from_event(chord.key, press(chord)), chord);
            }
        }
    }

    #[test]
    fn lookup_prefers_exact_modifiers() {
        let keymap = Keymap::new(&BTreeMap::new());
        let m = |text: &str| press(Chord::parse(&format!("{text}+A")).unwrap());
        assert_eq!(keymap.lookup(Scope::Global, Key::P, None, m("Ctrl+Shift")), Some(Command::CommandPalette));
        assert_eq!(keymap.lookup(Scope::Global, Key::P, None, m("Ctrl")), Some(Command::QuickOpen));
        // `+` needs Shift on a US keyboard.
        assert_eq!(keymap.lookup(Scope::Global, Key::Plus, None, m("Ctrl+Shift")), Some(Command::ZoomIn));
        // Ctrl+Shift+D is not Ctrl+D.
        assert_eq!(keymap.lookup(Scope::Editor, Key::D, None, m("Ctrl+Shift")), None);
        // A non-Latin layout: the logical key is unknown, the physical one is P.
        assert_eq!(keymap.lookup(Scope::Global, Key::Q, Some(Key::P), m("Ctrl")), Some(Command::Quit));
        assert_eq!(keymap.lookup(Scope::Global, Key::F20, Some(Key::P), m("Ctrl")), Some(Command::QuickOpen));
        assert_eq!(keymap.lookup(Scope::Editor, Key::Enter, None, m("Ctrl")), Some(Command::InsertLineBelow));
        assert_eq!(keymap.lookup(Scope::CommitMessage, Key::Enter, None, m("Ctrl")), Some(Command::GitCommit));
        if MAC {
            // Control and Cmd are different keys on a Mac.
            assert_eq!(keymap.lookup(Scope::Global, Key::P, None, m("MacCtrl")), None);
            assert_eq!(keymap.lookup(Scope::Global, Key::Backtick, None, m("MacCtrl")), Some(Command::ToggleTerminal));
            assert_eq!(keymap.lookup(Scope::Global, Key::Backtick, None, m("Ctrl")), None);
        }
    }

    #[test]
    fn clipboard_keys() {
        let primary = clipboard_modifiers();
        let key = |key, shift| Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers: Modifiers { shift, ..primary },
        };
        let paste = || Event::Paste("x".into());
        let run = |keymap: &Keymap, scope, event: Event, clipboard: bool| {
            let mut input = InputState::default();
            input.events.push(event);
            let triggered = consume_with(keymap, &mut input, scope, clipboard);
            (triggered, input.events.len())
        };
        let defaults = Keymap::new(&BTreeMap::new());
        // Ctrl+Shift+V / Ctrl+Shift+C in a terminal (egui 0.37 sends them as key presses).
        assert_eq!(run(&defaults, Scope::Terminal, key(Key::V, true), true), (vec![Command::TerminalPaste], 0));
        assert_eq!(run(&defaults, Scope::Terminal, key(Key::C, true), true), (vec![Command::TerminalCopy], 0));
        // Plain Ctrl+V is left to whoever has the focus.
        assert_eq!(run(&defaults, Scope::Global, paste(), true), (vec![], 1));
        assert_eq!(clipboard_action(&defaults, Scope::Editor, &paste()), Some(ClipboardAction::Native));
        assert_eq!(clipboard_action(&defaults, Scope::Terminal, &Event::Copy), Some(ClipboardAction::Native));
        // Ctrl+X in a terminal is no copy command: it goes to the program.
        assert_eq!(clipboard_action(&defaults, Scope::Terminal, &Event::Cut), Some(ClipboardAction::Unbound));
        // Ctrl+V given to another command runs it instead of pasting...
        let taken = Keymap::new(&BTreeMap::from([("quick_open".to_string(), vec!["Ctrl+V".to_string()])]));
        assert_eq!(run(&taken, Scope::Global, paste(), true), (vec![Command::QuickOpen], 0));
        // ...but not a paste a command asked for.
        assert_eq!(run(&taken, Scope::Global, paste(), false), (vec![], 1));
        // Paste without Ctrl+V: in the editor nothing, in a terminal ^V.
        let rebound = Keymap::new(&BTreeMap::from([
            ("paste".to_string(), vec!["Ctrl+Shift+V".to_string()]),
            ("terminal_paste".to_string(), vec!["Ctrl+Shift+V".to_string()]),
        ]));
        assert_eq!(clipboard_action(&rebound, Scope::Editor, &paste()), Some(ClipboardAction::Unbound));
        assert_eq!(clipboard_action(&rebound, Scope::Terminal, &paste()), Some(ClipboardAction::Unbound));
        let swapped = Keymap::new(&BTreeMap::from([("terminal_copy".to_string(), vec!["Ctrl+V".to_string()])]));
        assert_eq!(clipboard_action(&swapped, Scope::Terminal, &paste()), Some(ClipboardAction::Run(Command::TerminalCopy)));
    }

    #[test]
    fn overrides_replace_and_unbind() {
        let overrides = BTreeMap::from([
            ("quick_open".to_string(), vec!["Ctrl+E".to_string()]),
            ("quit".to_string(), vec![]),
            ("nope".to_string(), vec!["Ctrl+X".to_string()]),
            ("save".to_string(), vec!["Ctrl+Bogus".to_string()]),
            // Left by the old recorder that took a modifier for the key.
            ("new_terminal".to_string(), vec!["ControlLeft".to_string()]),
        ]);
        let keymap = Keymap::new(&overrides);
        assert_eq!(keymap.chords(Command::QuickOpen), [Chord::parse("Ctrl+E").unwrap()]);
        assert!(keymap.chords(Command::Quit).is_empty());
        // Nothing readable: the defaults stay.
        assert_eq!(keymap.chords(Command::Save), [Chord::parse("Ctrl+S").unwrap()]);
        assert_eq!(
            keymap.chords(Command::NewTerminal),
            defaults(Command::NewTerminal).iter().filter_map(|t| Chord::parse(t)).collect::<Vec<_>>()
        );
        assert_eq!(keymap.problems.len(), 3, "{:?}", keymap.problems);
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
        let primary = if MAC { "cmd" } else { "ctrl" };
        assert_eq!(find(&format!("{primary}+shift+p")), vec![Command::CommandPalette]);
        assert_eq!(find("Cmd + Shift + P"), vec![Command::CommandPalette]);
        assert_eq!(find(&format!("{primary}+=")), vec![Command::ZoomIn]);
        assert_eq!(find(&format!("{primary}++")), vec![Command::ZoomIn]);
        assert_eq!(find(&Chord::parse("MacCtrl+`").unwrap().label()), vec![Command::ToggleTerminal]);
        assert!(find("backtick").contains(&Command::ToggleTerminal));
        let f5 = find("f5");
        for command in [Command::RunDefault, Command::RerunLast, Command::KillTerminal] {
            assert!(f5.contains(&command), "{command:?} in {f5:?}");
        }
        assert!(find(&format!("{primary}+shift")).contains(&Command::CommandPalette));
        assert!(find("").len() == ALL.len());
        assert!(find("zzzz").is_empty());
    }
}
