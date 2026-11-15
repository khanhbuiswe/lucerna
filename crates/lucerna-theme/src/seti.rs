//! The built-in "Seti" icon theme: VS Code's file icons, matched by file
//! name, then extension, then language, as VS Code does.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock};

use crate::icon_theme::{IconSource, IconTheme};
use crate::seti_data::{
    DEFAULT_DEF, DEFS, FILE_EXTENSIONS, FILE_NAMES, LANG_EXTENSIONS, LANG_FILE_NAMES,
    LANG_NAME_GLOBS, LANG_NAME_PREFIXES, LANG_NAME_SUFFIXES, SetiDef,
};

type Table = HashMap<&'static str, &'static SetiDef>;

/// The lookup tables of the Seti theme, built once from the generated data.
pub(crate) struct SetiIndex {
    file_names: Table,
    file_extensions: Table,
    lang_file_names: Table,
    lang_extensions: Table,
    prefixes: Vec<(&'static str, &'static SetiDef)>,
    suffixes: Vec<(&'static str, &'static SetiDef)>,
    globs: Vec<(&'static str, &'static SetiDef)>,
    default: &'static SetiDef,
}

impl std::fmt::Debug for SetiIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetiIndex").finish_non_exhaustive()
    }
}

fn def(key: &str) -> &'static SetiDef {
    DEFS.iter()
        .find(|(k, _)| *k == key)
        .map(|(_, d)| d)
        .unwrap_or_else(|| panic!("seti def {key:?} is missing"))
}

fn table(rows: &'static [(&'static str, &'static str)]) -> Table {
    rows.iter().map(|(k, v)| (*k, def(v))).collect()
}

/// Rows sorted longest key first, so the first match is the longest.
fn longest_first(
    rows: &'static [(&'static str, &'static str)],
) -> Vec<(&'static str, &'static SetiDef)> {
    let mut v: Vec<_> = rows.iter().map(|(k, d)| (*k, def(d))).collect();
    v.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    v
}

/// Whole-name match of `pattern`, where `*` matches any run of characters
/// (including none) and everything else matches itself.
fn glob_match(pattern: &str, name: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };
    let mut parts: Vec<&str> = parts.collect();
    let Some(last) = parts.pop() else {
        // No `*`: the whole name must equal the pattern.
        return rest.is_empty();
    };
    for part in parts {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

impl SetiIndex {
    pub(crate) fn new() -> SetiIndex {
        SetiIndex {
            file_names: table(FILE_NAMES),
            file_extensions: table(FILE_EXTENSIONS),
            lang_file_names: table(LANG_FILE_NAMES),
            lang_extensions: table(LANG_EXTENSIONS),
            prefixes: longest_first(LANG_NAME_PREFIXES),
            suffixes: longest_first(LANG_NAME_SUFFIXES),
            globs: LANG_NAME_GLOBS.iter().map(|(k, d)| (*k, def(d))).collect(),
            default: def(DEFAULT_DEF),
        }
    }

    /// The first hit of `table` over the extensions of `name`, longest
    /// first: the part after each `.`, left to right (`a.spec.ts` tries
    /// `spec.ts`, then `ts`).
    fn extension(table: &Table, name: &str) -> Option<&'static SetiDef> {
        let mut rest = name;
        while let Some(dot) = rest.find('.') {
            rest = &rest[dot + 1..];
            if !rest.is_empty()
                && let Some(d) = table.get(rest)
            {
                return Some(d);
            }
        }
        None
    }

    fn find(&self, name: &str) -> &'static SetiDef {
        let n = name.to_lowercase();
        if let Some(d) = self.file_names.get(n.as_str()) {
            return d;
        }
        if let Some(d) = Self::extension(&self.file_extensions, &n) {
            return d;
        }
        if let Some(d) = self.lang_file_names.get(n.as_str()) {
            return d;
        }
        if let Some((_, d)) = self.prefixes.iter().find(|(p, _)| n.starts_with(p)) {
            return d;
        }
        if let Some((_, d)) = self.suffixes.iter().find(|(s, _)| n.ends_with(s)) {
            return d;
        }
        if let Some((_, d)) = self.globs.iter().find(|(g, _)| glob_match(g, &n)) {
            return d;
        }
        if let Some(d) = Self::extension(&self.lang_extensions, &n) {
            return d;
        }
        self.default
    }

    pub(crate) fn lookup(&self, name: &str) -> IconSource {
        let d = self.find(name);
        IconSource::Glyph {
            name: d.glyph,
            dark: d.dark,
            light: d.light,
        }
    }
}

/// The built-in Seti icon theme. It has file icons only: folders and
/// chevrons fall back to Lucide.
pub fn seti_icon_theme() -> IconTheme {
    IconTheme::seti(SetiIndex::new())
}

/// The one Seti theme every registry shares, built on first use.
pub(crate) fn shared_seti_theme() -> Arc<IconTheme> {
    static SETI: LazyLock<Arc<IconTheme>> = LazyLock::new(|| Arc::new(seti_icon_theme()));
    SETI.clone()
}

/// The glyph (SVG) names the theme can answer with, sorted, without
/// repeats, so the workbench can check that all of them are embedded.
pub fn seti_glyph_names() -> impl Iterator<Item = &'static str> {
    DEFS.iter()
        .map(|(_, d)| d.glyph)
        .collect::<BTreeSet<_>>()
        .into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance(rgb: u32) -> f64 {
        let ch = |shift: u32| {
            let c = f64::from((rgb >> shift) & 0xff) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    }

    fn contrast(a: u32, b: u32) -> f64 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_light_color_has_3_to_1_contrast() {
        for (key, d) in DEFS {
            let c = contrast(d.light, 0xffffff);
            assert!(c >= 3.0, "{key}: light {:06x} has contrast {c:.2}", d.light);
        }
    }

    /// VS Code's light colour: each channel x 0.9, rounded half up.
    fn vscode_light(rgb: u32) -> u32 {
        let ch = |shift: u32| (f64::from((rgb >> shift) & 0xff) * 0.9 + 0.5) as u32;
        (ch(16) << 16) | (ch(8) << 8) | ch(0)
    }

    #[test]
    fn light_color_is_vscodes_when_it_already_has_contrast() {
        let default = DEFS
            .iter()
            .find(|(k, _)| *k == DEFAULT_DEF)
            .map(|(_, d)| d)
            .unwrap();
        let mut checked = 0;
        for (key, d) in DEFS {
            // A definition without its own colour (`_todo`) takes the
            // default's; the generated data already stores that.
            let dark = if *key == "_todo" {
                default.dark
            } else {
                d.dark
            };
            let vs = vscode_light(dark);
            if contrast(vs, 0xffffff) >= 3.0 {
                assert_eq!(d.light, vs, "{key}: dark {:06x}", d.dark);
                checked += 1;
            }
        }
        assert!(checked > 0);
    }

    #[test]
    fn defs_referenced_by_tables_exist() {
        let tables: &[&[(&str, &str)]] = &[
            FILE_NAMES,
            FILE_EXTENSIONS,
            LANG_FILE_NAMES,
            LANG_NAME_PREFIXES,
            LANG_NAME_SUFFIXES,
            LANG_NAME_GLOBS,
            LANG_EXTENSIONS,
        ];
        for rows in tables {
            for (key, d) in *rows {
                assert!(DEFS.iter().any(|(k, _)| k == d), "{key} -> missing {d}");
            }
        }
        assert!(DEFS.iter().any(|(k, _)| *k == DEFAULT_DEF));
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*docker*compose*.yml", "docker-compose.yml"));
        assert!(glob_match(
            "*docker*compose*.yml",
            "my-docker.compose.prod.yml"
        ));
        assert!(!glob_match("*docker*compose*.yml", "docker-compose.yaml"));
        assert!(glob_match("jenkinsfile*", "jenkinsfile"));
        assert!(glob_match("compose.*.yml", "compose.dev.yml"));
        assert!(!glob_match("compose.*.yml", "compose..yaml"));
        assert!(glob_match("a", "a"));
        assert!(!glob_match("a", "ab"));
        assert!(!glob_match("a*a", "a"));
    }
}
