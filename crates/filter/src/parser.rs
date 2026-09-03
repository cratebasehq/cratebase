use crate::ast::{CompareOp, Expr, Literal, Operand};
use crate::error::FilterError;
use crate::lexer::{Lexer, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn parse(src: &str) -> Result<Expr, FilterError> {
        if src.trim().is_empty() {
            return Err(FilterError::Empty);
        }
        let tokens = Lexer::new(src).tokenize()?;
        let mut parser = Parser { tokens, pos: 0 };
        let expr = parser.parse_or()?;
        parser.expect_eof()?;
        Ok(expr)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn bump(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect_eof(&self) -> Result<(), FilterError> {
        if matches!(self.peek(), Token::Eof) {
            Ok(())
        } else {
            Err(FilterError::Parse(format!(
                "unexpected trailing token: {:?}",
                self.peek()
            )))
        }
    }

    fn parse_or(&mut self) -> Result<Expr, FilterError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.bump();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, FilterError> {
        let mut left = self.parse_unary()?;
        while matches!(self.peek(), Token::And) {
            self.bump();
            let right = self.parse_unary()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, FilterError> {
        if matches!(self.peek(), Token::LParen) {
            self.bump();
            let inner = self.parse_or()?;
            match self.bump() {
                Token::RParen => Ok(inner),
                other => Err(FilterError::Parse(format!(
                    "expected ')' but found {:?}",
                    other
                ))),
            }
        } else {
            self.parse_comparison()
        }
    }

    fn parse_comparison(&mut self) -> Result<Expr, FilterError> {
        let left = self.parse_operand()?;
        let (op, any_of) = match self.bump() {
            Token::Eq => (CompareOp::Eq, false),
            Token::NotEq => (CompareOp::NotEq, false),
            Token::Gt => (CompareOp::Gt, false),
            Token::Gte => (CompareOp::Gte, false),
            Token::Lt => (CompareOp::Lt, false),
            Token::Lte => (CompareOp::Lte, false),
            Token::Like => (CompareOp::Like, false),
            Token::NotLike => (CompareOp::NotLike, false),
            Token::QEq => (CompareOp::Eq, true),
            Token::QNotEq => (CompareOp::NotEq, true),
            Token::QGt => (CompareOp::Gt, true),
            Token::QGte => (CompareOp::Gte, true),
            Token::QLt => (CompareOp::Lt, true),
            Token::QLte => (CompareOp::Lte, true),
            Token::QLike => (CompareOp::Like, true),
            Token::QNotLike => (CompareOp::NotLike, true),
            other => {
                return Err(FilterError::Parse(format!(
                    "expected comparison operator but found {:?}",
                    other
                )))
            }
        };
        let right = self.parse_operand()?;
        Ok(Expr::Compare {
            left,
            op,
            any_of,
            right,
        })
    }

    fn parse_operand(&mut self) -> Result<Operand, FilterError> {
        match self.bump() {
            Token::Ident(name) => Ok(Operand::Ident(name)),
            Token::Str(s) => Ok(Operand::Literal(Literal::Str(s))),
            Token::Num(n) => Ok(Operand::Literal(Literal::Num(n))),
            Token::True => Ok(Operand::Literal(Literal::Bool(true))),
            Token::False => Ok(Operand::Literal(Literal::Bool(false))),
            Token::Null => Ok(Operand::Literal(Literal::Null)),
            other => Err(FilterError::Parse(format!(
                "expected value or field but found {:?}",
                other
            ))),
        }
    }
}
