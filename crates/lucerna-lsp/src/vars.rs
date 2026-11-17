//! Variables in a server's `args`, `env`, `initialization_options` and
//! `settings`: `{root}`, `{data_dir}` and `{npm_root}` (spec §3.3).

use std::fmt;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::launcher::Launcher;
use crate::table::ServerDef;

/// The values a server's variables expand to.
#[derive(Clone, Debug)]
pub struct Vars {
    /// The server's project root.
    pub root: PathBuf,
    /// `<data dir>/lsp/<server>/<hash of root>`; see [`data_dir_for`].
    pub data_dir: PathBuf,
    /// `npm root -g`; `None` when unknown or npm failed.
    pub npm_root: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum VarError {
    /// A `{name}` that is not a known variable (the name, without braces).
    Unknown(String),
    /// `{npm_root}` is used but `npm root -g` gave nothing.
    NpmRootUnavailable,
}

impl fmt::Display for VarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VarError::Unknown(name) => write!(f, "unknown variable {{{name}}}"),
            VarError::NpmRootUnavailable => f.write_str("npm not found; {npm_root} unavailable"),
        }
    }
}

impl std::error::Error for VarError {}

/// The variables in `text`: each `{name}` whose name is an identifier
/// (ASCII letter or `_`, then letters, digits or `_`), with its byte range.
/// Other braces are literal text.
fn variables(text: &str) -> impl Iterator<Item = (std::ops::Range<usize>, &str)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < bytes.len() {
            let start = i;
            i += 1;
            if bytes[start] != b'{' {
                continue;
            }
            let name_start = start + 1;
            let mut end = name_start;
            while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
                end += 1;
            }
            let valid = end > name_start
                && end < bytes.len()
                && bytes[end] == b'}'
                && !bytes[name_start].is_ascii_digit();
            if valid {
                i = end + 1;
                return Some((start..end + 1, &text[name_start..end]));
            }
        }
        None
    })
}

/// `text` with every variable replaced.
///
/// # Errors
/// [`VarError::Unknown`] for an unknown variable;
/// [`VarError::NpmRootUnavailable`] for `{npm_root}` without a value.
pub fn expand(text: &str, vars: &Vars) -> Result<String, VarError> {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (range, name) in variables(text) {
        out.push_str(&text[last..range.start]);
        let value = match name {
            "root" => vars.root.to_string_lossy(),
            "data_dir" => vars.data_dir.to_string_lossy(),
            "npm_root" => vars
                .npm_root
                .as_deref()
                .ok_or(VarError::NpmRootUnavailable)?
                .to_string_lossy(),
            _ => return Err(VarError::Unknown(name.to_owned())),
        };
        out.push_str(&value);
        last = range.end;
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// `value` with [`expand`] applied to every string inside it (object keys
/// are left alone).
///
/// # Errors
/// The first error of [`expand`].
pub fn expand_value(value: &Value, vars: &Vars) -> Result<Value, VarError> {
    Ok(match value {
        Value::String(text) => Value::String(expand(text, vars)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| expand_value(item, vars))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| Ok((key.clone(), expand_value(item, vars)?)))
                .collect::<Result<_, _>>()?,
        ),
        other => other.clone(),
    })
}

/// `base/lsp/<server>/<first 12 hex digits of sha256(root)>`: one
/// directory per server, root and slot, stable across sessions. Slot 0
/// hashes the root alone; another slot also hashes `\0<slot>`, so two
/// windows on one root get different directories. Not created.
pub fn data_dir_for(base: &Path, server: &str, root: &Path, slot: u32) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(root.as_os_str().as_encoded_bytes());
    if slot > 0 {
        hasher.update(format!("\0{slot}").as_bytes());
    }
    let digest = hasher.finalize();
    let hash: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    base.join("lsp").join(server).join(hash)
}

/// Whether `{name}` appears in any field of `def` that is expanded:
/// `args`, `env`, `binary.arguments`, `binary.env`,
/// `initialization_options` or `settings`.
pub fn uses_variable(def: &ServerDef, name: &str) -> bool {
    let has = |text: &str| variables(text).any(|(_, n)| n == name);
    fn in_value(value: &Value, has: &dyn Fn(&str) -> bool) -> bool {
        match value {
            Value::String(text) => has(text),
            Value::Array(items) => items.iter().any(|item| in_value(item, has)),
            Value::Object(map) => map.values().any(|item| in_value(item, has)),
            _ => false,
        }
    }
    let binary = def.binary.as_ref();
    def.args.iter().any(|a| has(a))
        || def.env.values().any(|v| has(v))
        || binary
            .and_then(|b| b.arguments.as_ref())
            .is_some_and(|args| args.iter().any(|a| has(a)))
        || binary
            .and_then(|b| b.env.as_ref())
            .is_some_and(|env| env.values().any(|v| has(v)))
        || [&def.initialization_options, &def.settings]
            .into_iter()
            .flatten()
            .any(|value| in_value(value, &has))
}

/// Whether starting `def` needs `npm root -g`.
pub fn uses_npm_root(def: &ServerDef) -> bool {
    uses_variable(def, "npm_root")
}

/// `npm root -g`, trimmed; `None` when npm is missing, fails or prints
/// nothing. Blocks (up to 5 s): call it off the UI thread.
pub fn npm_root(launcher: &dyn Launcher) -> Option<PathBuf> {
    let out = launcher.output("npm", &["root", "-g"])?;
    let trimmed = out.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}
