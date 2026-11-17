//! Replace-string expansion, Preserve Case and replace-all edits, free of gpui.
//!
//! Behaviour follows VS Code `replacePattern.ts` and `search.ts` (MIT); the
//! code is written independently.

use std::ops::Range;

use regex::{Captures, Regex};

#[derive(Debug, Clone, Copy, PartialEq)]
enum CaseOp {
    /// `\u`: uppercase the first character.
    UpperFirst,
    /// `\l`: lowercase the first character.
    LowerFirst,
    /// `\U`: uppercase everything.
    UpperAll,
    /// `\L`: lowercase everything.
    LowerAll,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Text(String),
    Group {
        /// 0 is the whole match.
        index: usize,
        /// Source text (`$12`), used for the fallbacks when the group is missing.
        raw: String,
        ops: Vec<CaseOp>,
    },
}

/// A parsed replacement string.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplacePattern {
    pieces: Vec<Piece>,
}

impl ReplacePattern {
    /// Parse `text`. Without `is_regex` the text is used verbatim.
    pub fn parse(text: &str, is_regex: bool) -> ReplacePattern {
        if !is_regex {
            return ReplacePattern {
                pieces: vec![Piece::Text(text.to_string())],
            };
        }
        let chars: Vec<char> = text.chars().collect();
        let mut pieces: Vec<Piece> = Vec::new();
        let mut lit = String::new();
        let mut pending: Vec<CaseOp> = Vec::new();
        let mut i = 0;
        let flush = |lit: &mut String, pieces: &mut Vec<Piece>| {
            if !lit.is_empty() {
                pieces.push(Piece::Text(std::mem::take(lit)));
            }
        };
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            match (c, next) {
                ('\\', Some(n)) => {
                    i += 2;
                    match n {
                        'n' => lit.push('\n'),
                        't' => lit.push('\t'),
                        '\\' => lit.push('\\'),
                        'u' => pending.push(CaseOp::UpperFirst),
                        'l' => pending.push(CaseOp::LowerFirst),
                        'U' => pending.push(CaseOp::UpperAll),
                        'L' => pending.push(CaseOp::LowerAll),
                        other => {
                            lit.push('\\');
                            lit.push(other);
                        }
                    }
                }
                ('$', Some('$')) => {
                    lit.push('$');
                    i += 2;
                }
                ('$', Some('&')) => {
                    flush(&mut lit, &mut pieces);
                    pieces.push(Piece::Group {
                        index: 0,
                        raw: "$&".into(),
                        ops: std::mem::take(&mut pending),
                    });
                    i += 2;
                }
                ('$', Some(d)) if d.is_ascii_digit() => {
                    let mut raw = String::from('$');
                    raw.push(d);
                    let mut index = d.to_digit(10).unwrap() as usize;
                    i += 2;
                    if let Some(d2) = chars.get(i).copied().filter(char::is_ascii_digit) {
                        raw.push(d2);
                        index = index * 10 + d2.to_digit(10).unwrap() as usize;
                        i += 1;
                    }
                    flush(&mut lit, &mut pieces);
                    pieces.push(Piece::Group {
                        index,
                        raw,
                        ops: std::mem::take(&mut pending),
                    });
                }
                _ => {
                    lit.push(c);
                    i += 1;
                }
            }
        }
        flush(&mut lit, &mut pieces);
        ReplacePattern { pieces }
    }

    /// True when the output never depends on the match.
    pub fn is_static(&self) -> bool {
        self.pieces.iter().all(|p| matches!(p, Piece::Text(_)))
    }

    /// Expand against one match.
    pub fn expand(&self, caps: &Captures<'_>) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(t) => out.push_str(t),
                Piece::Group { index, raw, ops } => {
                    let group = |n: usize| caps.get(n).map_or("", |m| m.as_str());
                    let text = if *index < caps.len() {
                        group(*index).to_string()
                    } else if raw.len() == 3 {
                        // `$NN` with no group NN: group N followed by the digit.
                        let first = raw.as_bytes()[1] - b'0';
                        if (first as usize) < caps.len() {
                            format!("{}{}", group(first as usize), &raw[2..])
                        } else {
                            raw.clone()
                        }
                    } else {
                        raw.clone()
                    };
                    out.push_str(&apply_ops(ops, &text));
                }
            }
        }
        out
    }
}

fn apply_ops(ops: &[CaseOp], s: &str) -> String {
    if ops.is_empty() {
        return s.to_string();
    }
    let mut text = s.to_string();
    for op in ops {
        match op {
            CaseOp::UpperAll => text = text.to_uppercase(),
            CaseOp::LowerAll => text = text.to_lowercase(),
            _ => {}
        }
    }
    for op in ops {
        let upper = match op {
            CaseOp::UpperFirst => true,
            CaseOp::LowerFirst => false,
            _ => continue,
        };
        let mut it = text.chars();
        if let Some(first) = it.next() {
            let head: String = if upper {
                first.to_uppercase().collect()
            } else {
                first.to_lowercase().collect()
            };
            text = head + it.as_str();
        }
    }
    text
}

/// Preserve Case: shape `replacement` like `matched`.
pub fn preserve_case(matched: &str, replacement: &str) -> String {
    for sep in ['-', '_'] {
        if matched.contains(sep) && replacement.contains(sep) {
            let ms: Vec<&str> = matched.split(sep).collect();
            let rs: Vec<&str> = replacement.split(sep).collect();
            if ms.len() == rs.len() {
                let parts: Vec<String> = ms
                    .iter()
                    .zip(&rs)
                    .map(|(m, r)| preserve_case(m, r))
                    .collect();
                return parts.join(&sep.to_string());
            }
        }
    }
    let Some(first) = matched.chars().next() else {
        return replacement.to_string();
    };
    if matched.to_uppercase() == matched {
        replacement.to_uppercase()
    } else if matched.to_lowercase() == matched {
        replacement.to_lowercase()
    } else if first.is_uppercase() {
        apply_ops(&[CaseOp::UpperFirst], replacement)
    } else if first.is_lowercase() {
        apply_ops(&[CaseOp::LowerFirst], replacement)
    } else {
        replacement.to_string()
    }
}

/// Expand, then apply Preserve Case against the whole match when `preserve`.
pub fn replacement_for(pattern: &ReplacePattern, caps: &Captures<'_>, preserve: bool) -> String {
    let expanded = pattern.expand(caps);
    if preserve {
        preserve_case(caps.get(0).map_or("", |m| m.as_str()), &expanded)
    } else {
        expanded
    }
}

/// One edit per non-empty match, in order.
pub fn replace_all(
    text: &str,
    regex: &Regex,
    pattern: &ReplacePattern,
    preserve: bool,
) -> Vec<(Range<usize>, String)> {
    if pattern.is_static() && !preserve {
        let caps_free: String = pattern
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Text(t) => t.as_str(),
                Piece::Group { .. } => "",
            })
            .collect();
        return regex
            .find_iter(text)
            .filter(|m| !m.is_empty())
            .map(|m| (m.range(), caps_free.clone()))
            .collect();
    }
    regex
        .captures_iter(text)
        .filter_map(|caps| {
            let m = caps.get(0)?;
            if m.is_empty() {
                return None;
            }
            Some((m.range(), replacement_for(pattern, &caps, preserve)))
        })
        .collect()
}

/// The replacement for the match at exactly `range`, or `None` if the regex
/// does not match there.
pub fn replacement_at(
    text: &str,
    regex: &Regex,
    range: Range<usize>,
    pattern: &ReplacePattern,
    preserve: bool,
) -> Option<String> {
    let caps = regex.captures_at(text, range.start)?;
    if caps.get(0)?.range() != range {
        return None;
    }
    Some(replacement_for(pattern, &caps, preserve))
}

#[cfg(test)]
mod tests {
    use super::*;
    use regex::RegexBuilder;

    fn exp(pat: &str, re: &str, hay: &str) -> String {
        let p = ReplacePattern::parse(pat, true);
        p.expand(&Regex::new(re).unwrap().captures(hay).unwrap())
    }

    #[test]
    fn literal_mode_is_verbatim() {
        let p = ReplacePattern::parse(r"$1\n", false);
        assert!(p.is_static());
        assert_eq!(
            replace_all("ab", &Regex::new("a").unwrap(), &p, false),
            vec![(0..1, r"$1\n".to_string())]
        );
    }
    #[test]
    fn dollar_forms() {
        assert_eq!(exp("[$0][$&]", "b+", "abbc"), "[bb][bb]");
        assert_eq!(exp("$2-$1", "(a)(b)", "ab"), "b-a");
        assert_eq!(exp("$$1", "(a)", "a"), "$1");
        assert_eq!(exp("$1a", "(x)", "x"), "xa");
        assert_eq!(exp("$10", "(x)", "x"), "x0");
        assert_eq!(exp("$5", "(a)(b)", "ab"), "$5");
        assert_eq!(exp("$1", "(a)|(b)", "b"), "");
        assert_eq!(exp("a$", "x", "x"), "a$");
        assert_eq!(exp("$x", "x", "x"), "$x");
    }
    #[test]
    fn escapes() {
        assert_eq!(exp(r"a\nb\tc\\d", "x", "x"), "a\nb\tc\\d");
        assert_eq!(exp(r"\q", "x", "x"), r"\q");
        assert_eq!(exp(r"a\", "x", "x"), r"a\");
    }
    #[test]
    fn case_ops() {
        assert_eq!(exp(r"\u$1", "(foo)", "foo"), "Foo");
        assert_eq!(exp(r"\U$1-$1", "(foo)", "foo"), "FOO-foo");
        assert_eq!(exp(r"\l$1", "(FOO)", "FOO"), "fOO");
        assert_eq!(exp(r"\L$0", "FOO", "FOO"), "foo");
        assert_eq!(exp(r"\u\L$1", "(fOO)", "fOO"), "Foo");
        assert_eq!(exp(r"\u", "x", "x"), "");
        assert_eq!(exp(r"\Uab$1", "(c)", "c"), "abC");
    }
    #[test]
    fn preserve_case_rules() {
        assert_eq!(preserve_case("FOO", "bar"), "BAR");
        assert_eq!(preserve_case("foo", "BaR"), "bar");
        assert_eq!(preserve_case("Foo", "bar"), "Bar");
        assert_eq!(preserve_case("fOO", "Bar"), "bar");
        assert_eq!(preserve_case("Foo-Bar", "baz-qux"), "Baz-Qux");
        assert_eq!(preserve_case("FOO_bar", "baz_qux"), "BAZ_qux");
        assert_eq!(preserve_case("Foo-Bar", "baz"), "Baz");
        assert_eq!(preserve_case("123", "bar"), "BAR");
        assert_eq!(preserve_case("", "bar"), "bar");
    }
    #[test]
    fn preserve_case_unicode() {
        assert_eq!(preserve_case("Đường", "phố"), "Phố");
        assert_eq!(preserve_case("ĐƯỜNG", "phố"), "PHỐ");
    }
    #[test]
    fn replace_all_skips_empty_matches() {
        let p = ReplacePattern::parse("-", true);
        assert_eq!(
            replace_all("ab", &Regex::new("x*").unwrap(), &p, false),
            vec![]
        );
    }
    #[test]
    fn replace_all_multibyte() {
        let p = ReplacePattern::parse("[$0]", true);
        let edits = replace_all("ê a ê", &Regex::new("ê").unwrap(), &p, false);
        assert_eq!(edits, vec![(0..2, "[ê]".into()), (5..7, "[ê]".into())]);
    }
    #[test]
    fn replace_all_with_preserve_case() {
        let p = ReplacePattern::parse("bar", false);
        let re = RegexBuilder::new("foo")
            .case_insensitive(true)
            .build()
            .unwrap();
        let edits = replace_all("foo Foo FOO", &re, &p, true);
        assert_eq!(
            edits.iter().map(|e| e.1.as_str()).collect::<Vec<_>>(),
            ["bar", "Bar", "BAR"]
        );
    }
    #[test]
    fn replacement_at_checks_range() {
        let p = ReplacePattern::parse("<$1>", true);
        let re = Regex::new("(o+)").unwrap();
        assert_eq!(
            replacement_at("foo boo", &re, 5..7, &p, false),
            Some("<oo>".into())
        );
        assert_eq!(replacement_at("foo boo", &re, 4..7, &p, false), None);
    }
}
