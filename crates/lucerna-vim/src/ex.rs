//! The `:` command line: parsing a line into a range and a command, and
//! resolving the range against a buffer. Running the command lives elsewhere.

use std::ops::RangeInclusive;

use crate::marks::FileMarks;

/// What an address counts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Base {
    /// A 1-based line number.
    Number(u32),
    /// `.`
    Current,
    /// `$`
    Last,
    /// `'x`
    Mark(char),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Address {
    pub base: Base,
    pub offset: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineRange {
    /// No range typed.
    Implicit,
    /// `%`
    All,
    One(Address),
    Two(Address, Address),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct SubFlags {
    pub global: bool,
    /// `i` is `Some(true)`, `I` is `Some(false)`.
    pub ignore_case: Option<bool>,
    pub report_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubstituteArgs {
    /// `:s`, `:s g`, `:s 3`: repeat the last substitute with these flags.
    Repeat { flags: SubFlags, count: Option<u32> },
    New {
        pattern: String,
        replacement: String,
        flags: SubFlags,
        count: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Command {
    /// A range alone.
    Goto,
    Write {
        bang: bool,
    },
    WriteAll,
    Quit {
        bang: bool,
    },
    QuitAll,
    /// `:wq`
    WriteQuit {
        bang: bool,
    },
    /// `:x`
    Exit {
        bang: bool,
    },
    Delete {
        register: Option<char>,
        count: Option<u32>,
    },
    Yank {
        register: Option<char>,
        count: Option<u32>,
    },
    Shift {
        right: bool,
        depth: u32,
        count: Option<u32>,
    },
    Substitute(SubstituteArgs),
    NoHighlight,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExLine {
    pub range: LineRange,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExError {
    NotACommand(String),
    InvalidRange,
    MarkNotSet,
    NoBangAllowed,
    NoRangeAllowed,
    TrailingCharacters(String),
    PatternNotFound(String),
    InvalidPattern(String),
    NoPreviousPattern,
    InvalidRegister(char),
}

impl ExError {
    /// The message with its Vim code, e.g. `E492: Not an editor command: foo`.
    pub(crate) fn message(&self) -> String {
        let code = match self {
            ExError::NotACommand(_) => "E492",
            ExError::InvalidRange => "E16",
            ExError::MarkNotSet => "E20",
            ExError::NoBangAllowed => "E477",
            ExError::NoRangeAllowed => "E481",
            ExError::TrailingCharacters(_) => "E488",
            ExError::PatternNotFound(_) => "E486",
            ExError::InvalidPattern(_) => "E383",
            ExError::NoPreviousPattern => "E35",
            ExError::InvalidRegister(_) => "E354",
        };
        format!("{code}: {}", self.short())
    }

    /// The message without its code, e.g. `Not an editor command: foo`.
    pub(crate) fn short(&self) -> String {
        match self {
            ExError::NotACommand(s) => format!("Not an editor command: {s}"),
            ExError::InvalidRange => "Invalid range".into(),
            ExError::MarkNotSet => "Mark not set".into(),
            ExError::NoBangAllowed => "No ! allowed".into(),
            ExError::NoRangeAllowed => "No range allowed".into(),
            ExError::TrailingCharacters(s) => format!("Trailing characters: {s}"),
            ExError::PatternNotFound(s) => format!("Pattern not found: {s}"),
            ExError::InvalidPattern(s) => format!("Invalid search string: {s}"),
            ExError::NoPreviousPattern => "No previous regular expression".into(),
            ExError::InvalidRegister(c) => format!("Invalid register name: '{c}'"),
        }
    }
}

/// `[abbreviation, full name]`.
const NAMES: &[[&str; 2]] = &[
    ["w", "write"],
    ["wa", "wall"],
    ["q", "quit"],
    ["qa", "qall"],
    ["wq", "wq"],
    ["x", "xit"],
    ["d", "delete"],
    ["y", "yank"],
    ["s", "substitute"],
    ["noh", "nohlsearch"],
];

fn full_name(name: &str) -> Option<&'static str> {
    NAMES
        .iter()
        .find(|[abbr, full]| full.starts_with(name) && name.len() >= abbr.len())
        .map(|[_, full]| *full)
}

fn parse_address(s: &str) -> Result<(Option<Address>, &str), ExError> {
    let mut rest = s;
    let mut base = None;
    let mut chars = rest.chars();
    match chars.next() {
        Some('.') => {
            base = Some(Base::Current);
            rest = &rest[1..];
        }
        Some('$') => {
            base = Some(Base::Last);
            rest = &rest[1..];
        }
        Some('\'') => {
            let name = chars.next().ok_or(ExError::MarkNotSet)?;
            if !matches!(name, 'a'..='z' | '<' | '>') {
                return Err(ExError::MarkNotSet);
            }
            base = Some(Base::Mark(name));
            rest = &rest[1 + name.len_utf8()..];
        }
        Some(c) if c.is_ascii_digit() => {
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            let n = rest[..end].parse().map_err(|_| ExError::InvalidRange)?;
            base = Some(Base::Number(n));
            rest = &rest[end..];
        }
        _ => {}
    }
    let mut offset = 0i64;
    let mut any_offset = false;
    while let Some(sign @ ('+' | '-')) = rest.chars().next() {
        let tail = &rest[1..];
        let end = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        let n: i64 = if end == 0 {
            1
        } else {
            tail[..end].parse().map_err(|_| ExError::InvalidRange)?
        };
        offset = offset.saturating_add(if sign == '+' { n } else { -n });
        any_offset = true;
        rest = &tail[end..];
    }
    if base.is_none() && !any_offset {
        return Ok((None, s));
    }
    Ok((
        Some(Address {
            base: base.unwrap_or(Base::Current),
            offset,
        }),
        rest,
    ))
}

fn parse_range(s: &str) -> Result<(LineRange, &str), ExError> {
    if let Some(rest) = s.strip_prefix('%') {
        return Ok((LineRange::All, rest));
    }
    let current = Address {
        base: Base::Current,
        offset: 0,
    };
    let (first, rest) = parse_address(s)?;
    if let Some(after) = rest.strip_prefix(',') {
        let (second, rest) = parse_address(after)?;
        return Ok((
            LineRange::Two(first.unwrap_or(current), second.unwrap_or(current)),
            rest,
        ));
    }
    Ok((first.map_or(LineRange::Implicit, LineRange::One), rest))
}

fn parse_count(s: &str) -> Result<Option<u32>, ExError> {
    let s = s.trim();
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let tail = s[end..].trim();
    if !tail.is_empty() {
        return Err(ExError::TrailingCharacters(tail.to_string()));
    }
    if end == 0 {
        return Ok(None);
    }
    match s[..end].parse::<u32>() {
        Ok(n) if n > 0 => Ok(Some(n)),
        _ => Err(ExError::InvalidRange),
    }
}

/// `[x] [count]` after `:d` / `:y`.
fn parse_register_count(s: &str) -> Result<(Option<char>, Option<u32>), ExError> {
    let s = s.trim_start();
    let mut register = None;
    let mut rest = s;
    if let Some(c) = s.chars().next()
        && !c.is_ascii_digit()
    {
        if !(c.is_ascii_alphabetic() || matches!(c, '"' | '_' | '+' | '*' | '-')) {
            return Err(ExError::InvalidRegister(c));
        }
        register = Some(c);
        rest = &s[c.len_utf8()..];
    }
    Ok((register, parse_count(rest)?))
}

fn parse_flags_count(s: &str) -> Result<(SubFlags, Option<u32>), ExError> {
    let mut flags = SubFlags::default();
    let mut rest = s.trim_start();
    while let Some(c) = rest.chars().next() {
        match c {
            'g' => flags.global = true,
            'i' => flags.ignore_case = Some(true),
            'I' => flags.ignore_case = Some(false),
            'n' => flags.report_only = true,
            _ => break,
        }
        rest = &rest[1..];
    }
    Ok((flags, parse_count(rest)?))
}

/// Reads up to the next unescaped `delim`; `\{delim}` becomes `delim`.
/// Returns the text and what follows the delimiter (`None` if it was missing).
fn take_delimited(s: &str, delim: char) -> (String, Option<&str>) {
    let mut out = String::new();
    let mut iter = s.char_indices();
    while let Some((i, c)) = iter.next() {
        if c == delim {
            return (out, Some(&s[i + c.len_utf8()..]));
        }
        if c == '\\' {
            match iter.next() {
                Some((_, n)) if n == delim => out.push(n),
                Some((_, n)) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    (out, None)
}

fn parse_substitute(rest: &str) -> Result<SubstituteArgs, ExError> {
    let delim = rest.chars().next();
    let Some(delim) = delim
        .filter(|c| !c.is_alphanumeric() && !c.is_whitespace() && !matches!(c, '\\' | '"' | '|'))
    else {
        let (flags, count) = parse_flags_count(rest)?;
        return Ok(SubstituteArgs::Repeat { flags, count });
    };
    let (pattern, after) = take_delimited(&rest[delim.len_utf8()..], delim);
    let (replacement, after) = match after {
        Some(a) => take_delimited(a, delim),
        None => (String::new(), None),
    };
    let (flags, count) = parse_flags_count(after.unwrap_or(""))?;
    Ok(SubstituteArgs::New {
        pattern,
        replacement,
        flags,
        count,
    })
}

pub(crate) fn parse(line: &str) -> Result<ExLine, ExError> {
    let line = line.trim_start_matches([':', ' ', '\t']).trim_end();
    let (range, rest) = parse_range(line)?;
    let rest = rest.trim_start();
    let name_end = match rest.chars().next() {
        Some(c @ ('>' | '<')) => rest.find(|x| x != c).unwrap_or(rest.len()),
        _ => rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len()),
    };
    let (name, rest) = rest.split_at(name_end);
    if name.is_empty() {
        if rest.is_empty() {
            return Ok(ExLine {
                range,
                command: Command::Goto,
            });
        }
        return Err(ExError::NotACommand(rest.to_string()));
    }
    let (bang, rest) = match rest.strip_prefix('!') {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let no_bang = |bang: bool| {
        if bang {
            Err(ExError::NoBangAllowed)
        } else {
            Ok(())
        }
    };
    let no_args = |rest: &str| {
        if rest.trim().is_empty() {
            Ok(())
        } else {
            Err(ExError::TrailingCharacters(rest.trim().to_string()))
        }
    };
    let command = if name.starts_with(['>', '<']) {
        no_bang(bang)?;
        Command::Shift {
            right: name.starts_with('>'),
            depth: name.len() as u32,
            count: parse_count(rest)?,
        }
    } else {
        let full = full_name(name).ok_or_else(|| ExError::NotACommand(name.to_string()))?;
        let takes_range = matches!(full, "delete" | "yank" | "substitute");
        if !takes_range && !matches!(range, LineRange::Implicit) {
            return Err(ExError::NoRangeAllowed);
        }
        match full {
            "write" => {
                no_args(rest)?;
                Command::Write { bang }
            }
            "quit" => {
                no_args(rest)?;
                Command::Quit { bang }
            }
            "wq" => {
                no_args(rest)?;
                Command::WriteQuit { bang }
            }
            "xit" => {
                no_args(rest)?;
                Command::Exit { bang }
            }
            "wall" => {
                no_bang(bang)?;
                no_args(rest)?;
                Command::WriteAll
            }
            "qall" => {
                no_bang(bang)?;
                no_args(rest)?;
                Command::QuitAll
            }
            "nohlsearch" => {
                no_bang(bang)?;
                no_args(rest)?;
                Command::NoHighlight
            }
            "delete" | "yank" => {
                no_bang(bang)?;
                let (register, count) = parse_register_count(rest)?;
                if full == "delete" {
                    Command::Delete { register, count }
                } else {
                    Command::Yank { register, count }
                }
            }
            _ => {
                no_bang(bang)?;
                Command::Substitute(parse_substitute(rest)?)
            }
        }
    };
    Ok(ExLine { range, command })
}

fn resolve_address(
    a: &Address,
    cursor_row: u32,
    last_row: u32,
    marks: Option<&FileMarks>,
) -> Result<i64, ExError> {
    let base = match a.base {
        Base::Number(n) => i64::from(n) - 1,
        Base::Current => i64::from(cursor_row),
        Base::Last => i64::from(last_row),
        Base::Mark(c) => i64::from(marks.and_then(|m| m.get(c)).ok_or(ExError::MarkNotSet)?.row),
    };
    Ok(base.saturating_add(a.offset))
}

/// 0-based inclusive rows. With `clamp`, out-of-range rows clamp (a bare
/// go-to-line) instead of failing with E16.
pub(crate) fn resolve(
    range: &LineRange,
    cursor_row: u32,
    last_row: u32,
    marks: Option<&FileMarks>,
    clamp: bool,
) -> Result<RangeInclusive<u32>, ExError> {
    let row = |a: &Address| -> Result<u32, ExError> {
        let r = resolve_address(a, cursor_row, last_row, marks)?;
        if clamp {
            Ok(r.clamp(0, i64::from(last_row)) as u32)
        } else if (0..=i64::from(last_row)).contains(&r) {
            Ok(r as u32)
        } else {
            Err(ExError::InvalidRange)
        }
    };
    let (a, b) = match range {
        LineRange::Implicit => (cursor_row.min(last_row), cursor_row.min(last_row)),
        LineRange::All => (0, last_row),
        LineRange::One(a) => {
            let r = row(a)?;
            (r, r)
        }
        LineRange::Two(a, b) => (row(a)?, row(b)?),
    };
    Ok(a.min(b)..=a.max(b))
}

/// The Quick Open row text after `:{line} — `, or the error row text.
pub fn describe_command(line: &str) -> Result<String, String> {
    let parsed = parse(line).map_err(|e| e.short())?;
    let mark = |c| Address {
        base: Base::Mark(c),
        offset: 0,
    };
    let scope = match parsed.range {
        LineRange::Implicit => "current line",
        LineRange::One(Address {
            base: Base::Current,
            offset: 0,
        }) => "current line",
        LineRange::All => "all lines",
        LineRange::Two(a, b) if a == mark('<') && b == mark('>') => "selected lines",
        _ => "lines in range",
    };
    let text = match parsed.command {
        Command::Goto => match parsed.range {
            LineRange::One(Address {
                base: Base::Number(n),
                offset: 0,
            }) => format!("Go to line {n}"),
            LineRange::One(Address {
                base: Base::Last,
                offset: 0,
            }) => "Go to last line".into(),
            _ => "Go to line".into(),
        },
        Command::Write { bang: false } => "Save".into(),
        Command::Write { bang: true } => "Save, overwriting the file on disk".into(),
        Command::WriteAll => "Save all".into(),
        Command::Quit { bang: false } => "Close editor".into(),
        Command::Quit { bang: true } => "Revert and close editor".into(),
        Command::QuitAll => "Close all editors".into(),
        Command::WriteQuit { .. } => "Save and close editor".into(),
        Command::Exit { .. } => "Save if modified and close editor".into(),
        Command::Delete { .. } => format!("Delete {scope}"),
        Command::Yank { .. } => format!("Yank {scope}"),
        Command::Shift { right: true, .. } => format!("Indent {scope}"),
        Command::Shift { right: false, .. } => format!("Outdent {scope}"),
        Command::Substitute(_) => match scope {
            "selected lines" => "Replace in selection".into(),
            "lines in range" => "Replace in range".into(),
            s => format!("Replace in {s}"),
        },
        Command::NoHighlight => "Clear search highlight".into(),
    };
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lucerna_text::Point;

    fn addr(base: Base, offset: i64) -> Address {
        Address { base, offset }
    }

    fn ok(line: &str) -> ExLine {
        parse(line).unwrap()
    }

    #[test]
    fn substitute_with_range_and_flags() {
        let l = ok("%s/foo/bar/g");
        assert_eq!(l.range, LineRange::All);
        let Command::Substitute(SubstituteArgs::New {
            pattern,
            replacement,
            flags,
            count,
        }) = l.command
        else {
            panic!()
        };
        assert_eq!((pattern.as_str(), replacement.as_str()), ("foo", "bar"));
        assert!(flags.global);
        assert_eq!(count, None);
    }

    #[test]
    fn ranges() {
        assert_eq!(
            ok("'<,'>d").range,
            LineRange::Two(addr(Base::Mark('<'), 0), addr(Base::Mark('>'), 0))
        );
        let l = ok(".+3yan2");
        assert_eq!(l.range, LineRange::One(addr(Base::Current, 3)));
        assert_eq!(
            l.command,
            Command::Yank {
                register: None,
                count: Some(2)
            }
        );
        assert_eq!(
            ok(".++1s/a/b/").range,
            LineRange::One(addr(Base::Current, 2))
        );
        assert_eq!(ok("+2s/a/b/").range, LineRange::One(addr(Base::Current, 2)));
        assert_eq!(
            ok("5,$d").range,
            LineRange::Two(addr(Base::Number(5), 0), addr(Base::Last, 0))
        );
        assert_eq!(ok("  ").range, LineRange::Implicit);
        assert_eq!(ok("").command, Command::Goto);
    }

    // Adapted from VSCodeVim test/cmd_line/substitute.test.ts: "Substitute with escaped delimiter"
    #[test]
    fn escaped_delimiter() {
        let Command::Substitute(SubstituteArgs::New {
            pattern,
            replacement,
            ..
        }) = ok("s/\\/\\/f/z/g").command
        else {
            panic!()
        };
        assert_eq!((pattern.as_str(), replacement.as_str()), ("//f", "z"));
    }

    #[test]
    fn repeat_and_count() {
        let Command::Substitute(SubstituteArgs::Repeat { flags, count }) = ok("s g").command else {
            panic!()
        };
        assert!(flags.global);
        assert_eq!(count, None);
        let Command::Substitute(SubstituteArgs::New { count, .. }) = ok("s/a/b/ 3").command else {
            panic!()
        };
        assert_eq!(count, Some(3));
        let Command::Substitute(SubstituteArgs::Repeat { count, .. }) = ok("s 3").command else {
            panic!()
        };
        assert_eq!(count, Some(3));
        let Command::Substitute(SubstituteArgs::New { replacement, .. }) = ok("s/a").command else {
            panic!()
        };
        assert_eq!(replacement, "");
    }

    #[test]
    fn commands_and_errors() {
        assert_eq!(ok("wq!").command, Command::WriteQuit { bang: true });
        assert_eq!(ok("write").command, Command::Write { bang: false });
        assert_eq!(ok(":noh").command, Command::NoHighlight);
        assert_eq!(
            ok(">>").command,
            Command::Shift {
                right: true,
                depth: 2,
                count: None
            }
        );
        assert_eq!(
            ok("<3").command,
            Command::Shift {
                right: false,
                depth: 1,
                count: Some(3)
            }
        );
        assert_eq!(
            ok("y3").command,
            Command::Yank {
                register: None,
                count: Some(3)
            }
        );
        assert_eq!(parse("wa!").unwrap_err(), ExError::NoBangAllowed);
        assert_eq!(parse("no").unwrap_err(), ExError::NotACommand("no".into()));
        assert_eq!(
            parse("foo").unwrap_err(),
            ExError::NotACommand("foo".into())
        );
        assert_eq!(
            parse("d x 2 z").unwrap_err(),
            ExError::TrailingCharacters("z".into())
        );
        assert_eq!(
            parse("s/a/b/q").unwrap_err(),
            ExError::TrailingCharacters("q".into())
        );
        assert_eq!(parse("d !").unwrap_err(), ExError::InvalidRegister('!'));
    }

    #[test]
    fn a_range_on_a_command_without_one_is_an_error() {
        for line in ["'<,'>w", "5q", "%wa", "1,2qa", ".wq", "3x", "1noh", "2q!"] {
            assert_eq!(parse(line).unwrap_err(), ExError::NoRangeAllowed, "{line}");
        }
        assert_eq!(ExError::NoRangeAllowed.message(), "E481: No range allowed");
        assert_eq!(ExError::NoRangeAllowed.short(), "No range allowed");
    }

    #[test]
    fn resolve_ranges() {
        let r =
            |s: &str, marks: Option<&FileMarks>, clamp| resolve(&ok(s).range, 2, 4, marks, clamp);
        assert_eq!(r("3,1d", None, false).unwrap(), 0..=2);
        assert_eq!(r("'ad", None, false), Err(ExError::MarkNotSet));
        assert_eq!(r("20", None, false), Err(ExError::InvalidRange));
        assert_eq!(r("20", None, true).unwrap(), 4..=4);
        assert_eq!(r("%d", None, false).unwrap(), 0..=4);
        assert_eq!(r("d", None, false).unwrap(), 2..=2);
        assert_eq!(r(".,$d", None, false).unwrap(), 2..=4);
        let mut marks = FileMarks::default();
        marks.letters[0] = Some(Point { row: 1, column: 0 });
        assert_eq!(r("'a,.d", Some(&marks), false).unwrap(), 1..=2);
    }

    #[test]
    fn messages() {
        assert_eq!(
            ExError::NotACommand("foo".into()).message(),
            "E492: Not an editor command: foo"
        );
        assert_eq!(
            ExError::PatternNotFound("foo".into()).message(),
            "E486: Pattern not found: foo"
        );
        assert_eq!(ExError::InvalidRange.short(), "Invalid range");
    }

    #[test]
    fn describe() {
        let d = |s: &str| describe_command(s);
        assert_eq!(d("12"), Ok("Go to line 12".into()));
        assert_eq!(d("$"), Ok("Go to last line".into()));
        assert_eq!(d("+2"), Ok("Go to line".into()));
        assert_eq!(d("w"), Ok("Save".into()));
        assert_eq!(d("w!"), Ok("Save, overwriting the file on disk".into()));
        assert_eq!(d("wa"), Ok("Save all".into()));
        assert_eq!(d("q"), Ok("Close editor".into()));
        assert_eq!(d("q!"), Ok("Revert and close editor".into()));
        assert_eq!(d("qa"), Ok("Close all editors".into()));
        assert_eq!(d("wq"), Ok("Save and close editor".into()));
        assert_eq!(d("x"), Ok("Save if modified and close editor".into()));
        assert_eq!(d("d"), Ok("Delete current line".into()));
        assert_eq!(d("%y"), Ok("Yank all lines".into()));
        assert_eq!(d("'<,'>>"), Ok("Indent selected lines".into()));
        assert_eq!(d("1,3<"), Ok("Outdent lines in range".into()));
        assert_eq!(d("s/a/b/"), Ok("Replace in current line".into()));
        assert_eq!(d("%s/a/b/"), Ok("Replace in all lines".into()));
        assert_eq!(d("'<,'>s/a/b/"), Ok("Replace in selection".into()));
        assert_eq!(d("1,2s/a/b/"), Ok("Replace in range".into()));
        assert_eq!(d("noh"), Ok("Clear search highlight".into()));
        assert_eq!(d("foo"), Err("Not an editor command: foo".into()));
    }
}
