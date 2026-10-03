//! Recursive-descent + Pratt parser with error recovery.
//!
//! Statements are terminated by newlines or `;`. Newlines are ignored inside `()`/`[]` and
//! inside match bodies, and an expression continues onto the next line when that line
//! starts with a binary operator, `.` or `else`.

use super::ast::*;
use super::lexer::{lex, parse_int, Kw, Tok, Token, P};
use super::{Diag, Span};

pub struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    nl_ignore: u32,
    pub diags: Vec<Diag>,
}

type R<T> = Result<T, ()>;

pub fn parse(src: &str) -> (File, Vec<Diag>) {
    let toks: Vec<Token> = lex(src)
        .into_iter()
        .filter(|t| !matches!(t.tok, Tok::LineComment | Tok::BlockComment))
        .collect();
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        nl_ignore: 0,
        diags: Vec::new(),
    };
    let file = p.file();
    (file, p.diags)
}

fn bin_prec(t: Tok) -> Option<(u8, BinOp)> {
    let Tok::Punct(p) = t else { return None };
    Some(match p {
        P::PipePipe => (1, BinOp::LOr),
        P::AmpAmp => (2, BinOp::LAnd),
        P::EqEq => (3, BinOp::Eq),
        P::Ne => (3, BinOp::Ne),
        P::Lt => (3, BinOp::Lt),
        P::Le => (3, BinOp::Le),
        P::Gt => (3, BinOp::Gt),
        P::Ge => (3, BinOp::Ge),
        P::Pipe => (4, BinOp::Or),
        P::Caret => (5, BinOp::Xor),
        P::Amp => (6, BinOp::And),
        P::PlusPlus => (7, BinOp::Concat),
        P::Shl => (8, BinOp::Shl),
        P::Shr => (8, BinOp::Shr),
        P::Sar => (8, BinOp::Sar),
        P::Plus => (9, BinOp::Add),
        P::Minus => (9, BinOp::Sub),
        P::Star => (10, BinOp::Mul),
        P::Slash => (10, BinOp::Div),
        P::Percent => (10, BinOp::Rem),
        _ => return None,
    })
}

impl<'a> Parser<'a> {
    // ---- token helpers ----
    fn raw(&self) -> Token {
        self.toks[self.pos.min(self.toks.len() - 1)]
    }
    fn skip_nl(&mut self) {
        while self.raw().tok == Tok::Newline || self.raw().tok == Tok::DocComment {
            self.pos += 1;
        }
    }
    fn peek(&mut self) -> Token {
        if self.nl_ignore > 0 {
            self.skip_nl();
        }
        while self.raw().tok == Tok::DocComment {
            self.pos += 1;
        }
        self.raw()
    }
    /// Next significant token after any newlines (without consuming).
    fn peek_past_nl(&self) -> Token {
        let mut i = self.pos;
        while i < self.toks.len() && matches!(self.toks[i].tok, Tok::Newline | Tok::DocComment) {
            i += 1;
        }
        self.toks[i.min(self.toks.len() - 1)]
    }
    fn bump(&mut self) -> Token {
        let t = self.peek();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn at(&mut self, p: P) -> bool {
        self.peek().tok == Tok::Punct(p)
    }
    fn at_kw(&mut self, k: Kw) -> bool {
        self.peek().tok == Tok::Kw(k)
    }
    fn eat(&mut self, p: P) -> bool {
        if self.at(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, k: Kw) -> bool {
        if self.at_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn text(&self, t: Token) -> &'a str {
        &self.src[t.span.start as usize..t.span.end as usize]
    }
    fn err(&mut self, span: Span, msg: impl Into<String>) {
        // Avoid cascades of errors at the same position.
        if self
            .diags
            .last()
            .is_some_and(|d| d.span.start == span.start)
        {
            return;
        }
        self.diags.push(Diag::error(span, msg));
    }
    fn expect(&mut self, p: P) -> R<Token> {
        let t = self.peek();
        if t.tok == Tok::Punct(p) {
            Ok(self.bump())
        } else {
            let found = self.describe(t);
            self.err(t.span, format!("expected `{}`, found {found}", p.text()));
            Err(())
        }
    }
    fn describe(&self, t: Token) -> String {
        match t.tok {
            Tok::Eof => "end of file".into(),
            Tok::Newline => "end of line".into(),
            _ => format!("`{}`", self.text(t)),
        }
    }
    fn ident(&mut self) -> R<Ident> {
        let t = self.peek();
        if t.tok == Tok::Ident {
            self.bump();
            Ok(Ident {
                name: self.text(t).to_string(),
                span: t.span,
            })
        } else {
            let found = self.describe(t);
            self.err(t.span, format!("expected identifier, found {found}"));
            Err(())
        }
    }
    fn span_from(&self, start: Span) -> Span {
        let prev = if self.pos > 0 {
            self.toks[self.pos - 1].span
        } else {
            start
        };
        Span::new(start.start, prev.end.max(start.end))
    }
    /// Skip to the end of the current statement (newline, `;`, or closing brace).
    fn recover_stmt(&mut self) {
        let mut depth = 0i32;
        loop {
            let t = self.raw();
            match t.tok {
                Tok::Eof => return,
                Tok::Newline | Tok::Punct(P::Semi) if depth <= 0 => return,
                Tok::Punct(P::LBrace) | Tok::Punct(P::LParen) | Tok::Punct(P::LBracket) => {
                    depth += 1
                }
                Tok::Punct(P::RBrace) | Tok::Punct(P::RParen) | Tok::Punct(P::RBracket) => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.pos += 1;
        }
    }
    fn doc_before(&self) -> Option<String> {
        // Collect consecutive doc comments immediately preceding the current position.
        let mut i = self.pos;
        let mut lines = Vec::new();
        while i > 0 {
            i -= 1;
            match self.toks[i].tok {
                Tok::DocComment => lines.push(self.text(self.toks[i])[3..].trim().to_string()),
                Tok::Newline => {}
                _ => break,
            }
        }
        if lines.is_empty() {
            None
        } else {
            lines.reverse();
            Some(lines.join("\n"))
        }
    }

    // ---- items ----
    fn file(&mut self) -> File {
        let mut items = Vec::new();
        loop {
            self.skip_nl();
            let t = self.peek();
            if t.tok == Tok::Eof {
                break;
            }
            let start = self.pos;
            match self.item() {
                Ok(it) => items.push(it),
                Err(()) => {
                    // skip to the next item keyword at line start
                    if self.pos == start {
                        self.pos += 1;
                    }
                    while !matches!(
                        self.raw().tok,
                        Tok::Eof
                            | Tok::Kw(Kw::Module)
                            | Tok::Kw(Kw::Fn)
                            | Tok::Kw(Kw::Const)
                            | Tok::Kw(Kw::Enum)
                            | Tok::Kw(Kw::Test)
                            | Tok::Punct(P::Hash)
                    ) {
                        self.pos += 1;
                    }
                }
            }
        }
        File { items }
    }

    fn attrs(&mut self) -> R<Vec<Attr>> {
        let mut v = Vec::new();
        while self.at(P::Hash) {
            let s = self.bump().span;
            self.expect(P::LBracket)?;
            let name = self.ident()?;
            let mut args = Vec::new();
            if self.eat(P::LParen) {
                self.nl_ignore += 1;
                while !self.at(P::RParen) && self.peek().tok != Tok::Eof {
                    args.push(self.expr()?);
                    if !self.eat(P::Comma) {
                        break;
                    }
                }
                self.nl_ignore -= 1;
                self.expect(P::RParen)?;
            }
            self.expect(P::RBracket)?;
            v.push(Attr {
                name,
                args,
                span: self.span_from(s),
            });
            self.skip_nl();
        }
        Ok(v)
    }

    fn item(&mut self) -> R<Item> {
        let doc = self.doc_before();
        let attrs = self.attrs()?;
        let doc = doc.or_else(|| self.doc_before());
        let t = self.peek();
        match t.tok {
            Tok::Kw(Kw::Module) => self.module(attrs, doc).map(Item::Module),
            Tok::Kw(Kw::Fn) => self.fn_def(doc).map(Item::Fn),
            Tok::Kw(Kw::Const) => self.const_def(doc).map(Item::Const),
            Tok::Kw(Kw::Enum) => self.enum_def(doc).map(Item::Enum),
            Tok::Kw(Kw::Test) => self.test_def().map(Item::Test),
            _ => {
                let found = self.describe(t);
                self.err(
                    t.span,
                    format!(
                        "expected an item (`module`, `fn`, `const`, `enum`, `test`), found {found}"
                    ),
                );
                Err(())
            }
        }
    }

    fn generics(&mut self) -> R<Vec<Generic>> {
        let mut v = Vec::new();
        if self.eat(P::Lt) {
            self.nl_ignore += 1;
            while !self.at(P::Gt) {
                let name = self.ident()?;
                if self.eat(P::Colon) && !self.eat_kw(Kw::Uint) {
                    let t = self.peek();
                    self.err(t.span, "generic parameters must have type `uint`");
                }
                let default = if self.eat(P::Eq) {
                    Some(self.type_expr()?)
                } else {
                    None
                };
                v.push(Generic { name, default });
                if !self.eat(P::Comma) {
                    break;
                }
            }
            self.nl_ignore -= 1;
            self.expect(P::Gt)?;
        }
        Ok(v)
    }

    fn params(&mut self) -> R<Vec<Param>> {
        self.expect(P::LParen)?;
        self.nl_ignore += 1;
        let mut v = Vec::new();
        while !self.at(P::RParen) && self.peek().tok != Tok::Eof {
            let name = self.ident()?;
            self.expect(P::Colon)?;
            let ty = self.ty()?;
            v.push(Param { name, ty });
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.nl_ignore -= 1;
        self.expect(P::RParen)?;
        Ok(v)
    }

    fn module(&mut self, attrs: Vec<Attr>, doc: Option<String>) -> R<Module> {
        let s = self.bump().span;
        let name = self.ident()?;
        let generics = self.generics()?;
        let inputs = self.params()?;
        let outputs = if self.eat(P::Arrow) {
            if self.at(P::LParen) {
                self.params()?
            } else {
                let ty = self.ty()?;
                vec![Param {
                    name: Ident {
                        name: "out".into(),
                        span: ty.span(),
                    },
                    ty,
                }]
            }
        } else {
            Vec::new()
        };
        let body = self.brace_stmts()?;
        Ok(Module {
            attrs,
            doc,
            name,
            generics,
            inputs,
            outputs,
            body,
            span: self.span_from(s),
        })
    }

    fn fn_def(&mut self, doc: Option<String>) -> R<FnDef> {
        let s = self.bump().span;
        let name = self.ident()?;
        let generics = self.generics()?;
        let params = self.params()?;
        let ret = if self.eat(P::Arrow) {
            Some(self.ty()?)
        } else {
            None
        };
        let body = self.block()?;
        Ok(FnDef {
            doc,
            name,
            generics,
            params,
            ret,
            body,
            span: self.span_from(s),
        })
    }

    fn const_def(&mut self, doc: Option<String>) -> R<ConstDef> {
        let s = self.bump().span;
        let name = self.ident()?;
        let ty = if self.eat(P::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(P::Eq)?;
        let value = self.expr()?;
        Ok(ConstDef {
            doc,
            name,
            ty,
            value,
            span: self.span_from(s),
        })
    }

    fn enum_def(&mut self, doc: Option<String>) -> R<EnumDef> {
        let s = self.bump().span;
        let name = self.ident()?;
        let repr = if self.eat(P::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(P::LBrace)?;
        self.nl_ignore += 1;
        let mut variants = Vec::new();
        while !self.at(P::RBrace) && self.peek().tok != Tok::Eof {
            let v = self.ident()?;
            let val = if self.eat(P::Eq) {
                Some(self.expr()?)
            } else {
                None
            };
            variants.push((v, val));
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.nl_ignore -= 1;
        self.expect(P::RBrace)?;
        Ok(EnumDef {
            doc,
            name,
            repr,
            variants,
            span: self.span_from(s),
        })
    }

    fn test_def(&mut self) -> R<TestDef> {
        let s = self.bump().span;
        let t = self.peek();
        let name = if t.tok == Tok::Str {
            self.bump();
            self.text(t).trim_matches('"').to_string()
        } else {
            self.ident()?.name
        };
        if !self.eat_kw(Kw::For) {
            let t = self.peek();
            self.err(t.span, "expected `for <Module>` after the test name");
            return Err(());
        }
        let module = self.ident()?;
        self.expect(P::LBrace)?;
        let saved = self.nl_ignore;
        self.nl_ignore = 0;
        let mut body = Vec::new();
        loop {
            self.skip_nl();
            if self.at(P::RBrace) || self.peek().tok == Tok::Eof {
                break;
            }
            while self.eat(P::Semi) {}
            if self.at(P::RBrace) {
                break;
            }
            let st = self.peek().span;
            let r = if self.eat_kw(Kw::Step) {
                let count = if self.eat(P::LParen) {
                    let e = self.expr()?;
                    self.expect(P::RParen)?;
                    Some(e)
                } else {
                    None
                };
                Ok(TestStmt::Step {
                    count,
                    span: self.span_from(st),
                })
            } else if self.eat_kw(Kw::Assert) {
                self.expr().map(|cond| TestStmt::Assert {
                    cond,
                    span: self.span_from(st),
                })
            } else {
                (|| {
                    let port = self.ident()?;
                    self.expect(P::Eq)?;
                    let value = self.expr()?;
                    Ok(TestStmt::Poke {
                        port,
                        value,
                        span: self.span_from(st),
                    })
                })()
            };
            match r {
                Ok(x) => body.push(x),
                Err(()) => self.recover_stmt(),
            }
        }
        self.nl_ignore = saved;
        self.expect(P::RBrace)?;
        Ok(TestDef {
            name,
            module,
            body,
            span: self.span_from(s),
        })
    }

    // ---- types ----
    fn ty(&mut self) -> R<Type> {
        let t = self.peek();
        let s = t.span;
        let mut ty = match t.tok {
            Tok::Kw(Kw::Bit) => {
                self.bump();
                Type::Bit(s)
            }
            Tok::Kw(Kw::Bits) => {
                self.bump();
                self.expect(P::Lt)?;
                let w = self.type_expr()?;
                self.expect(P::Gt)?;
                Type::Bits(Box::new(w), self.span_from(s))
            }
            Tok::Kw(Kw::Uint) => {
                self.bump();
                Type::Uint(s)
            }
            Tok::Ident => Type::Named(self.ident()?),
            _ => {
                let found = self.describe(t);
                self.err(t.span, format!("expected a type, found {found}"));
                return Err(());
            }
        };
        while self.at(P::LBracket) {
            self.bump();
            let n = if self.at(P::Underscore) {
                Expr { kind: ExprKind::Infer, span: self.bump().span }
            } else {
                self.expr()?
            };
            self.expect(P::RBracket)?;
            ty = Type::Array(Box::new(ty), Box::new(n), self.span_from(s));
        }
        Ok(ty)
    }

    /// Expression inside `<...>`: no comparisons or shifts.
    fn type_expr(&mut self) -> R<Expr> {
        self.nl_ignore += 1;
        let r = self.expr_bp(7);
        self.nl_ignore -= 1;
        r
    }

    // ---- statements ----
    fn brace_stmts(&mut self) -> R<Vec<Stmt>> {
        let b = self.block()?;
        let mut stmts = b.stmts;
        if let Some(t) = b.tail {
            stmts.push(Stmt::Expr(*t));
        }
        Ok(stmts)
    }

    fn block(&mut self) -> R<Block> {
        let s = self.expect(P::LBrace)?.span;
        let saved = self.nl_ignore;
        self.nl_ignore = 0;
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        loop {
            self.skip_nl();
            while self.eat(P::Semi) {
                self.skip_nl();
            }
            let t = self.peek();
            if t.tok == Tok::Punct(P::RBrace) || t.tok == Tok::Eof {
                break;
            }
            if let Some(e) = tail.take() {
                stmts.push(Stmt::Expr(*e));
            }
            let before = self.pos;
            match self.stmt() {
                Ok(Stmt::Expr(e)) => {
                    // An expression statement that is not followed by `;` may be the tail.
                    if self.raw().tok == Tok::Punct(P::Semi) {
                        stmts.push(Stmt::Expr(e));
                    } else {
                        tail = Some(Box::new(e));
                    }
                }
                Ok(st) => stmts.push(st),
                Err(()) => {
                    self.recover_stmt();
                    if self.pos == before {
                        self.pos += 1;
                    }
                }
            }
            // statement terminator
            let t = self.raw();
            match t.tok {
                Tok::Newline | Tok::Punct(P::Semi) => {
                    self.pos += 1;
                }
                Tok::Punct(P::RBrace) | Tok::Eof => {}
                _ => {
                    let found = self.describe(t);
                    self.err(t.span, format!("expected end of statement, found {found}"));
                    self.recover_stmt();
                }
            }
        }
        self.nl_ignore = saved;
        let e = self.expect(P::RBrace);
        let span = self.span_from(s);
        e?;
        Ok(Block { stmts, tail, span })
    }

    fn stmt(&mut self) -> R<Stmt> {
        let t = self.peek();
        let s = t.span;
        match t.tok {
            Tok::Kw(Kw::Let) => {
                self.bump();
                let pat = if self.eat(P::LParen) {
                    let mut names = Vec::new();
                    while !self.at(P::RParen) {
                        names.push(self.ident()?);
                        if !self.eat(P::Comma) {
                            break;
                        }
                    }
                    self.expect(P::RParen)?;
                    LetPat::Tuple(names)
                } else {
                    LetPat::Name(self.ident()?)
                };
                let ty = if self.eat(P::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                let value = if self.eat(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                Ok(Stmt::Let {
                    pat,
                    ty,
                    value,
                    span: self.span_from(s),
                })
            }
            Tok::Kw(Kw::Reg) => {
                self.bump();
                let name = self.ident()?;
                self.expect(P::Colon)?;
                let ty = self.ty()?;
                let init = if self.eat(P::Eq) {
                    Some(self.expr()?)
                } else {
                    None
                };
                Ok(Stmt::Reg {
                    name,
                    ty,
                    init,
                    span: self.span_from(s),
                })
            }
            Tok::Kw(Kw::Mem) => {
                self.bump();
                let name = self.ident()?;
                self.expect(P::Colon)?;
                let ty = self.ty()?;
                match ty {
                    Type::Array(elem, size, _) => Ok(Stmt::Mem {
                        name,
                        elem: *elem,
                        size: *size,
                        span: self.span_from(s),
                    }),
                    other => {
                        self.err(
                            other.span(),
                            "a memory needs an array type, e.g. `bits<8>[16]`",
                        );
                        Err(())
                    }
                }
            }
            Tok::Kw(Kw::Const) => {
                let d = self.doc_before();
                Ok(Stmt::Const(self.const_def(d)?))
            }
            Tok::Kw(Kw::For) => {
                self.bump();
                let var = self.ident()?;
                if !self.eat_kw(Kw::In) {
                    let t = self.peek();
                    self.err(t.span, "expected `in`");
                    return Err(());
                }
                let start = self.expr_bp(8)?;
                let inclusive = if self.eat(P::DotDotEq) {
                    true
                } else {
                    self.expect(P::DotDot)?;
                    false
                };
                let end = self.expr_bp(8)?;
                let body = self.brace_stmts()?;
                Ok(Stmt::For {
                    var,
                    start,
                    end,
                    inclusive,
                    body,
                    span: self.span_from(s),
                })
            }
            _ => {
                let e = self.expr()?;
                if self.at(P::Eq) {
                    self.bump();
                    let target = self.lvalue(e)?;
                    let value = self.expr()?;
                    Ok(Stmt::Assign {
                        target,
                        value,
                        span: self.span_from(s),
                    })
                } else {
                    Ok(Stmt::Expr(e))
                }
            }
        }
    }

    fn lvalue(&mut self, e: Expr) -> R<LValue> {
        match e.kind {
            ExprKind::Ident(i) => Ok(LValue::Name(i)),
            ExprKind::Index(b, i) => match b.kind {
                ExprKind::Ident(n) => Ok(LValue::Index(n, i)),
                _ => {
                    self.err(e.span, "invalid assignment target");
                    Err(())
                }
            },
            ExprKind::Slice(b, hi, lo) => match b.kind {
                ExprKind::Ident(n) => Ok(LValue::Slice(n, hi, lo)),
                _ => {
                    self.err(e.span, "invalid assignment target");
                    Err(())
                }
            },
            ExprKind::Field(b, f) if f.name == "next" => match b.kind {
                ExprKind::Ident(n) => Ok(LValue::Next(n)),
                _ => {
                    self.err(e.span, "invalid assignment target");
                    Err(())
                }
            },
            _ => {
                self.err(
                    e.span,
                    "invalid assignment target (expected a name, `x[i]`, `x[hi:lo]` or `r.next`)",
                );
                Err(())
            }
        }
    }

    // ---- expressions ----
    pub fn expr(&mut self) -> R<Expr> {
        self.expr_bp(0)
    }

    fn continues_expr(&self) -> bool {
        let t = self.peek_past_nl();
        bin_prec(t.tok).is_some() || t.tok == Tok::Punct(P::Dot)
    }

    fn expr_bp(&mut self, min: u8) -> R<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let mut t = self.peek();
            if t.tok == Tok::Newline && self.continues_expr() {
                self.skip_nl();
                t = self.peek();
            }
            let Some((prec, op)) = bin_prec(t.tok) else {
                break;
            };
            if prec <= min {
                break;
            }
            self.bump();
            if self.nl_ignore == 0 {
                self.skip_nl();
            }
            let rhs = self.expr_bp(prec)?;
            let span = lhs.span.join(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)),
                span,
            };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> R<Expr> {
        let t = self.peek();
        let op = match t.tok {
            Tok::Punct(P::Tilde) => Some(UnOp::Not),
            Tok::Punct(P::Bang) => Some(UnOp::LNot),
            Tok::Punct(P::Minus) => Some(UnOp::Neg),
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let e = self.unary()?;
            let span = t.span.join(e.span);
            return Ok(Expr {
                kind: ExprKind::Unary(op, Box::new(e)),
                span,
            });
        }
        self.postfix()
    }

    fn args(&mut self) -> R<Vec<Arg>> {
        self.expect(P::LParen)?;
        self.nl_ignore += 1;
        let mut args = Vec::new();
        while !self.at(P::RParen) && self.peek().tok != Tok::Eof {
            // named argument `name: expr`
            let named = self.peek().tok == Tok::Ident
                && self
                    .toks
                    .get(self.pos + 1)
                    .is_some_and(|t| t.tok == Tok::Punct(P::Colon));
            let name = if named {
                let n = self.ident()?;
                self.bump();
                Some(n)
            } else {
                None
            };
            let value = self.expr()?;
            args.push(Arg { name, value });
            if !self.eat(P::Comma) {
                break;
            }
        }
        self.nl_ignore -= 1;
        self.expect(P::RParen)?;
        Ok(args)
    }

    fn postfix(&mut self) -> R<Expr> {
        let mut e = self.primary()?;
        loop {
            let t = self.peek();
            let dot_next = t.tok == Tok::Newline && self.peek_past_nl().tok == Tok::Punct(P::Dot);
            if dot_next {
                self.skip_nl();
            }
            match self.peek().tok {
                Tok::Punct(P::LBracket) => {
                    self.bump();
                    self.nl_ignore += 1;
                    let a = self.expr()?;
                    let r = if self.eat(P::Colon) {
                        let b = self.expr()?;
                        self.nl_ignore -= 1;
                        let close = self.expect(P::RBracket)?;
                        let span = e.span.join(close.span);
                        Expr {
                            kind: ExprKind::Slice(Box::new(e), Box::new(a), Box::new(b)),
                            span,
                        }
                    } else {
                        self.nl_ignore -= 1;
                        let close = self.expect(P::RBracket)?;
                        let span = e.span.join(close.span);
                        Expr {
                            kind: ExprKind::Index(Box::new(e), Box::new(a)),
                            span,
                        }
                    };
                    e = r;
                }
                Tok::Punct(P::Dot) => {
                    self.bump();
                    let name = self.ident()?;
                    if self.at(P::LParen) {
                        let args = self.args()?;
                        let span = self.span_from(e.span);
                        e = Expr {
                            kind: ExprKind::Method {
                                recv: Box::new(e),
                                name,
                                args,
                            },
                            span,
                        };
                    } else {
                        let span = e.span.join(name.span);
                        e = Expr {
                            kind: ExprKind::Field(Box::new(e), name),
                            span,
                        };
                    }
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn try_generic_args(&mut self) -> Option<Vec<Expr>> {
        // Speculatively parse `<e, ...>(`.
        let save = self.pos;
        let saved_diags = self.diags.len();
        if !self.eat(P::Lt) {
            return None;
        }
        let mut gs = Vec::new();
        let ok = (|| -> R<()> {
            loop {
                gs.push(self.type_expr()?);
                if !self.eat(P::Comma) {
                    break;
                }
            }
            self.expect(P::Gt)?;
            Ok(())
        })();
        if ok.is_ok() && self.at(P::LParen) {
            Some(gs)
        } else {
            self.pos = save;
            self.diags.truncate(saved_diags);
            None
        }
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.peek();
        let s = t.span;
        match t.tok {
            Tok::Int => {
                self.bump();
                match parse_int(self.text(t)) {
                    Some(v) => Ok(Expr {
                        kind: ExprKind::Int(v),
                        span: s,
                    }),
                    None => {
                        self.err(s, "invalid integer literal");
                        Ok(Expr::error(s))
                    }
                }
            }
            Tok::Kw(Kw::True) => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span: s,
                })
            }
            Tok::Kw(Kw::False) => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span: s,
                })
            }
            Tok::Ident => {
                let id = self.ident()?;
                if self.at(P::ColonColon) {
                    self.bump();
                    let v = self.ident()?;
                    let span = id.span.join(v.span);
                    return Ok(Expr {
                        kind: ExprKind::Path(id, v),
                        span,
                    });
                }
                if self.at(P::Lt) {
                    if let Some(generics) = self.try_generic_args() {
                        let args = self.args()?;
                        let span = self.span_from(s);
                        return Ok(Expr {
                            kind: ExprKind::Call {
                                callee: id,
                                generics,
                                args,
                            },
                            span,
                        });
                    }
                }
                if self.at(P::LParen) {
                    let args = self.args()?;
                    let span = self.span_from(s);
                    return Ok(Expr {
                        kind: ExprKind::Call {
                            callee: id,
                            generics: vec![],
                            args,
                        },
                        span,
                    });
                }
                Ok(Expr {
                    kind: ExprKind::Ident(id.clone()),
                    span: id.span,
                })
            }
            Tok::Punct(P::LParen) => {
                self.bump();
                self.nl_ignore += 1;
                let first = self.expr()?;
                if self.eat(P::Comma) {
                    let mut v = vec![first];
                    while !self.at(P::RParen) {
                        v.push(self.expr()?);
                        if !self.eat(P::Comma) {
                            break;
                        }
                    }
                    self.nl_ignore -= 1;
                    self.expect(P::RParen)?;
                    return Ok(Expr {
                        kind: ExprKind::Tuple(v),
                        span: self.span_from(s),
                    });
                }
                self.nl_ignore -= 1;
                self.expect(P::RParen)?;
                Ok(Expr {
                    kind: ExprKind::Paren(Box::new(first)),
                    span: self.span_from(s),
                })
            }
            Tok::Punct(P::LBracket) => {
                self.bump();
                self.nl_ignore += 1;
                let mut v = Vec::new();
                if !self.at(P::RBracket) {
                    let first = self.expr()?;
                    if self.eat(P::Semi) {
                        let n = self.expr()?;
                        self.nl_ignore -= 1;
                        self.expect(P::RBracket)?;
                        return Ok(Expr {
                            kind: ExprKind::Repeat(Box::new(first), Box::new(n)),
                            span: self.span_from(s),
                        });
                    }
                    v.push(first);
                    while self.eat(P::Comma) {
                        if self.at(P::RBracket) {
                            break;
                        }
                        v.push(self.expr()?);
                    }
                }
                self.nl_ignore -= 1;
                self.expect(P::RBracket)?;
                Ok(Expr {
                    kind: ExprKind::Array(v),
                    span: self.span_from(s),
                })
            }
            Tok::Punct(P::LBrace) => {
                let b = self.block()?;
                let span = b.span;
                Ok(Expr {
                    kind: ExprKind::Block(b),
                    span,
                })
            }
            Tok::Kw(Kw::If) => self.if_expr(),
            Tok::Kw(Kw::Match) => self.match_expr(),
            Tok::Kw(Kw::Asm) => self.asm_expr(),
            _ => {
                let found = self.describe(t);
                self.err(t.span, format!("expected an expression, found {found}"));
                Err(())
            }
        }
    }

    fn asm_expr(&mut self) -> R<Expr> {
        let start = self.bump().span;
        if !self.eat_kw(Kw::For) {
            let span = self.peek().span;
            self.err(span, "expected `for` after `asm`");
            return Err(());
        }
        let isa = self.ident()?;
        let open = self.expect(P::LBrace)?;
        // The assembler owns the interior grammar. Preserve it verbatim, including
        // newlines, and do not interpret HDL expressions or keywords inside it.
        while !matches!(self.raw().tok, Tok::Punct(P::RBrace) | Tok::Eof) {
            if self.raw().tok == Tok::Punct(P::Hash) {
                while !matches!(self.raw().tok, Tok::Newline | Tok::Eof) {
                    self.pos += 1;
                }
            } else {
                self.pos += 1;
            }
        }
        let close = self.expect(P::RBrace)?;
        let body_span = Span::new(open.span.end, close.span.start);
        Ok(Expr {
            kind: ExprKind::Asm(AsmBlock {
                isa,
                body: self.src[body_span.start as usize..body_span.end as usize].into(),
                body_span,
            }),
            span: start.join(close.span),
        })
    }

    fn if_expr(&mut self) -> R<Expr> {
        let s = self.bump().span;
        let saved = self.nl_ignore;
        self.nl_ignore = 0;
        let cond = self.expr_no_brace()?;
        self.nl_ignore = saved;
        let then = self.block()?;
        let els = if self.peek_past_nl().tok == Tok::Kw(Kw::Else) {
            self.skip_nl();
            self.bump();
            if self.at_kw(Kw::If) {
                Some(Box::new(self.if_expr()?))
            } else {
                let b = self.block()?;
                let span = b.span;
                Some(Box::new(Expr {
                    kind: ExprKind::Block(b),
                    span,
                }))
            }
        } else {
            None
        };
        Ok(Expr {
            kind: ExprKind::If(Box::new(cond), then, els),
            span: self.span_from(s),
        })
    }

    /// Condition / scrutinee expression: a `{` ends it (no struct literals in GoLDL anyway).
    fn expr_no_brace(&mut self) -> R<Expr> {
        self.expr()
    }

    fn pat(&mut self) -> R<Pat> {
        let first = self.pat1()?;
        if self.at(P::Pipe) {
            let s = first.span();
            let mut v = vec![first];
            while self.eat(P::Pipe) {
                v.push(self.pat1()?);
            }
            let span = self.span_from(s);
            return Ok(Pat::Or(v, span));
        }
        Ok(first)
    }

    fn pat1(&mut self) -> R<Pat> {
        let t = self.peek();
        match t.tok {
            Tok::Punct(P::Underscore) => {
                self.bump();
                Ok(Pat::Wild(t.span))
            }
            Tok::Int => {
                self.bump();
                let a = parse_int(self.text(t)).unwrap_or(0);
                if self.eat(P::DotDotEq) {
                    let u = self.peek();
                    if u.tok != Tok::Int {
                        self.err(u.span, "expected integer after `..=`");
                        return Err(());
                    }
                    self.bump();
                    let b = parse_int(self.text(u)).unwrap_or(0);
                    return Ok(Pat::Range(a, b, t.span.join(u.span)));
                }
                Ok(Pat::Int(a, t.span))
            }
            Tok::Ident => {
                let a = self.ident()?;
                if self.eat(P::ColonColon) {
                    let b = self.ident()?;
                    Ok(Pat::Path(a, b))
                } else {
                    Ok(Pat::Const(a))
                }
            }
            _ => {
                let found = self.describe(t);
                self.err(t.span, format!("expected a pattern, found {found}"));
                Err(())
            }
        }
    }

    fn match_expr(&mut self) -> R<Expr> {
        let s = self.bump().span;
        let saved = self.nl_ignore;
        self.nl_ignore = 0;
        let scrut = self.expr_no_brace()?;
        self.expect(P::LBrace)?;
        self.nl_ignore = 1;
        let mut arms = Vec::new();
        while !self.at(P::RBrace) && self.peek().tok != Tok::Eof {
            let ps = self.peek().span;
            let r = (|| -> R<Arm> {
                let pat = self.pat()?;
                self.expect(P::FatArrow)?;
                let value = self.expr()?;
                Ok(Arm {
                    pat,
                    value,
                    span: self.span_from(ps),
                })
            })();
            match r {
                Ok(a) => arms.push(a),
                Err(()) => {
                    // skip to next comma or closing brace
                    while !matches!(
                        self.peek().tok,
                        Tok::Punct(P::Comma) | Tok::Punct(P::RBrace) | Tok::Eof
                    ) {
                        self.bump();
                    }
                }
            }
            if !self.eat(P::Comma) && !self.at(P::RBrace) {
                let t = self.peek();
                let found = self.describe(t);
                self.err(
                    t.span,
                    format!("expected `,` or `}}` after match arm, found {found}"),
                );
                break;
            }
        }
        self.nl_ignore = saved;
        self.expect(P::RBrace)?;
        Ok(Expr {
            kind: ExprKind::Match(Box::new(scrut), arms),
            span: self.span_from(s),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALU: &str = r#"
module Alu(a: bits<8>, b: bits<8>, op: bits<3>) -> (y: bits<8>, c: bit, v: bit) {
  let subtract: bit = op == 1
  let addend: bits<8> = if subtract { ~b } else { b }
  let carry_in: bits<9> = if subtract { 1 } else { 0 }
  let wide: bits<9> = zext(a, 9) + zext(addend, 9) + carry_in
  let arithmetic: bit = op == 0 || op == 1
  let result: bits<8> = match op {
    _ => wide[7:0],
    2 => a & b,
    3 => a | b,
    4 => a ^ b,
    6 => a >> 1,
    7 => ~a,
  }
  y = result
  c = if arithmetic { wide[8] != subtract } else if op == 6 { a[0] } else { 0 }
  v = arithmetic && a[7] == addend[7] && result[7] != a[7]
}
"#;

    #[test]
    fn parses_alu() {
        let (f, d) = parse(ALU);
        assert!(d.is_empty(), "{d:?}");
        let Item::Module(m) = &f.items[0] else {
            panic!()
        };
        assert_eq!(m.name.name, "Alu");
        assert_eq!(m.inputs.len(), 3);
        assert_eq!(m.outputs.len(), 3);
        assert_eq!(m.body.len(), 9);
    }

    #[test]
    fn generics_and_regs() {
        let src = "module Counter<N: uint = 4>(en: bit) -> (count: bits<N>) {\n  reg r: bits<N> = 0\n  r.next = if en { r + 1 } else { r }\n  count = r\n}\nmodule Top(x: bit) -> (y: bits<4>) {\n  let c = Counter<4>(en: x)\n  y = c.count\n}\n";
        let (f, d) = parse(src);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(f.items.len(), 2);
    }

    #[test]
    fn multiline_continuation() {
        let src = "module M(a: bit, b: bit) -> (y: bit) {\n  y = a\n    && b\n}\n";
        let (f, d) = parse(src);
        assert!(d.is_empty(), "{d:?}");
        let Item::Module(m) = &f.items[0] else {
            panic!()
        };
        assert_eq!(m.body.len(), 1);
    }

    #[test]
    fn recovers() {
        let src = "module M(a: bit) -> (y: bit) {\n  let = 3\n  y = a\n}\nmodule N() -> (z: bit) { z = 1 }\n";
        let (f, d) = parse(src);
        assert_eq!(d.len(), 1, "{d:?}");
        assert_eq!(f.items.len(), 2);
    }
}
