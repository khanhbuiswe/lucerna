use lucerna_settings::{Context, ContextValue, parse_when};

fn ctx(pairs: &[(&str, ContextValue)]) -> Context {
    let mut c = Context::default();
    for (k, v) in pairs {
        c.set(*k, v.clone());
    }
    c
}

fn b(v: bool) -> ContextValue {
    ContextValue::Bool(v)
}

#[test]
fn bare_key_truthy() {
    let e = parse_when("editorFocus").unwrap();
    assert!(e.eval(&ctx(&[("editorFocus", b(true))])));
    assert!(!e.eval(&ctx(&[("editorFocus", b(false))])));
    assert!(!e.eval(&Context::default()));
    let n = parse_when("!editorFocus").unwrap();
    assert!(n.eval(&Context::default()));
}

#[test]
fn not_and_or_precedence() {
    let e = parse_when("a || b && c").unwrap();
    let c = ctx(&[("a", b(false)), ("b", b(true)), ("c", b(false))]);
    assert!(!e.eval(&c));
    let c = ctx(&[("a", b(true)), ("b", b(false)), ("c", b(false))]);
    assert!(e.eval(&c));
}

#[test]
fn parens() {
    let e = parse_when("(a || b) && c").unwrap();
    let c = ctx(&[("a", b(true)), ("b", b(false)), ("c", b(false))]);
    assert!(!e.eval(&c));
    let c = ctx(&[("a", b(true)), ("b", b(false)), ("c", b(true))]);
    assert!(e.eval(&c));
}

#[test]
fn string_equality() {
    let c = ctx(&[
        ("resourceExtname", ContextValue::Str(".rs".into())),
        ("editorLangId", ContextValue::Str("rust".into())),
    ]);
    assert!(parse_when("resourceExtname == '.rs'").unwrap().eval(&c));
    assert!(!parse_when("resourceExtname != '.rs'").unwrap().eval(&c));
    assert!(!parse_when("editorLangId != rust").unwrap().eval(&c));
    // Unknown key compares as "".
    assert!(parse_when("missing != rust").unwrap().eval(&c));
    assert!(parse_when("missing == ''").unwrap().eval(&c));
}

#[test]
fn parse_error_reports() {
    assert!(parse_when("a &&").is_err());
    assert!(parse_when("(a").is_err());
    assert!(parse_when("a b").is_err());
    assert!(parse_when("").is_err());
}

#[test]
fn extensions_viewlet_focus_evaluates() {
    let e = parse_when("sideBarFocus && extensionsViewletFocus").unwrap();
    let on = ctx(&[
        ("sideBarFocus", b(true)),
        ("extensionsViewletFocus", b(true)),
    ]);
    assert!(e.eval(&on));
    assert!(!e.eval(&ctx(&[("sideBarFocus", b(true))])));
}
