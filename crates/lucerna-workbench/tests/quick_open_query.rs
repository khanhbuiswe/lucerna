//! Quick Open's input prefixes: files, `>` commands, `:` go to line, `@`
//! document symbols and `#` workspace symbols.

use lucerna_workbench::{QuickOpenQuery, parse_query, parse_query_vim, resolve_line};

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
            line: Some(12i64),
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
            line: Some(12i64),
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
            line: Some(-3),
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

#[test]
fn colon_separators_and_negative_lines() {
    let g = |line, column| QuickOpenQuery::GotoLine { line, column };
    assert_eq!(parse_query(":-1"), g(Some(-1), None));
    assert_eq!(parse_query(":12,5"), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12#5"), g(Some(12), Some(5)));
    assert_eq!(parse_query(": 12 , 5 "), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12:5:9"), g(Some(12), Some(5)));
    assert_eq!(parse_query(":12,"), g(Some(12), None));
    assert_eq!(parse_query(":,5"), g(None, Some(5)));
    assert_eq!(parse_query(":5:-1"), g(Some(5), None));
    assert_eq!(parse_query(":99999999999999999999"), g(None, None));
}

#[test]
fn resolve_line_counts_from_the_end_and_clamps() {
    assert_eq!(resolve_line(-1, 100), 100);
    assert_eq!(resolve_line(-100, 100), 1);
    assert_eq!(resolve_line(-500, 100), 1);
    assert_eq!(resolve_line(0, 100), 1);
    assert_eq!(resolve_line(50, 100), 50);
    assert_eq!(resolve_line(999, 100), 100);
    assert_eq!(resolve_line(i64::MIN, 100), 1);
    assert_eq!(resolve_line(-1, 0), 1);
}

#[test]
fn vim_colon_is_a_command_line() {
    assert_eq!(
        parse_query_vim(":%s/a/b/"),
        QuickOpenQuery::VimCommand("%s/a/b/".into())
    );
    assert_eq!(
        parse_query_vim(":  w"),
        QuickOpenQuery::VimCommand("w".into())
    );
    assert_eq!(
        parse_query_vim(":"),
        QuickOpenQuery::VimCommand(String::new())
    );
    // Other prefixes mean what they mean with Vim off.
    for input in ["@x", "#y", ">z", "main.rs", ""] {
        assert_eq!(parse_query_vim(input), parse_query(input), "{input}");
    }
    // With Vim off, `:` is still go to line.
    assert_eq!(
        parse_query(":-1"),
        QuickOpenQuery::GotoLine {
            line: Some(-1),
            column: None
        }
    );
}
