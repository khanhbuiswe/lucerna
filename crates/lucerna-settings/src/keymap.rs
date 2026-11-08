//! Keystroke parsing, key bindings and chord-aware resolution.

use serde_json::Value;

use crate::SettingsError;
use crate::store::parse_jsonc;
use crate::when::{Context, WhenExpr, parse_when};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keystroke {
    pub cmd: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Lowercase key name, for example `"p"`, `"\\"`, `"tab"`, `"="`.
    pub key: String,
}

impl Keystroke {
    fn label(&self) -> String {
        let mut out = String::new();
        for (on, sym) in [
            (self.ctrl, '⌃'),
            (self.alt, '⌥'),
            (self.shift, '⇧'),
            (self.cmd, '⌘'),
        ] {
            if on {
                out.push(sym);
            }
        }
        out.push_str(&key_label(&self.key));
        out
    }
}

fn key_label(key: &str) -> String {
    match key {
        "escape" | "esc" => "Esc".into(),
        "tab" => "Tab".into(),
        "enter" | "return" => "Enter".into(),
        "space" => "Space".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        k if k.chars().count() == 1 => k.to_uppercase(),
        k => {
            let mut chars = k.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

fn err(message: impl Into<String>) -> SettingsError {
    SettingsError {
        message: message.into(),
        line: None,
    }
}

fn parse_stroke(part: &str) -> Result<Keystroke, SettingsError> {
    // A trailing '+' is the key itself ("cmd++").
    let (mods, key) = if part == "+" {
        ("", "+")
    } else if let Some(mods) = part.strip_suffix("++") {
        (mods, "+")
    } else {
        part.rsplit_once('+').unwrap_or(("", part))
    };
    if key.is_empty() {
        return Err(err(format!("missing key in keystroke '{part}'")));
    }
    let mut stroke = Keystroke {
        cmd: false,
        ctrl: false,
        alt: false,
        shift: false,
        key: key.to_lowercase(),
    };
    for m in mods.split('+').filter(|m| !m.is_empty()) {
        match m.to_lowercase().as_str() {
            "cmd" | "meta" | "super" => stroke.cmd = true,
            "ctrl" | "control" => stroke.ctrl = true,
            "alt" | "option" => stroke.alt = true,
            "shift" => stroke.shift = true,
            other => return Err(err(format!("unknown modifier '{other}' in '{part}'"))),
        }
    }
    Ok(stroke)
}

/// Parses `"cmd+k cmd+t"`; a space separates the parts of a chord.
pub fn parse_keys(s: &str) -> Result<Vec<Keystroke>, SettingsError> {
    let strokes = s
        .split_whitespace()
        .map(parse_stroke)
        .collect::<Result<Vec<_>, _>>()?;
    if strokes.is_empty() {
        return Err(err("empty key binding"));
    }
    Ok(strokes)
}

#[derive(Debug, Clone)]
pub struct KeyBinding {
    pub keys: Vec<Keystroke>,
    pub command: String,
    pub args: Option<Value>,
    pub when: Option<WhenExpr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    Command {
        command: String,
        args: Option<Value>,
    },
    Pending,
    NoMatch,
}

#[derive(Debug, Clone)]
struct Entry {
    binding: KeyBinding,
    /// Trimmed `when` source, used to match `-command` removals.
    when_src: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Keymap {
    entries: Vec<Entry>,
}

const DEFAULT_BINDINGS: &str = include_str!("../assets/default-keybindings.json");

impl Keymap {
    pub fn default_bindings() -> &'static str {
        DEFAULT_BINDINGS
    }

    /// Defaults first, then user entries. A broken user file yields the
    /// defaults plus the error.
    pub fn load(default_json: &str, user_json: Option<&str>) -> (Keymap, Option<SettingsError>) {
        let mut keymap = Keymap::default();
        if let Err(e) = keymap.apply(default_json) {
            // Defaults are bundled; surface the error but stay usable.
            return (Keymap::default(), Some(e));
        }
        let Some(user) = user_json else {
            return (keymap, None);
        };
        let mut layered = keymap.clone();
        match layered.apply(user) {
            Ok(()) => (layered, None),
            Err(e) => (keymap, Some(e)),
        }
    }

    fn apply(&mut self, json: &str) -> Result<(), SettingsError> {
        let items = match parse_jsonc(json)? {
            None => return Ok(()),
            Some(Value::Array(items)) => items,
            Some(_) => return Err(err("keybindings must be a JSON array")),
        };
        for (i, item) in items.iter().enumerate() {
            let at = |e: SettingsError| err(format!("entry {}: {}", i + 1, e.message));
            let (entry, removal) = parse_entry(item).map_err(at)?;
            if removal {
                self.entries.retain(|e| {
                    !(e.binding.keys == entry.binding.keys
                        && e.binding.command == entry.binding.command
                        && (entry.when_src.is_none() || e.when_src == entry.when_src))
                });
            } else {
                self.entries.push(entry);
            }
        }
        Ok(())
    }

    fn active<'a>(&'a self, ctx: &'a Context) -> impl Iterator<Item = &'a KeyBinding> {
        self.entries
            .iter()
            .rev()
            .map(|e| &e.binding)
            .filter(|b| b.when.as_ref().is_none_or(|w| w.eval(ctx)))
    }

    pub fn resolve(&self, pending: &[Keystroke], ctx: &Context) -> Resolution {
        if let Some(b) = self.active(ctx).find(|b| b.keys == pending) {
            return Resolution::Command {
                command: b.command.clone(),
                args: b.args.clone(),
            };
        }
        let is_prefix = self
            .active(ctx)
            .any(|b| b.keys.len() > pending.len() && b.keys.starts_with(pending));
        if is_prefix {
            Resolution::Pending
        } else {
            Resolution::NoMatch
        }
    }

    /// Label of the last-defined binding for `command`, e.g. `"⌘P"` or `"⌘K ⌘T"`.
    pub fn label_for(&self, command: &str) -> Option<String> {
        let b = &self
            .entries
            .iter()
            .rev()
            .find(|e| e.binding.command == command)?
            .binding;
        Some(
            b.keys
                .iter()
                .map(Keystroke::label)
                .collect::<Vec<_>>()
                .join(" "),
        )
    }
}

/// Returns the entry and whether it is a `-command` removal.
fn parse_entry(item: &Value) -> Result<(Entry, bool), SettingsError> {
    let obj = item.as_object().ok_or_else(|| err("expected an object"))?;
    let string = |name: &str| -> Result<&str, SettingsError> {
        obj.get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| err(format!("missing string field '{name}'")))
    };
    let keys = parse_keys(string("key")?)?;
    let command = string("command")?;
    let (command, removal) = match command.strip_prefix('-') {
        Some(rest) => (rest, true),
        None => (command, false),
    };
    let when_src = match obj.get("when") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.trim().to_string()),
        Some(_) => return Err(err("'when' must be a string")),
    };
    let when = when_src.as_deref().map(parse_when).transpose()?;
    Ok((
        Entry {
            binding: KeyBinding {
                keys,
                command: command.to_string(),
                args: obj.get("args").cloned(),
                when,
            },
            when_src,
        },
        removal,
    ))
}
