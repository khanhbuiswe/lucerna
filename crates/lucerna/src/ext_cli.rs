//! `luc ext`: manage extensions from the command line. Never starts GPUI.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use clap::{Args, Subcommand};
use lucerna_extension::builder::Stage;
use lucerna_extension::process::CancelToken;
use lucerna_extension::registry::{RegistryEntry, is_stale};
use lucerna_extension::store::{InstallProgress, Store, StoreError};
use lucerna_workbench::extensions::{WasmGrammarValidator, extensions_root};
use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Manage extensions.
#[derive(Debug, Args)]
pub struct ExtArgs {
    /// Registry repository to use instead of the default (tests).
    #[arg(long, hide = true, global = true)]
    registry_url: Option<String>,
    /// wasi-sdk to build grammars with instead of the managed one (tests).
    #[arg(long, hide = true, global = true)]
    wasi_sdk: Option<PathBuf>,
    #[command(subcommand)]
    action: ExtCommand,
}

#[derive(Debug, Subcommand)]
pub enum ExtCommand {
    /// Search the registry by id, name and description.
    Search { query: String },
    /// List registry extensions.
    List {
        /// Only installed extensions.
        #[arg(long)]
        installed: bool,
        /// Only extensions with a newer version available.
        #[arg(long)]
        updates: bool,
    },
    /// Install extensions from the registry, or a local one with --dev.
    Install {
        /// Extension ids.
        #[arg(required_unless_present = "dev", conflicts_with = "dev")]
        ids: Vec<String>,
        /// Build and install the extension in this directory.
        #[arg(long, value_name = "DIR")]
        dev: Option<PathBuf>,
    },
    /// Remove installed extensions.
    Remove {
        #[arg(required = true)]
        ids: Vec<String>,
    },
    /// Update extensions (all with updates available when no id is given).
    Update { ids: Vec<String> },
}

/// Runs the command and returns the process exit code.
pub fn run(args: ExtArgs) -> i32 {
    let mut store = Store::open(extensions_root(), Arc::new(WasmGrammarValidator));
    if let Some(url) = args.registry_url {
        store = store.with_remote(url);
    }
    if let Some(sdk) = args.wasi_sdk {
        store = store.with_wasi_sdk(sdk);
    }
    let cli = Cli {
        store,
        cancel: CancelToken::default(),
    };
    match args.action {
        ExtCommand::Search { query } => cli.search(&query),
        ExtCommand::List { installed, updates } => cli.list(installed, updates),
        ExtCommand::Install {
            dev: Some(dir),
            ids: _,
        } => cli.install_dev(&dir),
        ExtCommand::Install { ids, dev: None } => cli.install_all(&ids),
        ExtCommand::Remove { ids } => cli.remove(&ids),
        ExtCommand::Update { ids } => cli.update(&ids),
    }
}

struct Cli {
    store: Store,
    cancel: CancelToken,
}

fn stage_line(p: &InstallProgress) -> String {
    let what = match &p.stage {
        Stage::Fetching => "fetching source".to_string(),
        Stage::ReadingManifest => "reading manifest".to_string(),
        Stage::PreparingToolchain => "preparing toolchain".to_string(),
        Stage::BuildingGrammar { name, index, total } => {
            format!("building grammar {}/{total} ({name})", index + 1)
        }
        Stage::Validating => "validating".to_string(),
        Stage::Installing => "installing".to_string(),
    };
    format!("{}: {what}", p.id)
}

fn progress(p: InstallProgress) {
    eprintln!("{}", stage_line(&p));
}

fn warn_all(warnings: &[String]) {
    for w in warnings {
        eprintln!("warning: {w}");
    }
}

fn line(id: &str, version: &str, description: Option<&str>) -> String {
    match description.filter(|d| !d.is_empty()) {
        Some(d) => format!("{id} {version}  {d}"),
        None => format!("{id} {version}"),
    }
}

impl Cli {
    /// Loads the registry, refreshing it when missing or stale. A failed
    /// refresh is a warning when a cached copy exists, an error otherwise.
    fn entries(&self) -> Result<Vec<RegistryEntry>, StoreError> {
        let stale = self
            .store
            .registry()
            .is_none_or(|r| is_stale(&r, SystemTime::now()));
        if stale {
            match self.store.refresh_registry(&self.cancel) {
                Ok(warnings) => warn_all(&warnings),
                Err(e) if self.store.registry().is_some() => {
                    eprintln!("warning: could not refresh the registry: {e}");
                }
                Err(e) => return Err(e),
            }
        }
        Ok(self.store.registry().map(|r| r.entries).unwrap_or_default())
    }

    fn fail(&self, what: &str, e: &StoreError) {
        eprintln!("error: {what}: {e}");
    }

    fn search(&self, query: &str) -> i32 {
        let entries = match self.entries() {
            Ok(e) => e,
            Err(e) => {
                self.fail("registry", &e);
                return 1;
            }
        };
        let mut matcher = Matcher::new(Config::DEFAULT);
        let pattern = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut buf = Vec::new();
        let mut best = |text: &str| {
            pattern
                .score(Utf32Str::new(text, &mut buf), &mut matcher)
                .unwrap_or(0)
        };
        let mut hits: Vec<(u32, &RegistryEntry)> = entries
            .iter()
            .filter_map(|e| {
                let score = [
                    Some(e.id.as_str()),
                    e.name.as_deref(),
                    e.description.as_deref(),
                ]
                .into_iter()
                .flatten()
                .map(&mut best)
                .max()
                .unwrap_or(0);
                (score > 0).then_some((score, e))
            })
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        for (_, e) in hits {
            println!("{}", line(&e.id, &e.version, e.description.as_deref()));
        }
        0
    }

    fn list(&self, installed: bool, updates: bool) -> i32 {
        let needs_registry = updates || !installed;
        let entries = if needs_registry {
            match self.entries() {
                Ok(e) => e,
                Err(e) => {
                    self.fail("registry", &e);
                    return 1;
                }
            }
        } else {
            Vec::new()
        };
        let update_ids = if updates {
            self.store.updates()
        } else {
            Vec::new()
        };
        if installed {
            let index = self.store.index();
            for (id, ext) in &index.extensions {
                if updates && !update_ids.contains(id) {
                    continue;
                }
                if let Some(why) = &ext.broken {
                    println!("{id} {}  (broken: {why})", ext.info.version);
                } else {
                    println!(
                        "{}",
                        line(id, &ext.info.version, ext.manifest.description.as_deref())
                    );
                }
            }
        } else {
            for e in &entries {
                if updates && !update_ids.contains(&e.id) {
                    continue;
                }
                println!("{}", line(&e.id, &e.version, e.description.as_deref()));
            }
        }
        0
    }

    fn install_one(&self, id: &str) -> bool {
        match self.store.install(id, &progress, &self.cancel) {
            Ok(warnings) => {
                warn_all(&warnings);
                true
            }
            Err(e) => {
                self.fail(id, &e);
                false
            }
        }
    }

    fn install_all(&self, ids: &[String]) -> i32 {
        if let Err(e) = self.entries() {
            self.fail("registry", &e);
            return 1;
        }
        exit_code(ids.iter().map(|id| self.install_one(id)))
    }

    fn install_dev(&self, dir: &std::path::Path) -> i32 {
        match self.store.install_dev(dir, &progress, &self.cancel) {
            Ok(warnings) => {
                warn_all(&warnings);
                0
            }
            Err(e) => {
                self.fail(&dir.display().to_string(), &e);
                1
            }
        }
    }

    fn remove(&self, ids: &[String]) -> i32 {
        exit_code(ids.iter().map(|id| match self.store.uninstall(id) {
            Ok(()) => true,
            Err(e) => {
                self.fail(id, &e);
                false
            }
        }))
    }

    fn update(&self, ids: &[String]) -> i32 {
        if let Err(e) = self.entries() {
            self.fail("registry", &e);
            return 1;
        }
        let targets = if ids.is_empty() {
            self.store.updates()
        } else {
            ids.to_vec()
        };
        let index = self.store.index();
        exit_code(targets.iter().map(|id| {
            if !index.extensions.contains_key(id) {
                eprintln!("error: {id}: not installed");
                return false;
            }
            self.install_one(id)
        }))
    }
}

/// 0 when every step succeeded (all steps run), otherwise 1.
fn exit_code(results: impl Iterator<Item = bool>) -> i32 {
    let mut failed = false;
    for ok in results {
        failed |= !ok;
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_lines_are_one_based() {
        let p = InstallProgress {
            id: "zig".into(),
            stage: Stage::BuildingGrammar {
                name: "zig".into(),
                index: 0,
                total: 1,
            },
        };
        assert_eq!(stage_line(&p), "zig: building grammar 1/1 (zig)");
    }

    #[test]
    fn exit_code_runs_all_and_fails_on_any() {
        assert_eq!(exit_code([true, true].into_iter()), 0);
        assert_eq!(exit_code([true, false, true].into_iter()), 1);
    }

    #[test]
    fn line_omits_empty_description() {
        assert_eq!(line("a", "1", None), "a 1");
        assert_eq!(line("a", "1", Some("")), "a 1");
        assert_eq!(line("a", "1", Some("d")), "a 1  d");
    }
}
