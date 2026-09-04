/// Everything that can go wrong while lexing, parsing, resolving or
/// compiling a filter expression.
///
/// The variants are deliberately coarse: the HTTP layer maps every one of
/// them to PocketBase's `400 Invalid filter` response, and the realtime
/// evaluator uses [`FilterError::Unsupported`] as the signal to fall back
/// to SQL evaluation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    /// The expression was empty or whitespace only.
    #[error("filter expression is empty")]
    Empty,
    /// An unexpected character at a byte position.
    #[error("invalid filter syntax: unexpected character '{1}' at position {0}")]
    Lex(usize, char),
    /// A grammar error (unbalanced parentheses, missing operand, ...).
    #[error("invalid filter syntax: {0}")]
    Parse(String),
    /// A field path that does not resolve against the collection schema.
    #[error("unknown field '{0}' in filter")]
    UnknownField(String),
    /// An `@identifier` that is neither a date macro nor a known
    /// `@request.*` / `@collection.*` path.
    #[error("unknown macro '{0}' in filter")]
    UnknownMacro(String),
    /// A `:modifier` used on an operand that does not support it.
    #[error("invalid modifier: {0}")]
    InvalidModifier(String),
    /// The expression is valid but cannot be handled by this backend
    /// (e.g. the in-process evaluator meeting a relation path).
    #[error("unsupported filter expression: {0}")]
    Unsupported(String),
}
