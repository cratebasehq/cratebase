//! Tokenizer for filter expressions. Filter strings are short (one WHERE
//! clause), so the whole input is tokenized up-front.

use crate::ast::Modifier;
use crate::error::FilterError;

#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// An identifier path (`title`, `author.name`, `@request.auth.id`,
    /// `comments_via_post.title`) with its optional `:modifier`.
    Ident {
        name: String,
        modifier: Option<Modifier>,
    },
    Str(String),
    Num(f64),
    True,
    False,
    Null,
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
    /// `?=` — "any of" variant of `Eq`.
    QEq,
    QNotEq,
    QGt,
    QGte,
    QLt,
    QLte,
    QLike,
    QNotLike,
    And,
    Or,
    LParen,
    RParen,
    Comma,
    Eof,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<u8> {
        self.src.get(self.pos + offset).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    /// The character starting at the current byte position, for error
    /// reporting (multi-byte UTF-8 sequences are decoded properly).
    fn current_char(&self) -> char {
        std::str::from_utf8(&self.src[self.pos..])
            .ok()
            .and_then(|s| s.chars().next())
            .or_else(|| {
                String::from_utf8_lossy(&self.src[self.pos..])
                    .chars()
                    .next()
            })
            .unwrap_or('?')
    }

    fn lex_error(&self) -> FilterError {
        FilterError::Lex(self.pos, self.current_char())
    }

    fn lex_string(&mut self, quote: u8) -> Result<Token, FilterError> {
        let mut bytes = Vec::new();
        loop {
            match self.bump() {
                Some(b'\\') => match self.bump() {
                    // Only the quote characters and the backslash itself
                    // are escapable, like PocketBase; any other escape is
                    // kept verbatim.
                    Some(c @ (b'"' | b'\'' | b'\\')) => bytes.push(c),
                    Some(c) => {
                        bytes.push(b'\\');
                        bytes.push(c);
                    }
                    None => return Err(FilterError::Parse("unterminated string literal".into())),
                },
                Some(q) if q == quote => break,
                Some(other) => bytes.push(other),
                None => return Err(FilterError::Parse("unterminated string literal".into())),
            }
        }
        Ok(Token::Str(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn lex_number(&mut self) -> Result<Token, FilterError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.bump();
        }
        let mut seen_dot = false;
        while let Some(d) = self.peek() {
            if d.is_ascii_digit() {
                self.pos += 1;
            } else if d == b'.' && !seen_dot && self.peek_at(1).is_some_and(|n| n.is_ascii_digit())
            {
                seen_dot = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap_or("");
        let n: f64 = text.parse().map_err(|_| self.lex_error())?;
        Ok(Token::Num(n))
    }

    fn lex_ident(&mut self) -> Result<Token, FilterError> {
        let start = self.pos;
        let is_macro = self.peek() == Some(b'@');
        if is_macro {
            self.bump();
        }
        if !self.peek().is_some_and(is_ident_start) {
            return Err(self.lex_error());
        }
        while let Some(c) = self.peek() {
            // A dot continues the path only when followed by a segment.
            if is_ident_char(c) || (c == b'.' && self.peek_at(1).is_some_and(is_ident_char)) {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .unwrap_or("")
            .to_string();
        // Optional `:modifier` suffix.
        let mut modifier = None;
        if self.peek() == Some(b':') {
            let mstart = self.pos + 1;
            let mut end = mstart;
            while self.src.get(end).is_some_and(|c| c.is_ascii_alphabetic()) {
                end += 1;
            }
            let name = std::str::from_utf8(&self.src[mstart..end]).unwrap_or("");
            match Modifier::parse(name) {
                Some(m) => {
                    modifier = Some(m);
                    self.pos = end;
                }
                None => return Err(FilterError::InvalidModifier(format!("{text}:{name}"))),
            }
        }
        if !is_macro && modifier.is_none() {
            match text.as_str() {
                "true" => return Ok(Token::True),
                "false" => return Ok(Token::False),
                "null" => return Ok(Token::Null),
                _ => {}
            }
        }
        Ok(Token::Ident {
            name: text,
            modifier,
        })
    }

    /// Tokenize the whole input up-front.
    pub fn tokenize(mut self) -> Result<Vec<Token>, FilterError> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            let Some(c) = self.peek() else {
                out.push(Token::Eof);
                break;
            };
            let tok = match c {
                b'(' => {
                    self.bump();
                    Token::LParen
                }
                b')' => {
                    self.bump();
                    Token::RParen
                }
                b',' => {
                    self.bump();
                    Token::Comma
                }
                b'&' if self.peek_at(1) == Some(b'&') => {
                    self.pos += 2;
                    Token::And
                }
                b'|' if self.peek_at(1) == Some(b'|') => {
                    self.pos += 2;
                    Token::Or
                }
                b'!' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    Token::NotEq
                }
                b'!' if self.peek_at(1) == Some(b'~') => {
                    self.pos += 2;
                    Token::NotLike
                }
                b'=' => {
                    self.bump();
                    Token::Eq
                }
                b'~' => {
                    self.bump();
                    Token::Like
                }
                b'>' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    Token::Gte
                }
                b'>' => {
                    self.bump();
                    Token::Gt
                }
                b'<' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    Token::Lte
                }
                b'<' => {
                    self.bump();
                    Token::Lt
                }
                b'?' => {
                    let tok = match (self.peek_at(1), self.peek_at(2)) {
                        (Some(b'='), _) => (Token::QEq, 2),
                        (Some(b'!'), Some(b'=')) => (Token::QNotEq, 3),
                        (Some(b'!'), Some(b'~')) => (Token::QNotLike, 3),
                        (Some(b'>'), Some(b'=')) => (Token::QGte, 3),
                        (Some(b'>'), _) => (Token::QGt, 2),
                        (Some(b'<'), Some(b'=')) => (Token::QLte, 3),
                        (Some(b'<'), _) => (Token::QLt, 2),
                        (Some(b'~'), _) => (Token::QLike, 2),
                        _ => return Err(self.lex_error()),
                    };
                    self.pos += tok.1;
                    tok.0
                }
                b'"' | b'\'' => {
                    self.bump();
                    self.lex_string(c)?
                }
                b'0'..=b'9' => self.lex_number()?,
                b'-' if self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) => self.lex_number()?,
                b'@' | b'_' | b'a'..=b'z' | b'A'..=b'Z' => self.lex_ident()?,
                _ => return Err(self.lex_error()),
            };
            out.push(tok);
        }
        Ok(out)
    }
}
