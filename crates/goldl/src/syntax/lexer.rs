//! Lexer for GoLDL source.

use super::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tok {
    Ident,
    Int,
    Str,
    Kw(Kw),
    Punct(P),
    Newline,
    LineComment,
    DocComment,
    BlockComment,
    Error,
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kw {
    Module,
    Fn,
    Let,
    Reg,
    Mem,
    Const,
    Enum,
    If,
    Else,
    Match,
    For,
    In,
    Test,
    Bit,
    Bits,
    Uint,
    True,
    False,
    Step,
    Assert,
}

impl Kw {
    pub fn from_str(s: &str) -> Option<Kw> {
        Some(match s {
            "module" => Kw::Module,
            "fn" => Kw::Fn,
            "let" => Kw::Let,
            "reg" => Kw::Reg,
            "mem" => Kw::Mem,
            "const" => Kw::Const,
            "enum" => Kw::Enum,
            "if" => Kw::If,
            "else" => Kw::Else,
            "match" => Kw::Match,
            "for" => Kw::For,
            "in" => Kw::In,
            "test" => Kw::Test,
            "bit" => Kw::Bit,
            "bits" => Kw::Bits,
            "uint" => Kw::Uint,
            "true" => Kw::True,
            "false" => Kw::False,
            "step" => Kw::Step,
            "assert" => Kw::Assert,
            _ => return None,
        })
    }
    pub const ALL: [&'static str; 20] = [
        "module", "fn", "let", "reg", "mem", "const", "enum", "if", "else", "match", "for", "in",
        "test", "bit", "bits", "uint", "true", "false", "step", "assert",
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum P {
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Colon,
    ColonColon,
    Semi,
    Dot,
    DotDot,
    DotDotEq,
    Arrow,
    FatArrow,
    Eq,
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Plus,
    PlusPlus,
    Minus,
    Star,
    Slash,
    Percent,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    Caret,
    Tilde,
    Bang,
    Shl,
    Shr,
    Sar,
    Hash,
    Underscore,
    Question,
}

impl P {
    pub fn text(self) -> &'static str {
        match self {
            P::LParen => "(",
            P::RParen => ")",
            P::LBrace => "{",
            P::RBrace => "}",
            P::LBracket => "[",
            P::RBracket => "]",
            P::Comma => ",",
            P::Colon => ":",
            P::ColonColon => "::",
            P::Semi => ";",
            P::Dot => ".",
            P::DotDot => "..",
            P::DotDotEq => "..=",
            P::Arrow => "->",
            P::FatArrow => "=>",
            P::Eq => "=",
            P::EqEq => "==",
            P::Ne => "!=",
            P::Lt => "<",
            P::Le => "<=",
            P::Gt => ">",
            P::Ge => ">=",
            P::Plus => "+",
            P::PlusPlus => "++",
            P::Minus => "-",
            P::Star => "*",
            P::Slash => "/",
            P::Percent => "%",
            P::Amp => "&",
            P::AmpAmp => "&&",
            P::Pipe => "|",
            P::PipePipe => "||",
            P::Caret => "^",
            P::Tilde => "~",
            P::Bang => "!",
            P::Shl => "<<",
            P::Shr => ">>",
            P::Sar => ">>>",
            P::Hash => "#",
            P::Underscore => "_",
            P::Question => "?",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
}

const PUNCTS: &[(&str, P)] = &[
    ("..=", P::DotDotEq),
    (">>>", P::Sar),
    ("::", P::ColonColon),
    ("..", P::DotDot),
    ("->", P::Arrow),
    ("=>", P::FatArrow),
    ("==", P::EqEq),
    ("!=", P::Ne),
    ("<=", P::Le),
    (">=", P::Ge),
    ("++", P::PlusPlus),
    ("&&", P::AmpAmp),
    ("||", P::PipePipe),
    ("<<", P::Shl),
    (">>", P::Shr),
    ("(", P::LParen),
    (")", P::RParen),
    ("{", P::LBrace),
    ("}", P::RBrace),
    ("[", P::LBracket),
    ("]", P::RBracket),
    (",", P::Comma),
    (":", P::Colon),
    (";", P::Semi),
    (".", P::Dot),
    ("=", P::Eq),
    ("<", P::Lt),
    (">", P::Gt),
    ("+", P::Plus),
    ("-", P::Minus),
    ("*", P::Star),
    ("/", P::Slash),
    ("%", P::Percent),
    ("&", P::Amp),
    ("|", P::Pipe),
    ("^", P::Caret),
    ("~", P::Tilde),
    ("!", P::Bang),
    ("#", P::Hash),
    ("?", P::Question),
];

/// Tokenize the whole source (comments and newlines included).
pub fn lex(src: &str) -> Vec<Token> {
    let b = src.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    let tok = |t: Tok, s: usize, e: usize| Token {
        tok: t,
        span: Span::new(s as u32, e as u32),
    };
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'\n' {
            i += 1;
            out.push(tok(Tok::Newline, start, i));
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            let doc = i + 2 < b.len() && b[i + 2] == b'/';
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push(tok(
                if doc {
                    Tok::DocComment
                } else {
                    Tok::LineComment
                },
                start,
                i,
            ));
            continue;
        }
        if c == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            let mut depth = 1;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            out.push(tok(Tok::BlockComment, start, i));
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let s = &src[start..i];
            let t = if s == "_" {
                Tok::Punct(P::Underscore)
            } else if let Some(k) = Kw::from_str(s) {
                Tok::Kw(k)
            } else {
                Tok::Ident
            };
            out.push(tok(t, start, i));
            continue;
        }
        if c.is_ascii_digit() {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(tok(Tok::Int, start, i));
            continue;
        }
        if c == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            if i < b.len() && b[i] == b'"' {
                i += 1;
            }
            out.push(tok(Tok::Str, start, i));
            continue;
        }
        let mut matched = false;
        for &(s, p) in PUNCTS {
            if src[i..].starts_with(s) {
                i += s.len();
                out.push(tok(Tok::Punct(p), start, i));
                matched = true;
                break;
            }
        }
        if !matched {
            // Skip one UTF-8 character.
            i += 1;
            while i < b.len() && (b[i] & 0xC0) == 0x80 {
                i += 1;
            }
            out.push(tok(Tok::Error, start, i));
        }
    }
    out.push(tok(Tok::Eof, b.len(), b.len()));
    out
}

/// Parse an integer literal (`123`, `0x7f`, `0b1010`, `1_000`).
pub fn parse_int(s: &str) -> Option<u128> {
    let s: String = s.chars().filter(|&c| c != '_').collect();
    if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u128::from_str_radix(h, 16).ok()
    } else if let Some(bn) = s.strip_prefix("0b").or_else(|| s.strip_prefix("0B")) {
        u128::from_str_radix(bn, 2).ok()
    } else if let Some(o) = s.strip_prefix("0o") {
        u128::from_str_radix(o, 8).ok()
    } else {
        s.parse().ok()
    }
}
