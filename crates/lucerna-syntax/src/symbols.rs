//! The symbols of a document as a tree (spec §3.1 `symbols`): what a
//! language server's `documentSymbol` answers, or what an `outline.scm`
//! query finds. Free of `lsp-types`, so both sources share it.

use std::ops::Range;

use lucerna_text::Point;

/// The kind of a symbol, in the order and with the meaning of LSP's
/// `SymbolKind` (1 = `File` … 26 = `TypeParameter`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SymbolKind {
    File,
    Module,
    Namespace,
    Package,
    Class,
    Method,
    Property,
    Field,
    Constructor,
    Enum,
    Interface,
    Function,
    Variable,
    Constant,
    String,
    Number,
    Boolean,
    Array,
    Object,
    Key,
    Null,
    EnumMember,
    Struct,
    Event,
    Operator,
    TypeParameter,
}

impl SymbolKind {
    /// Every kind, in LSP order: `ALL[n - 1]` is LSP kind `n`.
    pub const ALL: [SymbolKind; 26] = [
        SymbolKind::File,
        SymbolKind::Module,
        SymbolKind::Namespace,
        SymbolKind::Package,
        SymbolKind::Class,
        SymbolKind::Method,
        SymbolKind::Property,
        SymbolKind::Field,
        SymbolKind::Constructor,
        SymbolKind::Enum,
        SymbolKind::Interface,
        SymbolKind::Function,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::String,
        SymbolKind::Number,
        SymbolKind::Boolean,
        SymbolKind::Array,
        SymbolKind::Object,
        SymbolKind::Key,
        SymbolKind::Null,
        SymbolKind::EnumMember,
        SymbolKind::Struct,
        SymbolKind::Event,
        SymbolKind::Operator,
        SymbolKind::TypeParameter,
    ];
}

/// One symbol. `range` spans all of it (body, comments); `selection_range`
/// is the part to reveal and put the cursor at, usually the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub range: Range<Point>,
    pub selection_range: Range<Point>,
    pub children: Vec<Symbol>,
}

/// A symbol as [`SymbolTree::flatten`] lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatSymbol<'a> {
    /// 0 for top-level symbols.
    pub depth: usize,
    /// The names of the symbols containing it, outermost first.
    pub ancestors: Vec<&'a str>,
    pub symbol: &'a Symbol,
}

/// The top-level symbols of a document, in document order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SymbolTree(pub Vec<Symbol>);

/// Orders points; `Point` itself is not `Ord`.
fn key(p: Point) -> (u32, u32) {
    (p.row, p.column)
}

/// Whether `outer` contains `inner` (ends included).
fn contains(outer: &Range<Point>, inner: &Range<Point>) -> bool {
    key(outer.start) <= key(inner.start) && key(inner.end) <= key(outer.end)
}

impl SymbolTree {
    /// The symbols containing `p` (ends included), outermost first: the
    /// first containing symbol at each level, then its children.
    pub fn path_at(&self, p: Point) -> Vec<&Symbol> {
        let mut path = Vec::new();
        let mut level = &self.0;
        while let Some(symbol) = level.iter().find(|s| contains(&s.range, &(p..p))) {
            path.push(symbol);
            level = &symbol.children;
        }
        path
    }

    /// Every symbol, parents before their children, in document order.
    pub fn flatten(&self) -> Vec<FlatSymbol<'_>> {
        fn walk<'a>(
            symbols: &'a [Symbol],
            ancestors: &mut Vec<&'a str>,
            out: &mut Vec<FlatSymbol<'a>>,
        ) {
            for symbol in symbols {
                out.push(FlatSymbol {
                    depth: ancestors.len(),
                    ancestors: ancestors.clone(),
                    symbol,
                });
                ancestors.push(&symbol.name);
                walk(&symbol.children, ancestors, out);
                ancestors.pop();
            }
        }
        let mut out = Vec::new();
        walk(&self.0, &mut Vec::new(), &mut out);
        out
    }

    /// Nests `items` by their ranges: a symbol becomes a child of the
    /// smallest earlier one whose range contains its range. Symbols are
    /// sorted by start (the longer first on a tie); children they already
    /// have are kept.
    pub fn from_flat(mut items: Vec<Symbol>) -> SymbolTree {
        items.sort_by(|a, b| {
            key(a.range.start)
                .cmp(&key(b.range.start))
                .then(key(b.range.end).cmp(&key(a.range.end)))
        });
        let mut roots = Vec::new();
        // The open chain of nested symbols, outermost first.
        let mut stack: Vec<Symbol> = Vec::new();
        fn close(stack: &mut Vec<Symbol>, roots: &mut Vec<Symbol>) {
            if let Some(done) = stack.pop() {
                match stack.last_mut() {
                    Some(parent) => parent.children.push(done),
                    None => roots.push(done),
                }
            }
        }
        for item in items {
            while stack
                .last()
                .is_some_and(|top| !contains(&top.range, &item.range))
            {
                close(&mut stack, &mut roots);
            }
            stack.push(item);
        }
        while !stack.is_empty() {
            close(&mut stack, &mut roots);
        }
        SymbolTree(roots)
    }
}
