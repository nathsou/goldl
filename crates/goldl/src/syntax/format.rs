//! Conservative formatter: re-indents by bracket depth, trims trailing whitespace and
//! collapses runs of blank lines. Comments and token spelling are preserved.

use super::lexer::{lex, Tok, P};

pub fn format(src: &str, indent: usize) -> String {
    let toks = lex(src);
    let hash_comments: Vec<_> = crate::asm::blocks(src)
        .iter()
        .flat_map(|b| crate::asm::classifications(b))
        .filter(|(_, kind)| *kind == "comment")
        .map(|(span, _)| span)
        .collect();
    // Depth at the start of each line, and whether the line starts with a closing bracket.
    let mut depth: i32 = 0;
    let mut line_depth: Vec<i32> = Vec::new();
    let mut line_starts_close: Vec<bool> = Vec::new();
    let mut line_first_tok = true;
    let mut cur_line_depth = 0;
    let mut cur_starts_close = false;
    let mut continuation: Vec<bool> = Vec::new();
    let mut cur_cont = false;
    for t in &toks {
        if hash_comments
            .iter()
            .any(|s| s.start <= t.span.start && t.span.start < s.end)
        {
            continue;
        }
        match t.tok {
            Tok::Newline => {
                line_depth.push(cur_line_depth);
                line_starts_close.push(cur_starts_close);
                continuation.push(cur_cont);
                line_first_tok = true;
                cur_line_depth = depth;
                cur_starts_close = false;
                cur_cont = false;
            }
            Tok::Eof => {}
            _ => {
                if line_first_tok {
                    cur_line_depth = depth;
                    cur_starts_close = matches!(
                        t.tok,
                        Tok::Punct(P::RBrace) | Tok::Punct(P::RParen) | Tok::Punct(P::RBracket)
                    );
                    cur_cont = matches!(
                        t.tok,
                        Tok::Punct(
                            P::AmpAmp
                                | P::PipePipe
                                | P::Amp
                                | P::Pipe
                                | P::Caret
                                | P::Plus
                                | P::Star
                                | P::EqEq
                                | P::Ne
                                | P::PlusPlus
                                | P::Dot
                        )
                    );
                    line_first_tok = false;
                }
                match t.tok {
                    Tok::Punct(P::LBrace | P::LParen | P::LBracket) => depth += 1,
                    Tok::Punct(P::RBrace | P::RParen | P::RBracket) => depth = (depth - 1).max(0),
                    _ => {}
                }
            }
        }
    }
    line_depth.push(cur_line_depth);
    line_starts_close.push(cur_starts_close);
    continuation.push(cur_cont);

    let mut out = String::new();
    let mut blank_run = 0;
    for (k, line) in src.split('\n').enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            if blank_run <= 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        blank_run = 0;
        let mut d = *line_depth.get(k).unwrap_or(&0);
        if *line_starts_close.get(k).unwrap_or(&false) {
            d -= 1;
        }
        if *continuation.get(k).unwrap_or(&false) {
            d += 1;
        }
        // Block comments spanning lines keep their own indentation.
        for _ in 0..(d.max(0) as usize * indent) {
            out.push(' ');
        }
        out.push_str(trimmed);
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn reindents() {
        let src = "module M(a: bit) -> (y: bit) {\nlet x = a\n      y = x\n}\n\n\n\nmodule N() -> (z: bit) {\n z = 1\n}\n";
        let f = super::format(src, 2);
        assert_eq!(f, "module M(a: bit) -> (y: bit) {\n  let x = a\n  y = x\n}\n\nmodule N() -> (z: bit) {\n  z = 1\n}\n");
        assert_eq!(super::format(&f, 2), f);
    }
}
