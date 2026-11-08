//! VS Code–style `when` clauses: `!`, `&&`, `||`, `==`, `!=` and parentheses.

use std::collections::HashMap;

use crate::SettingsError;

#[derive(Debug, Clone, PartialEq)]
pub enum ContextValue {
    Bool(bool),
    Str(String),
}

impl ContextValue {
    fn truthy(&self) -> bool {
        match self {
            ContextValue::Bool(b) => *b,
            ContextValue::Str(s) => !s.is_empty(),
        }
    }

    fn as_text(&self) -> String {
        match self {
            ContextValue::Bool(b) => b.to_string(),
            ContextValue::Str(s) => s.clone(),
        }
    }
}

/// Key/value context a `when` clause is evaluated against.
#[derive(Debug, Clone, Default)]
pub struct Context(HashMap<String, ContextValue>);

impl Context {
    pub fn set(&mut self, key: impl Into<String>, value: ContextValue) {
        self.0.insert(key.into(), value);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WhenExpr {
    Key(String),
    Not(Box<WhenExpr>),
    And(Box<WhenExpr>, Box<WhenExpr>),
    Or(Box<WhenExpr>, Box<WhenExpr>),
    Eq(String, String),
    Ne(String, String),
}

impl WhenExpr {
    /// An unknown key is falsy and compares as `""`.
    pub fn eval(&self, ctx: &Context) -> bool {
        let text = |key: &str| {
            ctx.0
                .get(key)
                .map(ContextValue::as_text)
                .unwrap_or_default()
        };
        match self {
            WhenExpr::Key(k) => ctx.0.get(k).is_some_and(ContextValue::truthy),
            WhenExpr::Not(e) => !e.eval(ctx),
            WhenExpr::And(a, b) => a.eval(ctx) && b.eval(ctx),
            WhenExpr::Or(a, b) => a.eval(ctx) || b.eval(ctx),
            WhenExpr::Eq(k, v) => text(k) == *v,
            WhenExpr::Ne(k, v) => text(k) != *v,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    Not,
    And,
    Or,
    EqEq,
    NotEq,
    Word(String),
    Quoted(String),
}

fn err(message: impl Into<String>) -> SettingsError {
    SettingsError {
        message: message.into(),
        line: None,
    }
}

fn tokenize(src: &str) -> Result<Vec<Token>, SettingsError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match (c, next) {
            (c, _) if c.is_whitespace() => i += 1,
            ('(', _) => {
                tokens.push(Token::LParen);
                i += 1;
            }
            (')', _) => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ('&', Some('&')) => {
                tokens.push(Token::And);
                i += 2;
            }
            ('|', Some('|')) => {
                tokens.push(Token::Or);
                i += 2;
            }
            ('=', Some('=')) => {
                tokens.push(Token::EqEq);
                i += 2;
            }
            ('!', Some('=')) => {
                tokens.push(Token::NotEq);
                i += 2;
            }
            ('!', _) => {
                tokens.push(Token::Not);
                i += 1;
            }
            ('\'', _) => {
                let rest = &chars[i + 1..];
                let len = rest
                    .iter()
                    .position(|&c| c == '\'')
                    .ok_or_else(|| err("unterminated string in when clause"))?;
                tokens.push(Token::Quoted(rest[..len].iter().collect()));
                i += len + 2;
            }
            ('&' | '|' | '=', _) => return Err(err(format!("unexpected '{c}' in when clause"))),
            _ => {
                let len = chars[i..]
                    .iter()
                    .position(|&c| c.is_whitespace() || "()!&|='".contains(c))
                    .unwrap_or(chars.len() - i);
                tokens.push(Token::Word(chars[i..i + len].iter().collect()));
                i += len;
            }
        }
    }
    Ok(tokens)
}

pub fn parse_when(src: &str) -> Result<WhenExpr, SettingsError> {
    let mut parser = Parser {
        tokens: tokenize(src)?,
        pos: 0,
    };
    let expr = parser.or()?;
    match parser.peek() {
        None => Ok(expr),
        Some(t) => Err(err(format!("unexpected {t:?} in when clause"))),
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn eat(&mut self, token: &Token) -> bool {
        let hit = self.peek() == Some(token);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn or(&mut self) -> Result<WhenExpr, SettingsError> {
        let mut left = self.and()?;
        while self.eat(&Token::Or) {
            left = WhenExpr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<WhenExpr, SettingsError> {
        let mut left = self.unary()?;
        while self.eat(&Token::And) {
            left = WhenExpr::And(Box::new(left), Box::new(self.unary()?));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<WhenExpr, SettingsError> {
        if self.eat(&Token::Not) {
            return Ok(WhenExpr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<WhenExpr, SettingsError> {
        if self.eat(&Token::LParen) {
            let inner = self.or()?;
            if !self.eat(&Token::RParen) {
                return Err(err("missing ')' in when clause"));
            }
            return Ok(inner);
        }
        let key = match self.peek().cloned() {
            Some(Token::Word(w)) => {
                self.pos += 1;
                w
            }
            Some(t) => return Err(err(format!("unexpected {t:?} in when clause"))),
            None => return Err(err("unexpected end of when clause")),
        };
        let equal = if self.eat(&Token::EqEq) {
            true
        } else if self.eat(&Token::NotEq) {
            false
        } else {
            return Ok(WhenExpr::Key(key));
        };
        let value = match self.peek().cloned() {
            Some(Token::Word(v) | Token::Quoted(v)) => {
                self.pos += 1;
                v
            }
            _ => return Err(err("expected a value after comparison in when clause")),
        };
        Ok(if equal {
            WhenExpr::Eq(key, value)
        } else {
            WhenExpr::Ne(key, value)
        })
    }
}
