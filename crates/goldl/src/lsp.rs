//! Language server for GoLDL (LSP over JSON-RPC). Transport-agnostic: `Server::handle` takes
//! one decoded message and returns the messages to send back (responses and
//! notifications). Used by the playground (through WASM) and by `goldl lsp` (stdio).

use crate::elab::{analyze, Analysis, DefKind};
use crate::json::Json;
use crate::syntax::lexer::{lex, Kw, Tok};
use crate::syntax::{LineIndex, Severity, Span};
use std::collections::HashMap;

/// Builtin functions: (name, signature, documentation).
pub const BUILTINS: &[(&str, &str, &str)] = &[
    ("zext", "zext(x, n)", "Zero-extend `x` to `n` bits."),
    ("sext", "sext(x, n)", "Sign-extend `x` to `n` bits."),
    ("trunc", "trunc(x, n)", "Keep the low `n` bits of `x`."),
    ("cat", "cat(a, b, ...)", "Concatenate, most significant first."),
    ("rep", "rep(x, n)", "Repeat `x` `n` times."),
    ("clog2", "clog2(n)", "Ceiling of log2 (compile-time)."),
    ("width", "width(x)", "Bit width of `x` (compile-time)."),
    ("any", "any(x)", "OR of all bits."),
    ("all", "all(x)", "AND of all bits."),
    ("parity", "parity(x)", "XOR of all bits."),
    ("mux", "mux(sel, a, b)", "`a` if `sel` else `b`."),
    ("slt", "slt(a, b)", "Signed a < b."),
    ("sle", "sle(a, b)", "Signed a <= b."),
    ("sgt", "sgt(a, b)", "Signed a > b."),
    ("sge", "sge(a, b)", "Signed a >= b."),
];

const KW_DOCS: &[(&str, &str)] = &[
    ("module", "A hardware module: `module Name<N: uint = 8>(inputs) -> (outputs) { ... }`."),
    ("fn", "A combinational function, inlined at every call."),
    ("let", "A named wire: `let x: bits<8> = expr` (or declared, then assigned bit by bit)."),
    ("reg", "A register: `reg r: bits<8> = init`, updated with `r.next = expr` every clock cycle."),
    ("mem", "A memory: `mem m: bits<8>[16]`, written with `m.write(addr, data, enable)`, read with `m[addr]`."),
    ("const", "A compile-time constant or ROM: `const T: bits<8>[4] = [...]`."),
    ("match", "Select by value; arms must be disjoint, `_` is the default wherever it appears."),
    ("for", "Compile-time loop (unrolled): `for i in 0..N { ... }`."),
    ("test", "A test bench: `test \"name\" for Module { a = 1; step; assert y == 2 }`."),
    ("bits", "Bit vector type `bits<N>`."),
    ("bit", "Single bit, same as `bits<1>`."),
];

pub const TOKEN_TYPES: &[&str] = &["keyword", "type", "function", "variable", "parameter", "property", "number", "string", "comment", "operator", "class", "enumMember", "macro"];
pub const TOKEN_MODS: &[&str] = &["declaration", "readonly", "defaultLibrary", "documentation"];

pub struct Doc {
    pub text: String,
    pub version: i64,
    pub an: Analysis,
    pub li: LineIndex,
}

impl Doc {
    pub fn new(text: String, version: i64) -> Doc {
        let an = analyze(&text);
        let li = LineIndex::new(&text);
        Doc { text, version, an, li }
    }
    fn pos(&self, off: u32) -> Json {
        let (l, c) = self.li.line_col(off);
        Json::obj().with("line", l).with("character", c)
    }
    pub fn range(&self, s: Span) -> Json {
        Json::obj().with("start", self.pos(s.start)).with("end", self.pos(s.end))
    }
    fn offset(&self, p: &Json) -> u32 {
        self.li.offset(p.get("line").as_i64().unwrap_or(0) as u32, p.get("character").as_i64().unwrap_or(0) as u32)
    }
    /// Definition index referenced or defined at `off`.
    fn def_at(&self, off: u32) -> Option<usize> {
        let info = &self.an.info;
        if let Some(&(_, d)) = info.refs.iter().find(|(s, _)| s.start <= off && off <= s.end) {
            return Some(d);
        }
        info.defs.iter().position(|d| d.span.start <= off && off <= d.span.end)
    }
    fn word_at(&self, off: u32) -> Option<(Span, &str)> {
        let b = self.text.as_bytes();
        let is_id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
        let mut s = off as usize;
        while s > 0 && is_id(b[s - 1]) {
            s -= 1;
        }
        let mut e = off as usize;
        while e < b.len() && is_id(b[e]) {
            e += 1;
        }
        (e > s).then(|| (Span::new(s as u32, e as u32), &self.text[s..e]))
    }

    pub fn diagnostics(&self) -> Json {
        let v: Vec<Json> = self
            .an
            .diags
            .iter()
            .map(|d| {
                let sev = match d.severity {
                    Severity::Error => 1,
                    Severity::Warning => 2,
                    Severity::Info => 3,
                };
                let mut j = Json::obj().with("range", self.range(d.span)).with("severity", sev).with("source", "goldl").with("message", d.message.clone());
                if !d.notes.is_empty() {
                    let rel: Vec<Json> = d.notes.iter().map(|(s, m)| Json::obj().with("message", m.clone()).with("location", Json::obj().with("uri", "").with("range", self.range(*s)))).collect();
                    j.set("relatedInformation", rel);
                }
                j
            })
            .collect();
        Json::Arr(v)
    }

    pub fn hover(&self, off: u32) -> Json {
        let info = &self.an.info;
        if let Some(d) = self.def_at(off) {
            let def = &info.defs[d];
            let span = info.refs.iter().find(|(s, _)| s.start <= off && off <= s.end).map_or(def.span, |r| r.0);
            let mut md = format!("```goldl\n{}\n```", def.detail);
            if let Some(doc) = &def.doc {
                md.push_str("\n\n");
                md.push_str(doc);
            }
            return Json::obj().with("contents", Json::obj().with("kind", "markdown").with("value", md)).with("range", self.range(span));
        }
        if let Some((span, w)) = self.word_at(off) {
            if let Some((_, sig, doc)) = BUILTINS.iter().find(|b| b.0 == w) {
                let md = format!("```goldl\n{sig}\n```\n\n{doc}");
                return Json::obj().with("contents", Json::obj().with("kind", "markdown").with("value", md)).with("range", self.range(span));
            }
            if let Some((_, doc)) = KW_DOCS.iter().find(|k| k.0 == w) {
                return Json::obj().with("contents", Json::obj().with("kind", "markdown").with("value", doc.to_string())).with("range", self.range(span));
            }
        }
        Json::Null
    }

    pub fn definition(&self, off: u32, uri: &str) -> Json {
        match self.def_at(off) {
            Some(d) => Json::obj().with("uri", uri).with("range", self.range(self.an.info.defs[d].span)),
            None => Json::Null,
        }
    }

    pub fn references(&self, off: u32, uri: &str, include_decl: bool) -> Json {
        let Some(d) = self.def_at(off) else { return Json::Arr(vec![]) };
        let info = &self.an.info;
        let mut v: Vec<Json> = Vec::new();
        if include_decl {
            v.push(Json::obj().with("uri", uri).with("range", self.range(info.defs[d].span)));
        }
        for &(s, k) in &info.refs {
            if k == d {
                v.push(Json::obj().with("uri", uri).with("range", self.range(s)));
            }
        }
        Json::Arr(v)
    }

    /// Spans of the definition and all uses of the symbol at `off` (for highlights and rename).
    pub fn occurrences(&self, off: u32) -> Vec<Span> {
        let Some(d) = self.def_at(off) else { return vec![] };
        let info = &self.an.info;
        let mut v = vec![info.defs[d].span];
        v.extend(info.refs.iter().filter(|r| r.1 == d).map(|r| r.0));
        v
    }

    pub fn completion(&self, off: u32) -> Json {
        let info = &self.an.info;
        let mut items: Vec<Json> = Vec::new();
        let mut seen: HashMap<String, ()> = HashMap::new();
        let prefix_dot = off > 0 && self.text.as_bytes().get(off as usize - 1) == Some(&b'.');
        // After `.`: register/memory/instance members.
        if prefix_dot {
            for (label, detail) in [("next", "register next state"), ("write", "mem.write(addr, data, enable)")] {
                items.push(Json::obj().with("label", label).with("kind", 5).with("detail", detail));
            }
            for d in &info.defs {
                if d.kind == DefKind::Output {
                    if seen.insert(d.name.clone(), ()).is_none() {
                        items.push(Json::obj().with("label", d.name.clone()).with("kind", 5).with("detail", d.detail.clone()));
                    }
                }
            }
            return Json::obj().with("isIncomplete", false).with("items", items);
        }
        for d in &info.defs {
            let visible = d.scope.contains(off) || d.scope == Span::default() || matches!(d.kind, DefKind::Module | DefKind::Function | DefKind::Const | DefKind::Enum);
            if !visible || d.span.start > off && d.scope.contains(off) && !matches!(d.kind, DefKind::Module | DefKind::Function | DefKind::Const) {
                continue;
            }
            if seen.insert(d.name.clone(), ()).is_some() {
                continue;
            }
            let kind = match d.kind {
                DefKind::Module => 7,
                DefKind::Function => 3,
                DefKind::Const => 21,
                DefKind::Enum => 13,
                DefKind::Variant => 20,
                DefKind::Input | DefKind::Output => 5,
                DefKind::Reg | DefKind::Mem => 6,
                DefKind::Generic => 25,
                DefKind::Instance => 9,
                _ => 6,
            };
            let mut it = Json::obj().with("label", d.name.clone()).with("kind", kind).with("detail", d.detail.clone());
            if let Some(doc) = &d.doc {
                it.set("documentation", doc.clone());
            }
            items.push(it);
        }
        for (name, sig, doc) in BUILTINS {
            items.push(Json::obj().with("label", *name).with("kind", 3).with("detail", *sig).with("documentation", *doc).with("insertText", format!("{name}($1)")).with("insertTextFormat", 2));
        }
        for kw in Kw::ALL {
            items.push(Json::obj().with("label", kw).with("kind", 14));
        }
        for (label, body) in [
            ("module", "module ${1:Name}(${2:a}: bits<${3:8}>) -> (${4:y}: bits<${3:8}>) {\n  $0\n}"),
            ("reg", "reg ${1:r}: bits<${2:8}> = ${3:0}\n${1:r}.next = $0"),
            ("match", "match ${1:x} {\n  _ => $0,\n}"),
            ("for", "for ${1:i} in 0..${2:N} {\n  $0\n}"),
            ("test", "test \"${1:name}\" for ${2:Module} {\n  $0\n  step\n}"),
        ] {
            items.push(Json::obj().with("label", label).with("kind", 15).with("detail", "snippet").with("insertText", body).with("insertTextFormat", 2));
        }
        Json::obj().with("isIncomplete", false).with("items", items)
    }

    pub fn formatting(&self, tab: usize) -> Json {
        let f = crate::syntax::format::format(&self.text, tab);
        if f == self.text {
            return Json::Arr(vec![]);
        }
        let end = self.text.len() as u32;
        Json::Arr(vec![Json::obj().with("range", self.range(Span::new(0, end))).with("newText", f)])
    }

    /// Semantic tokens as (offset, length, type, modifiers), in document order.
    pub fn semantic_tokens_raw(&self) -> Vec<(u32, u32, u32, u32)> {
        let info = &self.an.info;
        let ty = |k: &str| TOKEN_TYPES.iter().position(|t| *t == k).unwrap() as u32;
        let mut by_span: HashMap<u32, (u32, u32)> = HashMap::new();
        let classify = |kind: DefKind| -> &str {
            match kind {
                DefKind::Module => "class",
                DefKind::Function => "function",
                DefKind::Const => "macro",
                DefKind::Enum => "type",
                DefKind::Variant => "enumMember",
                DefKind::Input | DefKind::Output => "parameter",
                DefKind::Generic => "parameter",
                DefKind::Reg | DefKind::Mem => "property",
                DefKind::Instance => "class",
                _ => "variable",
            }
        };
        for d in &info.defs {
            by_span.insert(d.span.start, (ty(classify(d.kind)), 1));
        }
        for &(s, k) in &info.refs {
            by_span.entry(s.start).or_insert((ty(classify(info.defs[k].kind)), 0));
        }
        let mut out = Vec::new();
        for t in lex(&self.text) {
            let (st, m) = match t.tok {
                Tok::Kw(Kw::Bit | Kw::Bits | Kw::Uint) => (ty("type"), 0),
                Tok::Kw(Kw::True | Kw::False) => (ty("number"), 0),
                Tok::Kw(_) => (ty("keyword"), 0),
                Tok::Int => (ty("number"), 0),
                Tok::Str => (ty("string"), 0),
                Tok::LineComment | Tok::BlockComment => (ty("comment"), 0),
                Tok::DocComment => (ty("comment"), 8),
                Tok::Punct(_) => (ty("operator"), 0),
                Tok::Ident => match by_span.get(&t.span.start) {
                    Some(&(k, decl)) => (k, decl),
                    None => {
                        let w = &self.text[t.span.start as usize..t.span.end as usize];
                        if BUILTINS.iter().any(|b| b.0 == w) {
                            (ty("function"), 4)
                        } else {
                            (ty("variable"), 0)
                        }
                    }
                },
                _ => continue,
            };
            out.push((t.span.start, t.span.end - t.span.start, st, m));
        }
        out
    }

    pub fn semantic_tokens(&self) -> Json {
        let mut data: Vec<Json> = Vec::new();
        let (mut pl, mut pc) = (0u32, 0u32);
        for (off, len, t, m) in self.semantic_tokens_raw() {
            // Tokens spanning lines (block comments) are emitted per line.
            let text = &self.text[off as usize..(off + len) as usize];
            let mut o = off;
            for part in text.split('\n') {
                let (l, c) = self.li.line_col(o);
                let n = part.encode_utf16().count() as u32;
                if n > 0 {
                    let dl = l - pl;
                    let dc = if dl == 0 { c - pc } else { c };
                    data.extend([dl, dc, n, t, m].map(Json::from));
                    pl = l;
                    pc = c;
                }
                o += part.len() as u32 + 1;
            }
        }
        Json::obj().with("data", data)
    }

    pub fn inlay_hints(&self, range: Option<(u32, u32)>) -> Json {
        let v: Vec<Json> = self
            .an
            .info
            .inlays
            .iter()
            .filter(|(o, _)| range.map_or(true, |(a, b)| *o >= a && *o <= b))
            .map(|(o, l)| Json::obj().with("position", self.pos(*o)).with("label", l.clone()).with("kind", 1).with("paddingLeft", false))
            .collect();
        Json::Arr(v)
    }

    pub fn document_symbols(&self) -> Json {
        let info = &self.an.info;
        let mut v = Vec::new();
        for d in &info.defs {
            let kind = match d.kind {
                DefKind::Module => 2,
                DefKind::Function => 12,
                DefKind::Const => 14,
                DefKind::Enum => 10,
                DefKind::Reg | DefKind::Mem => 13,
                DefKind::Instance => 19,
                _ => continue,
            };
            v.push(Json::obj().with("name", d.name.clone()).with("detail", d.detail.clone()).with("kind", kind).with("range", self.range(d.span)).with("selectionRange", self.range(d.span)));
        }
        Json::Arr(v)
    }

    pub fn signature_help(&self, off: u32) -> Json {
        // Find the innermost unclosed `name(` before the cursor.
        let b = self.text.as_bytes();
        let mut depth = 0i32;
        let mut commas = 0;
        let mut i = off as usize;
        while i > 0 {
            i -= 1;
            match b[i] {
                b')' => depth += 1,
                b'(' => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                b',' if depth == 0 => commas += 1,
                b'\n' if depth == 0 && i + 1 < off as usize && b.get(i + 1) == Some(&b'\n') => return Json::Null,
                _ => {}
            }
        }
        if i == 0 && b.first() != Some(&b'(') {
            return Json::Null;
        }
        let Some((_, name)) = self.word_at(i as u32) else { return Json::Null };
        let sig = BUILTINS.iter().find(|x| x.0 == name).map(|x| (x.1.to_string(), x.2.to_string())).or_else(|| {
            self.an.info.defs.iter().find(|d| d.name == name && matches!(d.kind, DefKind::Function | DefKind::Module)).map(|d| (d.detail.clone(), d.doc.clone().unwrap_or_default()))
        });
        let Some((label, doc)) = sig else { return Json::Null };
        Json::obj().with("signatures", vec![Json::obj().with("label", label).with("documentation", doc)]).with("activeSignature", 0).with("activeParameter", commas)
    }
}

/// The language server state.
#[derive(Default)]
pub struct Server {
    pub docs: HashMap<String, Doc>,
    pub tab_size: usize,
    shutdown: bool,
}

fn response(id: &Json, result: Json) -> Json {
    Json::obj().with("jsonrpc", "2.0").with("id", id.clone()).with("result", result)
}

fn notification(method: &str, params: Json) -> Json {
    Json::obj().with("jsonrpc", "2.0").with("method", method).with("params", params)
}

impl Server {
    pub fn new() -> Server {
        Server { tab_size: 2, ..Default::default() }
    }

    pub fn capabilities() -> Json {
        Json::obj()
            .with("textDocumentSync", 1)
            .with("hoverProvider", true)
            .with("definitionProvider", true)
            .with("referencesProvider", true)
            .with("documentHighlightProvider", true)
            .with("renameProvider", true)
            .with("documentFormattingProvider", true)
            .with("documentSymbolProvider", true)
            .with("inlayHintProvider", true)
            .with("completionProvider", Json::obj().with("triggerCharacters", vec!["."]))
            .with("signatureHelpProvider", Json::obj().with("triggerCharacters", vec!["(", ","]))
            .with(
                "semanticTokensProvider",
                Json::obj().with("legend", Json::obj().with("tokenTypes", TOKEN_TYPES.to_vec()).with("tokenModifiers", TOKEN_MODS.to_vec())).with("full", true),
            )
    }

    fn publish(&self, uri: &str) -> Json {
        let d = &self.docs[uri];
        notification("textDocument/publishDiagnostics", Json::obj().with("uri", uri).with("version", d.version).with("diagnostics", d.diagnostics()))
    }

    /// Whether `exit` was received after `shutdown`.
    pub fn exited(&self) -> bool {
        self.shutdown
    }

    /// Handle one JSON-RPC message; returns responses/notifications to send.
    pub fn handle(&mut self, msg: &Json) -> Vec<Json> {
        let method = msg.get("method").as_str().unwrap_or("");
        let id = msg.get("id");
        let params = msg.get("params");
        let uri = params.get("textDocument").get("uri").as_str().unwrap_or("").to_string();
        let mut out = Vec::new();
        match method {
            "initialize" => {
                if let Some(t) = params.get("initializationOptions").get("tabSize").as_i64() {
                    self.tab_size = t as usize;
                }
                out.push(response(id, Json::obj().with("capabilities", Self::capabilities()).with("serverInfo", Json::obj().with("name", "goldl").with("version", env!("CARGO_PKG_VERSION")))));
            }
            "initialized" | "$/cancelRequest" | "$/setTrace" => {}
            "shutdown" => {
                self.shutdown = true;
                out.push(response(id, Json::Null));
            }
            "exit" => self.shutdown = true,
            "textDocument/didOpen" => {
                let td = params.get("textDocument");
                let text = td.get("text").as_str().unwrap_or("").to_string();
                self.docs.insert(uri.clone(), Doc::new(text, td.get("version").as_i64().unwrap_or(0)));
                out.push(self.publish(&uri));
            }
            "textDocument/didChange" => {
                let changes = params.get("contentChanges").as_arr();
                if let Some(last) = changes.last() {
                    let text = last.get("text").as_str().unwrap_or("").to_string();
                    let v = params.get("textDocument").get("version").as_i64().unwrap_or(0);
                    self.docs.insert(uri.clone(), Doc::new(text, v));
                    out.push(self.publish(&uri));
                }
            }
            "textDocument/didClose" => {
                self.docs.remove(&uri);
                out.push(notification("textDocument/publishDiagnostics", Json::obj().with("uri", uri.as_str()).with("diagnostics", Json::Arr(vec![]))));
            }
            "workspace/didChangeConfiguration" => {
                if let Some(t) = params.get("settings").get("goldl").get("tabSize").as_i64() {
                    self.tab_size = t as usize;
                }
            }
            _ if !id.is_null() => {
                let result = match self.docs.get(&uri) {
                    None => Json::Null,
                    Some(d) => {
                        let off = d.offset(params.get("position"));
                        match method {
                            "textDocument/hover" => d.hover(off),
                            "textDocument/definition" => d.definition(off, &uri),
                            "textDocument/references" => d.references(off, &uri, params.get("context").get("includeDeclaration").as_bool().unwrap_or(true)),
                            "textDocument/documentHighlight" => Json::Arr(d.occurrences(off).into_iter().map(|s| Json::obj().with("range", d.range(s))).collect()),
                            "textDocument/rename" => {
                                let new = params.get("newName").as_str().unwrap_or("");
                                let edits: Vec<Json> = d.occurrences(off).into_iter().map(|s| Json::obj().with("range", d.range(s)).with("newText", new)).collect();
                                let mut changes = Json::obj();
                                changes.set(&uri, edits);
                                Json::obj().with("changes", changes)
                            }
                            "textDocument/completion" => d.completion(off),
                            "textDocument/formatting" => {
                                let tab = params.get("options").get("tabSize").as_i64().map_or(self.tab_size, |t| t as usize);
                                d.formatting(tab)
                            }
                            "textDocument/semanticTokens/full" => d.semantic_tokens(),
                            "textDocument/inlayHint" => {
                                let r = params.get("range");
                                let range = (!r.is_null()).then(|| (d.offset(r.get("start")), d.offset(r.get("end"))));
                                d.inlay_hints(range)
                            }
                            "textDocument/documentSymbol" => d.document_symbols(),
                            "textDocument/signatureHelp" => d.signature_help(off),
                            _ => {
                                out.push(Json::obj().with("jsonrpc", "2.0").with("id", id.clone()).with("error", Json::obj().with("code", -32601).with("message", format!("method not found: {method}"))));
                                return out;
                            }
                        }
                    }
                };
                out.push(response(id, result));
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(id: i64, method: &str, params: Json) -> Json {
        Json::obj().with("jsonrpc", "2.0").with("id", id).with("method", method).with("params", params)
    }

    #[test]
    fn basic_session() {
        let mut s = Server::new();
        let r = s.handle(&req(1, "initialize", Json::obj()));
        assert!(r[0].get("result").get("capabilities").get("hoverProvider").as_bool().unwrap());
        let src = "module M(a: bits<4>) -> (y: bits<4>) {\n  let t = a + 1\n  y = t\n}\n";
        let open = Json::obj().with("jsonrpc", "2.0").with("method", "textDocument/didOpen").with("params", Json::obj().with("textDocument", Json::obj().with("uri", "file:///m.goldl").with("version", 1).with("text", src)));
        let r = s.handle(&open);
        assert_eq!(r[0].get("method").as_str(), Some("textDocument/publishDiagnostics"));
        assert!(r[0].get("params").get("diagnostics").as_arr().is_empty(), "{}", r[0]);
        let pos = |l: i64, c: i64| Json::obj().with("textDocument", Json::obj().with("uri", "file:///m.goldl")).with("position", Json::obj().with("line", l).with("character", c));
        // Hover on `t` in `y = t`.
        let h = s.handle(&req(2, "textDocument/hover", pos(2, 6)));
        assert!(h[0].get("result").get("contents").get("value").as_str().unwrap().contains("t"), "{}", h[0]);
        // Definition of `t`.
        let d = s.handle(&req(3, "textDocument/definition", pos(2, 6)));
        assert_eq!(d[0].get("result").get("range").get("start").get("line").as_i64(), Some(1));
        let c = s.handle(&req(4, "textDocument/completion", pos(2, 4)));
        assert!(c[0].get("result").get("items").as_arr().iter().any(|i| i.get("label").as_str() == Some("zext")));
        let t = s.handle(&req(5, "textDocument/semanticTokens/full", Json::obj().with("textDocument", Json::obj().with("uri", "file:///m.goldl"))));
        assert!(t[0].get("result").get("data").as_arr().len() > 20);
    }
}
