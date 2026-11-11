//! Document symbols (spec §7.6): where they come from ([`SymbolSource`])
//! and Quick Open `@`, which lists them with a preview in the editor.

pub(crate) mod breadcrumbs;
pub(crate) mod quick_open;
mod source;

pub use breadcrumbs::{
    BREADCRUMBS_BATCH, BreadcrumbMode, FilePathMode, Segment, SymbolPathMode, breadcrumb_segments,
};
pub use source::{EMPTY_SYMBOLS_TTL, SYMBOLS_DEBOUNCE, SymbolSource, SymbolState};

use std::path::Path;

use gpui::Context;
use lucerna_syntax::SymbolKind;

use crate::{CommandRegistry, Workbench, icons};
use quick_open::SymbolsContext;

pub(crate) fn register_commands(commands: &mut CommandRegistry) {
    commands.register(
        "workbench.action.gotoSymbol",
        "Go to Symbol in Editor…",
        |wb, _, window, cx| wb.show_quick_open("@", window, cx),
    );
    commands.register(
        "breadcrumbs.focusAndSelect",
        "Focus and Select Breadcrumbs",
        |wb, _, window, cx| wb.show_quick_open("@", window, cx),
    );
}

/// The plural a `@:` group header uses, e.g. `functions (12)`.
pub(crate) fn kind_plural(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::File => "files",
        SymbolKind::Module => "modules",
        SymbolKind::Namespace => "namespaces",
        SymbolKind::Package => "packages",
        SymbolKind::Class => "classes",
        SymbolKind::Method => "methods",
        SymbolKind::Property => "properties",
        SymbolKind::Field => "fields",
        SymbolKind::Constructor => "constructors",
        SymbolKind::Enum => "enums",
        SymbolKind::Interface => "interfaces",
        SymbolKind::Function => "functions",
        SymbolKind::Variable => "variables",
        SymbolKind::Constant => "constants",
        SymbolKind::String => "strings",
        SymbolKind::Number => "numbers",
        SymbolKind::Boolean => "booleans",
        SymbolKind::Array => "arrays",
        SymbolKind::Object => "objects",
        SymbolKind::Key => "keys",
        SymbolKind::Null => "nulls",
        SymbolKind::EnumMember => "enum members",
        SymbolKind::Struct => "structs",
        SymbolKind::Event => "events",
        SymbolKind::Operator => "operators",
        SymbolKind::TypeParameter => "type parameters",
    }
}

/// The Lucide icon of a kind; one icon per group of kinds.
pub(crate) fn kind_icon(kind: SymbolKind) -> &'static [u8] {
    match kind {
        SymbolKind::Function | SymbolKind::Method | SymbolKind::Constructor => {
            icons::SQUARE_FUNCTION
        }
        SymbolKind::Class | SymbolKind::Struct => icons::BOX,
        SymbolKind::Interface => icons::COMPONENT,
        SymbolKind::Enum | SymbolKind::EnumMember => icons::LIST,
        SymbolKind::Field | SymbolKind::Property => icons::BRACKETS,
        SymbolKind::Variable => icons::VARIABLE,
        SymbolKind::Constant => icons::HASH,
        SymbolKind::Module | SymbolKind::Namespace | SymbolKind::Package | SymbolKind::File => {
            icons::PACKAGE
        }
        SymbolKind::TypeParameter => icons::TYPE,
        _ => icons::CIRCLE,
    }
}

impl Workbench {
    /// The active editor, its symbols and cursor, for Quick Open `@`.
    pub(crate) fn symbols_context(&mut self, cx: &mut Context<Self>) -> Option<SymbolsContext> {
        let editor = self.active_code_view(cx)?;
        let path = self.absolute_path(editor.read(cx).path());
        let cursor = editor.read(cx).cursor();
        let state = self.symbols.get(&path, cx);
        Some(SymbolsContext {
            path,
            editor,
            state,
            cursor,
        })
    }

    /// Hands the symbols of `path` to Quick Open when it lists them, and
    /// to the breadcrumbs.
    pub(crate) fn symbols_changed(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.breadcrumbs_changed(cx);
        let Some((qo, _)) = &self.quick_open else {
            return;
        };
        let qo = qo.clone();
        let path = self.absolute_path(path);
        if qo.read(cx).symbols_path() != Some(&path) {
            return;
        }
        let state = self.symbols.get(&path, cx);
        qo.update(cx, |qo, cx| qo.set_symbol_state(state, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_plural_and_icon() {
        for kind in SymbolKind::ALL {
            assert!(!kind_plural(kind).is_empty());
            assert!(!kind_icon(kind).is_empty());
        }
        assert_eq!(kind_plural(SymbolKind::Class), "classes");
        assert_eq!(kind_plural(SymbolKind::Property), "properties");
    }
}
