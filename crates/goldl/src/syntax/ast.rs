//! Abstract syntax tree. Every node carries a byte span.

use super::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct File {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug)]
pub struct Attr {
    pub name: Ident,
    pub args: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum Item {
    Module(Module),
    Fn(FnDef),
    Const(ConstDef),
    Enum(EnumDef),
    Test(TestDef),
}

impl Item {
    pub fn name(&self) -> Option<&Ident> {
        match self {
            Item::Module(m) => Some(&m.name),
            Item::Fn(f) => Some(&f.name),
            Item::Const(c) => Some(&c.name),
            Item::Enum(e) => Some(&e.name),
            Item::Test(_) => None,
        }
    }
    pub fn span(&self) -> Span {
        match self {
            Item::Module(m) => m.span,
            Item::Fn(f) => f.span,
            Item::Const(c) => c.span,
            Item::Enum(e) => e.span,
            Item::Test(t) => t.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Generic {
    pub name: Ident,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub attrs: Vec<Attr>,
    pub doc: Option<String>,
    pub name: Ident,
    pub generics: Vec<Generic>,
    pub inputs: Vec<Param>,
    pub outputs: Vec<Param>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct FnDef {
    pub doc: Option<String>,
    pub name: Ident,
    pub generics: Vec<Generic>,
    pub params: Vec<Param>,
    pub ret: Option<Type>,
    pub body: Block,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ConstDef {
    pub doc: Option<String>,
    pub name: Ident,
    pub ty: Option<Type>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct EnumDef {
    pub doc: Option<String>,
    pub name: Ident,
    pub repr: Option<Type>,
    pub variants: Vec<(Ident, Option<Expr>)>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct TestDef {
    pub name: String,
    pub module: Ident,
    pub body: Vec<TestStmt>,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum TestStmt {
    /// `name = expr` sets an input.
    Poke { port: Ident, value: Expr, span: Span },
    /// `step` or `step(n)`.
    Step { count: Option<Expr>, span: Span },
    /// `assert expr` (expression over outputs and inputs of the module).
    Assert { cond: Expr, span: Span },
}

#[derive(Clone, Debug)]
pub enum Type {
    Bit(Span),
    Bits(Box<Expr>, Span),
    Array(Box<Type>, Box<Expr>, Span),
    Named(Ident),
    Uint(Span),
}

impl Type {
    pub fn span(&self) -> Span {
        match self {
            Type::Bit(s) | Type::Bits(_, s) | Type::Array(_, _, s) | Type::Uint(s) => *s,
            Type::Named(i) => i.span,
        }
    }
}

#[derive(Clone, Debug)]
pub enum LetPat {
    Name(Ident),
    Tuple(Vec<Ident>),
}

#[derive(Clone, Debug)]
pub enum LValue {
    Name(Ident),
    Index(Ident, Box<Expr>),
    Slice(Ident, Box<Expr>, Box<Expr>),
    /// `r.next`
    Next(Ident),
}

impl LValue {
    pub fn base(&self) -> &Ident {
        match self {
            LValue::Name(i) | LValue::Index(i, _) | LValue::Slice(i, _, _) | LValue::Next(i) => i,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Stmt {
    Let { pat: LetPat, ty: Option<Type>, value: Option<Expr>, span: Span },
    Assign { target: LValue, value: Expr, span: Span },
    Reg { name: Ident, ty: Type, init: Option<Expr>, span: Span },
    Mem { name: Ident, elem: Type, size: Expr, span: Span },
    Expr(Expr),
    For { var: Ident, start: Expr, end: Expr, inclusive: bool, body: Vec<Stmt>, span: Span },
    Const(ConstDef),
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Assign { span, .. }
            | Stmt::Reg { span, .. }
            | Stmt::Mem { span, .. }
            | Stmt::For { span, .. } => *span,
            Stmt::Expr(e) => e.span,
            Stmt::Const(c) => c.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub tail: Option<Box<Expr>>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,  // ~
    LNot, // !
    Neg,  // -
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Mul,
    Div,
    Rem,
    Add,
    Sub,
    Concat,
    Shl,
    Shr,
    Sar,
    And,
    Xor,
    Or,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    LAnd,
    LOr,
}

impl BinOp {
    pub fn text(self) -> &'static str {
        match self {
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Concat => "++",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Sar => ">>>",
            BinOp::And => "&",
            BinOp::Xor => "^",
            BinOp::Or => "|",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::LAnd => "&&",
            BinOp::LOr => "||",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Clone, Debug)]
pub enum Pat {
    Wild(Span),
    Int(u128, Span),
    Range(u128, u128, Span),
    Path(Ident, Ident),
    Const(Ident),
    Or(Vec<Pat>, Span),
}

impl Pat {
    pub fn span(&self) -> Span {
        match self {
            Pat::Wild(s) | Pat::Int(_, s) | Pat::Range(_, _, s) | Pat::Or(_, s) => *s,
            Pat::Path(a, b) => a.span.join(b.span),
            Pat::Const(i) => i.span,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Arm {
    pub pat: Pat,
    pub value: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    Int(u128),
    Bool(bool),
    Ident(Ident),
    Path(Ident, Ident),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Index(Box<Expr>, Box<Expr>),
    Slice(Box<Expr>, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, Ident),
    Call { callee: Ident, generics: Vec<Expr>, args: Vec<Arg> },
    Method { recv: Box<Expr>, name: Ident, args: Vec<Arg> },
    If(Box<Expr>, Block, Option<Box<Expr>>),
    Match(Box<Expr>, Vec<Arm>),
    Block(Block),
    Array(Vec<Expr>),
    Repeat(Box<Expr>, Box<Expr>),
    Tuple(Vec<Expr>),
    Paren(Box<Expr>),
    Error,
}

#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn error(span: Span) -> Expr {
        Expr { kind: ExprKind::Error, span }
    }
}
