use crate::lexer::LexError;

#[derive(Debug, thiserror::Error)]
pub enum FilterError {
    #[error("invalid filter syntax: {0}")]
    Lex(#[from] LexError),
    #[error("invalid filter syntax: {0}")]
    Parse(String),
    #[error("unknown field '{0}' in filter")]
    UnknownField(String),
    #[error("filter expression is empty")]
    Empty,
}
