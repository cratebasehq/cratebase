#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Ident(String),
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
    And,
    Or,
    LParen,
    RParen,
    Eof,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("unexpected character '{0}' at position {1}")]
pub struct LexError(pub char, pub usize);

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

    /// Tokenize the whole input up-front. Filter expressions are short
    /// (a single WHERE clause) so this is simpler than a streaming lexer.
    pub fn tokenize(mut self) -> Result<Vec<Token>, LexError> {
        let mut out = Vec::new();
        loop {
            self.skip_ws();
            let Some(c) = self.peek() else {
                out.push(Token::Eof);
                break;
            };
            match c {
                b'(' => {
                    self.bump();
                    out.push(Token::LParen);
                }
                b')' => {
                    self.bump();
                    out.push(Token::RParen);
                }
                b'&' if self.peek_at(1) == Some(b'&') => {
                    self.pos += 2;
                    out.push(Token::And);
                }
                b'|' if self.peek_at(1) == Some(b'|') => {
                    self.pos += 2;
                    out.push(Token::Or);
                }
                b'!' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    out.push(Token::NotEq);
                }
                b'!' if self.peek_at(1) == Some(b'~') => {
                    self.pos += 2;
                    out.push(Token::NotLike);
                }
                b'=' => {
                    self.bump();
                    out.push(Token::Eq);
                }
                b'~' => {
                    self.bump();
                    out.push(Token::Like);
                }
                b'>' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    out.push(Token::Gte);
                }
                b'>' => {
                    self.bump();
                    out.push(Token::Gt);
                }
                b'<' if self.peek_at(1) == Some(b'=') => {
                    self.pos += 2;
                    out.push(Token::Lte);
                }
                b'<' => {
                    self.bump();
                    out.push(Token::Lt);
                }
                b'"' | b'\'' => {
                    let quote = c;
                    self.bump();
                    let mut s = String::new();
                    loop {
                        match self.bump() {
                            Some(b'\\') => {
                                if let Some(next) = self.bump() {
                                    s.push(next as char);
                                }
                            }
                            Some(q) if q == quote => break,
                            Some(other) => s.push(other as char),
                            None => break,
                        }
                    }
                    out.push(Token::Str(s));
                }
                b'0'..=b'9' => {
                    let start = self.pos;
                    self.bump();
                    while let Some(d) = self.peek() {
                        if d.is_ascii_digit() || d == b'.' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    let n: f64 = text
                        .parse()
                        .map_err(|_| LexError(text.chars().next().unwrap_or('0'), start))?;
                    out.push(Token::Num(n));
                }
                b'-' if self.peek_at(1).is_some_and(|d| d.is_ascii_digit()) => {
                    let start = self.pos;
                    self.bump();
                    while let Some(d) = self.peek() {
                        if d.is_ascii_digit() || d == b'.' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    let n: f64 = text
                        .parse()
                        .map_err(|_| LexError('-', start))?;
                    out.push(Token::Num(n));
                }
                b'@' | b'_' | b'a'..=b'z' | b'A'..=b'Z' => {
                    let start = self.pos;
                    self.pos += 1;
                    while let Some(d) = self.peek() {
                        if d.is_ascii_alphanumeric() || d == b'_' || d == b'.' || d == b'@' {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let text = std::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    match text {
                        "true" => out.push(Token::True),
                        "false" => out.push(Token::False),
                        "null" => out.push(Token::Null),
                        _ => out.push(Token::Ident(text.to_string())),
                    }
                }
                other => return Err(LexError(other as char, self.pos)),
            }
        }
        Ok(out)
    }
}
