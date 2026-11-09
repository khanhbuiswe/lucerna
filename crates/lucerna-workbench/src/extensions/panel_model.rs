//! What the Extensions page lists (spec §5.2), without a window: the
//! query's filters and fuzzy text, the Installed and Available rows, and
//! the text each card shows.

use lucerna_extension::builder::Stage;
use lucerna_extension::index::{ExtensionIndex, IndexedExtension};
use lucerna_extension::registry::Registry;
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// A `@` filter of the search box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Installed,
    Updates,
    Themes,
    Icons,
    Languages,
}

impl Filter {
    /// The `@` token typed for this filter.
    pub fn token(self) -> &'static str {
        match self {
            Filter::Installed => "@installed",
            Filter::Updates => "@updates",
            Filter::Themes => "@themes",
            Filter::Icons => "@icons",
            Filter::Languages => "@languages",
        }
    }

    fn parse(token: &str) -> Option<Filter> {
        Some(match token {
            "@installed" => Filter::Installed,
            "@updates" => Filter::Updates,
            "@themes" => Filter::Themes,
            "@icons" => Filter::Icons,
            "@languages" => Filter::Languages,
            _ => return None,
        })
    }

    /// Whether an installed extension passes. Every filter hides the
    /// Available group: the registry does not say what an extension holds.
    fn keeps(self, id: &str, ext: &IndexedExtension, updates: &[String]) -> bool {
        match self {
            Filter::Installed => true,
            Filter::Updates => updates.iter().any(|u| u == id),
            Filter::Themes => !ext.themes.is_empty(),
            Filter::Icons => !ext.icon_themes.is_empty(),
            Filter::Languages => !ext.languages.is_empty(),
        }
    }
}

/// The search box split into its filters and the free text between them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub filters: Vec<Filter>,
    pub text: String,
}

/// Splits `query` on whitespace; known `@` tokens are filters, the rest is
/// the free text. An unknown `@word` is text.
pub fn parse_query(query: &str) -> Query {
    let mut parsed = Query::default();
    let mut text = Vec::new();
    for token in query.split_whitespace() {
        match Filter::parse(token) {
            Some(filter) => parsed.filters.push(filter),
            None => text.push(token),
        }
    }
    parsed.text = text.join(" ");
    parsed
}

/// `query` with its filter tokens replaced by `filter`'s (none for
/// `None`, "All"), placed before the free text, which is kept.
pub fn with_filter(query: &str, filter: Option<Filter>) -> String {
    let text = parse_query(query).text;
    match filter {
        Some(filter) if text.is_empty() => filter.token().to_string(),
        Some(filter) => format!("{} {text}", filter.token()),
        None => text,
    }
}

/// The filter button `query` selects: `Some(None)` ("All") without
/// filters, `Some(Some(f))` with exactly `f`, `None` when it combines
/// several.
pub fn selected_filter(query: &str) -> Option<Option<Filter>> {
    match parse_query(query).filters.as_slice() {
        [] => Some(None),
        [filter] => Some(Some(*filter)),
        _ => None,
    }
}

/// What a row's extension is doing, or why it cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowState {
    NotInstalled,
    Installed,
    UpdateAvailable,
    Installing(Stage),
    Failed(String),
    Broken(String),
}

/// `"Building grammar 2/3: zig"`: an install stage as the panel shows it.
pub fn stage_label(stage: &Stage) -> String {
    match stage {
        Stage::Fetching => "Fetching source".into(),
        Stage::ReadingManifest => "Reading manifest".into(),
        Stage::PreparingToolchain => "Preparing toolchain".into(),
        Stage::BuildingGrammar { name, index, total } => {
            format!("Building grammar {}/{total}: {name}", index + 1)
        }
        Stage::Validating => "Validating".into(),
        Stage::Installing => "Installing".into(),
    }
}

/// One extension in the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    /// The installed version, or the registry's for an available one.
    pub version: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub repository: Option<String>,
    pub commit: Option<String>,
    /// What an installed extension holds, e.g. `"2 themes · 1 language"`.
    pub summary: String,
    /// The manifest declares language servers ("Language server: Phase 3").
    pub lsp_note: bool,
    pub state: RowState,
}

impl Row {
    /// The name when known, else the id.
    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// The panel's two groups.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PanelRows {
    pub installed: Vec<Row>,
    pub available: Vec<Row>,
}

impl PanelRows {
    /// Replaces each row's state with what `activity` says about it (a
    /// running install, the last failure), when it says something.
    pub fn overlay(&mut self, activity: impl Fn(&Row) -> Option<RowState>) {
        for row in self.installed.iter_mut().chain(&mut self.available) {
            if let Some(state) = activity(row) {
                row.state = state;
            }
        }
    }

    /// The row of `id` in either group.
    pub fn get(&self, id: &str) -> Option<&Row> {
        self.installed
            .iter()
            .chain(&self.available)
            .find(|row| row.id == id)
    }
}

/// `"1 theme"`, `"2 themes"`.
fn count(n: usize, word: &str) -> String {
    match n {
        1 => format!("1 {word}"),
        n => format!("{n} {word}s"),
    }
}

/// `"2 themes · 1 icon theme · 1 language · 14 snippets (Phase 4)"`, with
/// the empty parts left out.
pub fn summary(ext: &IndexedExtension) -> String {
    let snippets: usize = ext.snippets.values().sum();
    let parts = [
        (ext.themes.len(), "theme", ""),
        (ext.icon_themes.len(), "icon theme", ""),
        (ext.languages.len(), "language", ""),
        (snippets, "snippet", " (Phase 4)"),
    ];
    parts
        .iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, word, suffix)| format!("{}{suffix}", count(*n, word)))
        .collect::<Vec<_>>()
        .join(" · ")
}

fn installed_row(id: &str, ext: &IndexedExtension, updates: &[String]) -> Row {
    let manifest = &ext.manifest;
    let state = match &ext.broken {
        Some(reason) => RowState::Broken(reason.clone()),
        None if updates.iter().any(|u| u == id) => RowState::UpdateAvailable,
        None => RowState::Installed,
    };
    let version = if ext.info.version.is_empty() {
        manifest.version.clone()
    } else {
        ext.info.version.clone()
    };
    Row {
        id: id.to_string(),
        version,
        name: Some(manifest.name.clone()).filter(|n| !n.is_empty()),
        description: manifest.description.clone(),
        repository: manifest.repository.clone(),
        commit: Some(ext.info.commit.clone()).filter(|c| !c.is_empty()),
        summary: summary(ext),
        lsp_note: !manifest.language_servers.is_empty(),
        state,
    }
}

/// Keeps the rows whose id, name or description fuzzy-matches `text`, best
/// first (ties by id); all of them, in order, when `text` is empty.
fn fuzzy(rows: Vec<Row>, text: &str) -> Vec<Row> {
    if text.is_empty() {
        return rows;
    }
    let pattern = Pattern::new(
        text,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, Row)> = rows
        .into_iter()
        .filter_map(|row| {
            let fields = [Some(&row.id), row.name.as_ref(), row.description.as_ref()];
            let score = fields
                .into_iter()
                .flatten()
                .filter_map(|field| pattern.score(Utf32Str::new(field, &mut buf), &mut matcher))
                .max()?;
            Some((score, row))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    scored.into_iter().map(|(_, row)| row).collect()
}

/// The rows for `query`: installed extensions from `index` (by id), and
/// the registry's other entries. `updates` are the installed ids with a
/// newer registry version.
pub fn rows(
    registry: Option<&Registry>,
    index: &ExtensionIndex,
    updates: &[String],
    query: &str,
) -> PanelRows {
    let query = parse_query(query);
    let installed: Vec<Row> = index
        .extensions
        .iter()
        .filter(|(id, ext)| query.filters.iter().all(|f| f.keeps(id, ext, updates)))
        .map(|(id, ext)| installed_row(id, ext, updates))
        .collect();
    let available: Vec<Row> = match registry {
        Some(registry) if query.filters.is_empty() => registry
            .entries
            .iter()
            .filter(|e| !index.extensions.contains_key(&e.id))
            .map(|e| Row {
                id: e.id.clone(),
                version: e.version.clone(),
                name: e.name.clone(),
                description: e.description.clone(),
                repository: Some(e.url.clone()),
                commit: Some(e.commit.clone()),
                summary: String::new(),
                lsp_note: false,
                state: RowState::NotInstalled,
            })
            .collect(),
        _ => Vec::new(),
    };
    PanelRows {
        installed: fuzzy(installed, &query.text),
        available: fuzzy(available, &query.text),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::time::SystemTime;

    use lucerna_extension::index::{InstallInfo, LanguageEntry, Source, ThemeEntry};
    use lucerna_extension::manifest::ExtensionManifest;
    use lucerna_extension::registry::RegistryEntry;

    use super::*;

    fn entry(id: &str, version: &str) -> RegistryEntry {
        RegistryEntry {
            id: id.into(),
            version: version.into(),
            url: format!("https://example.com/{id}"),
            commit: "abc".into(),
            path: None,
            name: None,
            description: None,
        }
    }

    fn registry(entries: Vec<RegistryEntry>) -> Registry {
        Registry {
            fetched_at: SystemTime::UNIX_EPOCH,
            entries,
        }
    }

    fn theme(name: &str) -> ThemeEntry {
        ThemeEntry {
            name: name.into(),
            appearance: "dark".into(),
            path: PathBuf::from("themes/x.json"),
        }
    }

    fn language(name: &str) -> LanguageEntry {
        LanguageEntry {
            name: name.into(),
            dir: PathBuf::from("languages/x"),
            grammar: None,
            path_suffixes: Vec::new(),
            first_line_pattern: None,
            hidden: false,
            queries: Vec::new(),
        }
    }

    fn installed(id: &str, version: &str) -> IndexedExtension {
        IndexedExtension {
            manifest: ExtensionManifest {
                id: id.into(),
                name: format!("Ext {id}"),
                version: version.into(),
                schema_version: 1,
                ..Default::default()
            },
            info: InstallInfo::new("c0ffee".into(), version.into(), Source::Registry),
            ..Default::default()
        }
    }

    fn index(exts: Vec<(&str, IndexedExtension)>) -> ExtensionIndex {
        ExtensionIndex {
            extensions: exts
                .into_iter()
                .map(|(id, ext)| (id.to_string(), ext))
                .collect(),
        }
    }

    fn ids(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn filter_installed_and_updates() {
        let reg = registry(vec![
            entry("a", "0.2.0"),
            entry("b", "0.1.0"),
            entry("c", "0.1.0"),
        ]);
        let mut themed = installed("b", "0.1.0");
        themed.themes.push(theme("B Dark"));
        let idx = index(vec![("a", installed("a", "0.1.0")), ("b", themed)]);
        let updates = vec!["a".to_string()];

        let all = rows(Some(&reg), &idx, &updates, "");
        assert_eq!(ids(&all.installed), ["a", "b"]);
        assert_eq!(ids(&all.available), ["c"]);
        assert_eq!(all.installed[0].state, RowState::UpdateAvailable);
        assert_eq!(all.installed[1].state, RowState::Installed);
        assert_eq!(all.available[0].state, RowState::NotInstalled);

        let only_installed = rows(Some(&reg), &idx, &updates, "@installed");
        assert_eq!(ids(&only_installed.installed), ["a", "b"]);
        assert!(only_installed.available.is_empty());

        let only_updates = rows(Some(&reg), &idx, &updates, "@updates");
        assert_eq!(ids(&only_updates.installed), ["a"]);
        assert!(only_updates.available.is_empty());

        let themes = rows(Some(&reg), &idx, &updates, "@themes");
        assert_eq!(ids(&themes.installed), ["b"]);
        assert!(
            rows(Some(&reg), &idx, &updates, "@icons")
                .installed
                .is_empty()
        );
        assert!(
            rows(Some(&reg), &idx, &updates, "@languages")
                .installed
                .is_empty()
        );

        // A filter and free text combine.
        assert!(
            rows(Some(&reg), &idx, &updates, "@installed zzz")
                .installed
                .is_empty()
        );
        assert_eq!(
            ids(&rows(Some(&reg), &idx, &updates, "@installed Ext b").installed),
            ["b"]
        );
        // Without a registry, the installed group still shows.
        assert_eq!(ids(&rows(None, &idx, &updates, "").installed), ["a", "b"]);
    }

    #[test]
    fn fuzzy_matches_name_when_known() {
        let mut named = entry("zz-theme", "1.0.0");
        named.name = Some("Catppuccin".into());
        named.description = Some("Soothing pastel theme".into());
        let reg = registry(vec![
            entry("catppuccin-icons", "1.0.0"),
            named,
            entry("x", "1.0.0"),
        ]);
        let idx = ExtensionIndex::default();

        let found = rows(Some(&reg), &idx, &[], "catppuccin");
        assert_eq!(ids(&found.available), ["catppuccin-icons", "zz-theme"]);
        let by_description = rows(Some(&reg), &idx, &[], "pastel");
        assert_eq!(ids(&by_description.available), ["zz-theme"]);
        assert_eq!(by_description.available[0].title(), "Catppuccin");
        assert!(
            rows(Some(&reg), &idx, &[], "nothing-like-it")
                .available
                .is_empty()
        );
    }

    #[test]
    fn summary_text() {
        let mut ext = installed("a", "1.0.0");
        ext.themes = vec![theme("A Dark"), theme("A Light")];
        ext.languages = vec![language("A")];
        ext.snippets = BTreeMap::from([("a".to_string(), 10), ("*".to_string(), 4)]);
        assert_eq!(
            summary(&ext),
            "2 themes · 1 language · 14 snippets (Phase 4)"
        );
        let idx = index(vec![("a", ext)]);
        assert_eq!(
            rows(None, &idx, &[], "").installed[0].summary,
            "2 themes · 1 language · 14 snippets (Phase 4)"
        );
        assert_eq!(summary(&installed("b", "1.0.0")), "");
    }

    #[test]
    fn lsp_note_from_manifest() {
        let mut with_lsp = installed("a", "1.0.0");
        with_lsp.manifest = toml::from_str(
            "id = \"a\"\nname = \"A\"\nversion = \"1.0.0\"\nschema_version = 1\n\
             [language_servers.a-ls]\nlanguages = [\"A\"]\n",
        )
        .unwrap();
        let idx = index(vec![("a", with_lsp), ("b", installed("b", "1.0.0"))]);
        let all = rows(None, &idx, &[], "");
        assert!(all.installed[0].lsp_note);
        assert!(!all.installed[1].lsp_note);
    }

    #[test]
    fn installing_state_shows_stage() {
        let reg = registry(vec![entry("zig", "0.1.0")]);
        let mut all = rows(Some(&reg), &ExtensionIndex::default(), &[], "");
        let stage = Stage::BuildingGrammar {
            name: "zig".into(),
            index: 1,
            total: 3,
        };
        let installing = stage.clone();
        all.overlay(|row| (row.id == "zig").then(|| RowState::Installing(installing.clone())));
        assert_eq!(
            all.get("zig").unwrap().state,
            RowState::Installing(stage.clone())
        );
        assert_eq!(stage_label(&stage), "Building grammar 2/3: zig");
        assert_eq!(stage_label(&Stage::Fetching), "Fetching source");
    }

    #[test]
    fn broken_extension_keeps_its_reason() {
        let mut broken = installed("a", "1.0.0");
        broken.broken = Some("extension.toml is missing".into());
        let idx = index(vec![("a", broken)]);
        assert_eq!(
            rows(None, &idx, &["a".into()], "").installed[0].state,
            RowState::Broken("extension.toml is missing".into())
        );
    }

    #[test]
    fn filter_buttons_replace_tokens_and_keep_text() {
        assert_eq!(with_filter("", Some(Filter::Installed)), "@installed");
        assert_eq!(
            with_filter("@themes foo  bar", Some(Filter::Icons)),
            "@icons foo bar"
        );
        assert_eq!(with_filter("foo @updates", None), "foo");
        assert_eq!(with_filter("@installed @themes", None), "");
        assert_eq!(selected_filter("foo"), Some(None));
        assert_eq!(
            selected_filter("@languages foo"),
            Some(Some(Filter::Languages))
        );
        assert_eq!(selected_filter("@installed @themes"), None);
        for filter in [
            Filter::Installed,
            Filter::Updates,
            Filter::Themes,
            Filter::Icons,
            Filter::Languages,
        ] {
            assert_eq!(Filter::parse(filter.token()), Some(filter));
        }
    }

    #[test]
    fn unknown_at_token_is_text() {
        assert_eq!(
            parse_query("@installed foo @bar"),
            Query {
                filters: vec![Filter::Installed],
                text: "foo @bar".into()
            }
        );
    }
}
