//! The parsed shape of a filter expression.
//!
//! ```text
//! expr     := or
//! or       := and ( "||" and )*
//! and      := unary ( "&&" unary )*
//! unary    := "(" expr ")" | compare
//! compare  := operand OP operand
//! OP       := = != > >= < <= ~ !~ ?= ?!= ?> ?>= ?< ?<= ?~ ?!~
//! operand  := literal | ident modifier? | call
//! literal  := string | number | true | false | null
//! ident    := [@]?[A-Za-z_][A-Za-z0-9_]* ( "." [A-Za-z0-9_]+ )*
//! modifier := ":isset" | ":length" | ":each" | ":lower"
//! call     := name "(" operand ( "," operand )* ")"
//! ```

/// A literal value in the expression source.
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

/// The comparison operators. The "any of" (`?`-prefixed) variants share
/// these and are flagged separately on [`Expr::Compare::any_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    /// `~`: LIKE / contains (auto-wrapped in `%` when the pattern has none).
    Like,
    /// `!~`
    NotLike,
}

impl CompareOp {
    /// The operator with its operands swapped (`a < b` ⇔ `b > a`).
    pub fn mirrored(self) -> CompareOp {
        match self {
            CompareOp::Gt => CompareOp::Lt,
            CompareOp::Gte => CompareOp::Lte,
            CompareOp::Lt => CompareOp::Gt,
            CompareOp::Lte => CompareOp::Gte,
            other => other,
        }
    }

    pub fn is_like(self) -> bool {
        matches!(self, CompareOp::Like | CompareOp::NotLike)
    }
}

/// The optional `:modifier` suffix on an identifier, as in PocketBase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// `@request.body.x:isset` — whether the key was sent at all.
    IsSet,
    /// `x:length` — number of elements of a multi-valued field.
    Length,
    /// `x:each` — compare every element of a multi-valued field.
    Each,
    /// `x:lower` — lowercase before comparing.
    Lower,
}

impl Modifier {
    pub fn parse(name: &str) -> Option<Modifier> {
        match name {
            "isset" => Some(Modifier::IsSet),
            "length" => Some(Modifier::Length),
            "each" => Some(Modifier::Each),
            "lower" => Some(Modifier::Lower),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Modifier::IsSet => "isset",
            Modifier::Length => "length",
            Modifier::Each => "each",
            Modifier::Lower => "lower",
        }
    }
}

/// Known functions and their arity. Extend this table to add new callable
/// operands (`vectorDistance`, `search`, ...).
pub const FUNCTIONS: &[(&str, usize)] = &[("geoDistance", 4)];

/// One side of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Literal(Literal),
    /// A field path, `@macro`, `@request.*` or `@collection.*` reference,
    /// with an optional modifier.
    Ident {
        path: String,
        modifier: Option<Modifier>,
    },
    /// A function call such as `geoDistance(a, b, c, d)`.
    Call {
        name: String,
        args: Vec<Operand>,
    },
}

impl Operand {
    /// A plain identifier without modifier.
    pub fn ident(path: impl Into<String>) -> Operand {
        Operand::Ident {
            path: path.into(),
            modifier: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Compare {
        left: Operand,
        op: CompareOp,
        /// Set for the `?=`, `?!=`, `?>`, ... "any of" operators: the
        /// comparison is satisfied if *any* element of a multi-valued
        /// operand matches, instead of requiring every element to match.
        any_of: bool,
        right: Operand,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
