#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    NotEq,
    Gt,
    Gte,
    Lt,
    Lte,
    Like,
    NotLike,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Literal(Literal),
    Ident(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Compare {
        left: Operand,
        op: CompareOp,
        right: Operand,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
}
