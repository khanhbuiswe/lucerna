//! Snippet file validation (TextMate syntax). There is no snippet runtime.

use std::path::Path;

use serde_json::Value;

/// Maximum `${n:...}` nesting accepted.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetFile {
    /// `None` for the global `snippets` file.
    pub language: Option<String>,
    pub valid: usize,
    pub errors: Vec<String>,
}

/// Validate a snippet file; the language comes from the lowercased stem.
pub fn check_snippets(path: &Path) -> Result<SnippetFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value = jsonc_parser::parse_to_serde_value(&text, &Default::default())
        .map_err(|e| format!("{}: {e}", path.display()))?
        .ok_or_else(|| format!("{}: empty file", path.display()))?;
    let Value::Object(map) = value else {
        return Err(format!("{}: expected a JSON object", path.display()));
    };
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let language = (stem != "snippets" && !stem.is_empty()).then_some(stem);
    let mut valid = 0;
    let mut errors = Vec::new();
    for (name, entry) in &map {
        match check_entry(entry) {
            Ok(()) => valid += 1,
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Ok(SnippetFile {
        language,
        valid,
        errors,
    })
}

fn string_or_lines(v: &Value, key: &str) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Array(a) => a
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(|l| l.join("\n"))
            .ok_or_else(|| format!("`{key}` array must contain only strings")),
        _ => Err(format!("`{key}` must be a string or an array of strings")),
    }
}

fn check_entry(entry: &Value) -> Result<(), String> {
    let obj = entry.as_object().ok_or("entry must be an object")?;
    let body = obj.get("body").ok_or("missing `body`")?;
    for key in ["prefix", "description"] {
        if let Some(v) = obj.get(key) {
            string_or_lines(v, key)?;
        }
    }
    check_snippet_body(&string_or_lines(body, "body")?)
}

/// Check a snippet body against the TextMate snippet grammar.
pub fn check_snippet_body(body: &str) -> Result<(), String> {
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    parse_text(&chars, &mut i, false, 0)
}

/// Parse until end of input, or (when `nested`) until the closing `}`,
/// which is consumed.
fn parse_text(c: &[char], i: &mut usize, nested: bool, depth: usize) -> Result<(), String> {
    while *i < c.len() {
        match c[*i] {
            '\\' => *i += 2,
            '}' if nested => {
                *i += 1;
                return Ok(());
            }
            '$' => {
                *i += 1;
                parse_dollar(c, i, depth)?;
            }
            _ => *i += 1,
        }
    }
    if nested {
        Err("unclosed `${`".into())
    } else {
        Ok(())
    }
}

fn take_while(c: &[char], i: &mut usize, f: impl Fn(char) -> bool) -> usize {
    let start = *i;
    while *i < c.len() && f(c[*i]) {
        *i += 1;
    }
    *i - start
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_'
}

fn parse_dollar(c: &[char], i: &mut usize, depth: usize) -> Result<(), String> {
    match c.get(*i) {
        Some(ch) if ch.is_ascii_digit() => {
            take_while(c, i, |x| x.is_ascii_digit());
            Ok(())
        }
        Some(ch) if is_ident_start(*ch) => {
            take_while(c, i, |x| x.is_ascii_alphanumeric() || x == '_');
            Ok(())
        }
        Some('{') => {
            *i += 1;
            parse_braced(c, i, depth)
        }
        // A lone `$` is literal text.
        _ => Ok(()),
    }
}

fn parse_braced(c: &[char], i: &mut usize, depth: usize) -> Result<(), String> {
    if depth >= MAX_DEPTH {
        return Err("snippet nesting too deep".into());
    }
    let is_tabstop = c.get(*i).is_some_and(|ch| ch.is_ascii_digit());
    if is_tabstop {
        take_while(c, i, |x| x.is_ascii_digit());
    } else if c.get(*i).is_some_and(|ch| is_ident_start(*ch)) {
        take_while(c, i, |x| x.is_ascii_alphanumeric() || x == '_');
    } else {
        return Err("expected a tabstop number or variable name after `${`".into());
    }
    match c.get(*i) {
        Some('}') => {
            *i += 1;
            Ok(())
        }
        Some(':') => {
            *i += 1;
            parse_text(c, i, true, depth + 1)
        }
        Some('|') if is_tabstop => {
            *i += 1;
            loop {
                match c.get(*i) {
                    None => return Err("unterminated choice".into()),
                    Some('\\') => *i += 2,
                    Some('|') if c.get(*i + 1) == Some(&'}') => {
                        *i += 2;
                        return Ok(());
                    }
                    Some('|') => return Err("unterminated choice".into()),
                    Some(_) => *i += 1,
                }
            }
        }
        Some('/') => {
            // Regex transform: not interpreted, only required to be closed.
            while *i < c.len() {
                match c[*i] {
                    '\\' => *i += 2,
                    '}' => {
                        *i += 1;
                        return Ok(());
                    }
                    _ => *i += 1,
                }
            }
            Err("unclosed `${`".into())
        }
        _ => Err("unclosed `${`".into()),
    }
}
