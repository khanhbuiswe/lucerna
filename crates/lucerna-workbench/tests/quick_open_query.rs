//! Quick Open's input prefixes: files, `>` commands, `:` go to line, `@`
//! document symbols and `#` workspace symbols.

use lucerna_workbench::{QuickOpenQuery, parse_query};

#[test]
fn plain_is_files() {
    assert_eq!(
        parse_query("main.rs"),
        QuickOpenQuery::Files("main.rs".into())
    );
    assert_eq!(parse_query(""), QuickOpenQuery::Files(String::new()));
    assert_eq!(parse_query("  src  "), QuickOpenQuery::Files("src".into()));
}

#[test]
fn gt_is_commands_trimmed() {
    assert_eq!(
        parse_query(">  toggle sidebar "),
        QuickOpenQuery::Commands("toggle sidebar".into())
    );
    assert_eq!(parse_query(">"), QuickOpenQuery::Commands(String::new()));
}

#[test]
fn colon_line() {
    assert_eq!(
        parse_query(":12"),
        QuickOpenQuery::GotoLine {
            line: Some(12),
            column: None
        }
    );
    assert_eq!(
        parse_query(": 7 "),
        QuickOpenQuery::GotoLine {
            line: Some(7),
            column: None
        }
    );
}

#[test]
fn colon_line_col() {
    assert_eq!(
        parse_query(":12:5"),
        QuickOpenQuery::GotoLine {
            line: Some(12),
            column: Some(5)
        }
    );
}

#[test]
fn colon_garbage_none() {
    let none = QuickOpenQuery::GotoLine {
        line: None,
        column: None,
    };
    assert_eq!(parse_query(":"), none);
    assert_eq!(parse_query(":abc"), none);
    assert_eq!(parse_query(":x:y"), none);
    assert_eq!(
        parse_query(":4:zz"),
        QuickOpenQuery::GotoLine {
            line: Some(4),
            column: None
        }
    );
    assert_eq!(
        parse_query(":-3"),
        QuickOpenQuery::GotoLine {
            line: None,
            column: None
        }
    );
}

#[test]
fn at_prefix_parses() {
    assert_eq!(
        parse_query("@ foo bar "),
        QuickOpenQuery::Symbols {
            text: "foo bar".into(),
            group_by_kind: false
        }
    );
    assert_eq!(
        parse_query("@"),
        QuickOpenQuery::Symbols {
            text: String::new(),
            group_by_kind: false
        }
    );
    assert_eq!(
        parse_query("#  Widget "),
        QuickOpenQuery::WorkspaceSymbols("Widget".into())
    );
}

#[test]
fn at_colon_groups() {
    assert_eq!(
        parse_query("@:"),
        QuickOpenQuery::Symbols {
            text: String::new(),
            group_by_kind: true
        }
    );
    assert_eq!(
        parse_query("@: new"),
        QuickOpenQuery::Symbols {
            text: "new".into(),
            group_by_kind: true
        }
    );
}
