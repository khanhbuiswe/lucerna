//! Built-in language server adapters for servers found on `PATH`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::Value;

/// Which marker directory wins when several enclose a file.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum RootSearch {
    /// The nearest directory containing any marker.
    #[default]
    Nearest,
    /// The outermost directory containing any marker.
    Outermost,
    /// Markers are ranked by their order: the nearest directory containing
    /// the first marker wins, then the second, and so on.
    #[serde(rename = "ordered")]
    NearestByMarker,
}

pub trait Adapter: Send + Sync + 'static {
    /// The server key used in settings, e.g. `rust-analyzer`.
    fn name(&self) -> &'static str;
    /// Lucerna language ids this server handles.
    fn language_ids(&self) -> &'static [&'static str];
    /// Executable name looked up on `PATH`.
    fn binary(&self) -> &'static str;
    fn args(&self) -> &'static [&'static str];
    fn root_markers(&self) -> &'static [&'static str];
    fn root_search(&self) -> RootSearch {
        RootSearch::Nearest
    }
    /// Arguments of the check run before use; empty means do not probe.
    fn probe_args(&self) -> &'static [&'static str] {
        &["--version"]
    }
    /// The language id sent in `didOpen`.
    fn lsp_language_id(&self, lucerna_id: &str) -> String {
        lucerna_id.to_string()
    }
    fn initialization_options(&self) -> Option<Value> {
        None
    }
    fn enabled_by_default(&self) -> bool {
        true
    }
}

struct Static {
    name: &'static str,
    language_ids: &'static [&'static str],
    binary: &'static str,
    args: &'static [&'static str],
    root_markers: &'static [&'static str],
    root_search: RootSearch,
    probe_args: &'static [&'static str],
    enabled_by_default: bool,
}

impl Adapter for Static {
    fn name(&self) -> &'static str {
        self.name
    }
    fn language_ids(&self) -> &'static [&'static str] {
        self.language_ids
    }
    fn binary(&self) -> &'static str {
        self.binary
    }
    fn args(&self) -> &'static [&'static str] {
        self.args
    }
    fn root_markers(&self) -> &'static [&'static str] {
        self.root_markers
    }
    fn root_search(&self) -> RootSearch {
        self.root_search
    }
    fn probe_args(&self) -> &'static [&'static str] {
        self.probe_args
    }
    fn enabled_by_default(&self) -> bool {
        self.enabled_by_default
    }
}

const PYTHON_MARKERS: &[&str] = &[
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "pyrightconfig.json",
];

pub fn builtin_adapters() -> Vec<Arc<dyn Adapter>> {
    let nearest = Static {
        name: "",
        language_ids: &[],
        binary: "",
        args: &[],
        root_markers: &[],
        root_search: RootSearch::Nearest,
        probe_args: &["--version"],
        enabled_by_default: true,
    };
    let all = vec![
        Static {
            name: "rust-analyzer",
            language_ids: &["rust"],
            binary: "rust-analyzer",
            root_markers: &["Cargo.toml"],
            root_search: RootSearch::Outermost,
            ..nearest
        },
        Static {
            name: "gopls",
            language_ids: &["go"],
            binary: "gopls",
            root_markers: &["go.work", "go.mod"],
            root_search: RootSearch::NearestByMarker,
            probe_args: &["version"],
            ..nearest
        },
        Static {
            name: "basedpyright",
            language_ids: &["python"],
            binary: "basedpyright-langserver",
            args: &["--stdio"],
            root_markers: PYTHON_MARKERS,
            probe_args: &[],
            ..nearest
        },
        Static {
            name: "pyright",
            language_ids: &["python"],
            binary: "pyright-langserver",
            args: &["--stdio"],
            root_markers: PYTHON_MARKERS,
            probe_args: &[],
            enabled_by_default: false,
            ..nearest
        },
        Static {
            name: "typescript-language-server",
            language_ids: &[
                "typescript",
                "typescriptreact",
                "javascript",
                "javascriptreact",
            ],
            binary: "typescript-language-server",
            args: &["--stdio"],
            root_markers: &["tsconfig.json", "jsconfig.json", "package.json"],
            ..nearest
        },
        Static {
            name: "clangd",
            language_ids: &["c", "cpp"],
            binary: "clangd",
            root_markers: &["compile_commands.json", ".clangd"],
            ..nearest
        },
    ];
    all.into_iter().map(|a| Arc::new(a) as _).collect()
}

/// The project root `adapter` uses for `file`. Never above `workspace_root`;
/// the workspace root when no marker is found. `None` when the file lies
/// outside the workspace: no server is ever started for such a root (it is
/// not trusted, and a library file would index a whole registry); the file
/// joins an already running server instead.
pub fn find_root(adapter: &dyn Adapter, file: &Path, workspace_root: &Path) -> Option<PathBuf> {
    let parent = file.parent().unwrap_or(file);
    if !parent.starts_with(workspace_root) {
        return None;
    }
    // From the file's directory up to and including the workspace root.
    let dirs: Vec<&Path> = parent
        .ancestors()
        .take_while(|dir| dir.starts_with(workspace_root))
        .collect();
    let markers = adapter.root_markers();
    let has = |dir: &Path, marker: &str| dir.join(marker).exists();
    let found = match adapter.root_search() {
        RootSearch::Nearest => dirs.iter().find(|dir| markers.iter().any(|m| has(dir, m))),
        RootSearch::Outermost => dirs
            .iter()
            .rev()
            .find(|dir| markers.iter().any(|m| has(dir, m))),
        RootSearch::NearestByMarker => markers
            .iter()
            .find_map(|m| dirs.iter().find(|dir| has(dir, m))),
    };
    Some(found.map_or_else(|| workspace_root.to_path_buf(), |dir| dir.to_path_buf()))
}
