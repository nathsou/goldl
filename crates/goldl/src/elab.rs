//! Elaboration (semantic analysis): AST → flat word-level RTL.
//!
//! * Statements in a module body are order-independent dataflow: names are bound lazily and
//!   forced on demand, which also detects combinational loops.
//! * Widths are checked strictly; unsized integer literals adapt to the expected width.
//! * Module instances are flattened (each instance becomes a provenance group).
//! * Name resolution and types are recorded for the language server.

use crate::rtl::{mask, ROp, RId, Rtl};
use crate::syntax::ast::*;
use crate::syntax::{Diag, Span};
use crate::gnl::PortInfo;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefKind {
    Module,
    Function,
    Const,
    Enum,
    Variant,
    Input,
    Output,
    Wire,
    Reg,
    Mem,
    Generic,
    Instance,
    LoopVar,
}

#[derive(Clone, Debug)]
pub struct DefInfo {
    pub name: String,
    pub span: Span,
    pub kind: DefKind,
    pub detail: String,
    pub doc: Option<String>,
    /// Enclosing item span (for scoping completions).
    pub scope: Span,
}

#[derive(Clone, Debug, Default)]
pub struct SemaInfo {
    pub defs: Vec<DefInfo>,
    /// (use span, index into defs)
    pub refs: Vec<(Span, usize)>,
    /// Inlay hints: (offset, label)
    pub inlays: Vec<(u32, String)>,
    /// Signal names → RTL node, for hover values and probes: (def span, node) of the last
    /// elaboration of the top module.
    pub signals: Vec<(Span, String, RId)>,
}

#[derive(Clone, Debug)]
enum Val {
    Sig(RId),
    Int(i128),
    Arr(Vec<Val>),
    Tuple(Vec<Val>),
    Inst(Vec<(String, Val)>),
    Mem(usize),
    Err,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TypeV {
    Bits(u32),
    Arr(Box<TypeV>, u32),
    Int,
    Err,
}

impl TypeV {
    fn show(&self) -> String {
        match self {
            TypeV::Bits(1) => "bit".into(),
            TypeV::Bits(w) => format!("bits<{w}>"),
            TypeV::Arr(t, n) => format!("{}[{n}]", t.show()),
            TypeV::Int => "uint".into(),
            TypeV::Err => "?".into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Bind {
    Thunk(usize),
    TupleElem(usize, usize),
    Wire(usize),
    Reg(usize),
    Mem(usize),
    Val(usize),
}

struct Scope {
    parent: Option<usize>,
    names: HashMap<String, (Bind, Span)>,
}

enum ThunkState {
    Pending,
    InProgress,
    Done(Val),
}

struct Thunk<'a> {
    expr: &'a Expr,
    ty: Option<&'a Type>,
    scope: usize,
    state: ThunkState,
    name: String,
    def: Span,
}

#[derive(Clone)]
struct AssignE<'a> {
    lv: &'a LValue,
    expr: &'a Expr,
    scope: usize,
    span: Span,
    /// Bit range for partial assignments (None = whole wire).
    range: Option<(u32, u32)>,
    value: Option<RId>,
    busy: bool,
}

struct Wire<'a> {
    name: String,
    width: u32,
    assigns: Vec<AssignE<'a>>,
    value: Option<RId>,
    in_progress: bool,
    def: Span,
    is_output: bool,
}

struct RegE<'a> {
    idx: usize,
    width: u32,
    nexts: Vec<(&'a Expr, usize, Span)>,
    def: Span,
}

struct MemE<'a> {
    regs: Vec<usize>,
    width: u32,
    writes: Vec<(&'a [Arg], usize, Span)>,
    group: u32,
    def: Span,
}

/// Per-module-instance elaboration state.
struct Inst<'a> {
    scopes: Vec<Scope>,
    thunks: Vec<Thunk<'a>>,
    wires: Vec<Wire<'a>>,
    regs: Vec<RegE<'a>>,
    mems: Vec<MemE<'a>>,
    vals: Vec<Val>,
    group: u32,
    path: String,
    record: bool,
}

pub struct Elab<'a> {
    file: &'a File,
    items: HashMap<String, &'a Item>,
    pub rtl: Rtl,
    pub diags: Vec<Diag>,
    pub info: SemaInfo,
    def_index: HashMap<(u32, u32), usize>,
    stack: Vec<Inst<'a>>,
    global_consts: HashMap<String, Val>,
    global_busy: Vec<String>,
    depth: u32,
    record_signals: bool,
}

fn clog2(n: i128) -> i128 {
    let mut k = 0;
    while (1i128 << k) < n {
        k += 1;
    }
    k
}

impl<'a> Elab<'a> {
    pub fn new(file: &'a File) -> Self {
        let mut items = HashMap::new();
        let mut diags = Vec::new();
        for it in &file.items {
            if let Some(n) = it.name() {
                if let Some(prev) = items.insert(n.name.clone(), it) {
                    diags.push(Diag::error(n.span, format!("`{}` is defined more than once", n.name)).with_note(prev.name().unwrap().span, "first definition here"));
                }
            }
        }
        let mut e = Elab {
            file,
            items,
            rtl: Rtl::new("top"),
            diags,
            info: SemaInfo::default(),
            def_index: HashMap::new(),
            stack: Vec::new(),
            global_consts: HashMap::new(),
            global_busy: Vec::new(),
            depth: 0,
            record_signals: false,
        };
        // Item-level definitions.
        for it in &file.items {
            match it {
                Item::Module(m) => {
                    let sig = module_signature(m);
                    e.def(&m.name, DefKind::Module, sig, m.doc.clone(), m.span);
                }
                Item::Fn(f) => {
                    let params: Vec<String> = f.params.iter().map(|p| format!("{}: {}", p.name.name, type_src(&p.ty))).collect();
                    let ret = f.ret.as_ref().map(|t| format!(" -> {}", type_src(t))).unwrap_or_default();
                    e.def(&f.name, DefKind::Function, format!("fn {}({}){ret}", f.name.name, params.join(", ")), f.doc.clone(), f.span);
                }
                Item::Const(c) => {
                    let ty = c.ty.as_ref().map(type_src).unwrap_or_else(|| "uint".into());
                    e.def(&c.name, DefKind::Const, format!("const {}: {ty}", c.name.name), c.doc.clone(), c.span);
                }
                Item::Enum(en) => {
                    let repr = en.repr.as_ref().map(type_src).unwrap_or_else(|| "bits<?>".into());
                    e.def(&en.name, DefKind::Enum, format!("enum {}: {repr}", en.name.name), en.doc.clone(), en.span);
                    for (v, _) in &en.variants {
                        e.def(v, DefKind::Variant, format!("{}::{}", en.name.name, v.name), None, en.span);
                    }
                }
                Item::Test(_) => {}
            }
        }
        e
    }

    // ---- diagnostics & LSP bookkeeping ----
    fn err(&mut self, span: Span, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.diags.iter().any(|d| d.span == span && d.message == msg) {
            self.diags.push(Diag::error(span, msg));
        }
    }
    fn warn(&mut self, span: Span, msg: impl Into<String>) {
        let msg = msg.into();
        if !self.diags.iter().any(|d| d.span == span && d.message == msg) {
            self.diags.push(Diag::warning(span, msg));
        }
    }
    fn recording(&self) -> bool {
        self.stack.last().map_or(true, |i| i.record)
    }
    fn def(&mut self, id: &Ident, kind: DefKind, detail: String, doc: Option<String>, scope: Span) -> usize {
        let key = (id.span.start, id.span.end);
        if let Some(&i) = self.def_index.get(&key) {
            if self.info.defs[i].detail.contains('?') && !detail.contains('?') {
                self.info.defs[i].detail = detail;
            }
            return i;
        }
        self.info.defs.push(DefInfo { name: id.name.clone(), span: id.span, kind, detail, doc, scope });
        let i = self.info.defs.len() - 1;
        self.def_index.insert(key, i);
        i
    }
    fn reference(&mut self, use_span: Span, def_span: Span) {
        if let Some(&i) = self.def_index.get(&(def_span.start, def_span.end)) {
            if !self.info.refs.iter().any(|&(s, _)| s == use_span) {
                self.info.refs.push((use_span, i));
            }
        }
    }
    fn set_detail(&mut self, def_span: Span, detail: String) {
        if let Some(&i) = self.def_index.get(&(def_span.start, def_span.end)) {
            self.info.defs[i].detail = detail;
        }
    }

    fn cur(&mut self) -> &mut Inst<'a> {
        self.stack.last_mut().unwrap()
    }
    fn group(&self) -> u32 {
        self.stack.last().map_or(0, |i| i.group)
    }

    // ---- scopes ----
    fn new_scope(&mut self, parent: Option<usize>) -> usize {
        let inst = self.cur();
        inst.scopes.push(Scope { parent, names: HashMap::new() });
        inst.scopes.len() - 1
    }
    fn bind(&mut self, scope: usize, id: &Ident, b: Bind) {
        let inst = self.cur();
        if let Some((_, prev)) = inst.scopes[scope].names.get(&id.name) {
            let prev = *prev;
            self.diags.push(Diag::error(id.span, format!("`{}` is already defined in this scope", id.name)).with_note(prev, "previous definition"));
            return;
        }
        inst.scopes[scope].names.insert(id.name.clone(), (b, id.span));
    }
    fn lookup(&self, scope: usize, name: &str) -> Option<(Bind, Span)> {
        let inst = self.stack.last()?;
        let mut s = Some(scope);
        while let Some(k) = s {
            if let Some(&b) = inst.scopes[k].names.get(name) {
                return Some(b);
            }
            s = inst.scopes[k].parent;
        }
        None
    }

    // ---- values ----
    fn width(&self, id: RId) -> u32 {
        self.rtl.width(id)
    }
    fn konst(&mut self, v: i128, w: u32) -> RId {
        self.rtl.konst(v as u64, w)
    }
    fn node(&mut self, op: ROp, w: u32, span: Span) -> RId {
        let g = self.group();
        self.rtl.node(op, w, g, span)
    }
    fn node_g(&mut self, op: ROp, w: u32, g: u32, span: Span) -> RId {
        self.rtl.node(op, w, g, span)
    }

    /// Convert a value to a signal of width `w` (reporting width errors).
    fn coerce(&mut self, v: &Val, w: u32, span: Span) -> RId {
        match v {
            Val::Sig(id) => {
                let have = self.width(*id);
                if have != w {
                    self.err(span, format!("width mismatch: expected bits<{w}>, found bits<{have}>"));
                    return self.resize(*id, w, span);
                }
                *id
            }
            Val::Int(x) => {
                let x = *x;
                let fits = if x >= 0 { w >= 64 || (x as u128) < (1u128 << w) } else { w >= 64 || x >= -(1i128 << (w - 1).min(126)) };
                if !fits {
                    self.err(span, format!("literal {x} does not fit in bits<{w}>"));
                }
                self.konst(x, w)
            }
            Val::Err => self.konst(0, w),
            _ => {
                self.err(span, format!("expected a bits<{w}> value"));
                self.konst(0, w)
            }
        }
    }

    fn resize(&mut self, id: RId, w: u32, span: Span) -> RId {
        let have = self.width(id);
        if have == w {
            id
        } else if have < w {
            self.node(ROp::Zext(id), w, span)
        } else {
            self.node(ROp::Slice(id, 0), w, span)
        }
    }

    /// Value as a signal of its natural width (unsized literals need an expected width).
    fn sig(&mut self, v: &Val, expected: Option<u32>, span: Span) -> Option<RId> {
        match v {
            Val::Sig(id) => Some(*id),
            Val::Int(x) => {
                let w = expected.unwrap_or_else(|| (128 - (*x).max(0).leading_zeros()).max(1));
                Some(self.coerce(&Val::Int(*x), w, span))
            }
            Val::Err => None,
            _ => {
                self.err(span, "expected a bit-vector value");
                None
            }
        }
    }

    fn const_int(&mut self, e: &'a Expr, scope: usize) -> Option<i128> {
        let v = self.eval(e, scope, None);
        match v {
            Val::Int(x) => Some(x),
            Val::Sig(id) => match self.rtl.const_val(id) {
                Some(c) => Some(c as i128),
                None => {
                    self.err(e.span, "expected a constant expression");
                    None
                }
            },
            Val::Err => None,
            _ => {
                self.err(e.span, "expected a constant integer");
                None
            }
        }
    }

    fn eval_type(&mut self, t: &'a Type, scope: usize) -> TypeV {
        match t {
            Type::Bit(_) => TypeV::Bits(1),
            Type::Bits(e, _) => match self.const_int(e, scope) {
                Some(w) if (1..=64).contains(&w) => TypeV::Bits(w as u32),
                Some(w) => {
                    self.err(e.span, format!("bit widths must be between 1 and 64 (got {w})"));
                    TypeV::Err
                }
                None => TypeV::Err,
            },
            Type::Array(t, n, _) => {
                let et = self.eval_type(t, scope);
                match self.const_int(n, scope) {
                    Some(n) if n > 0 && n <= 4096 => TypeV::Arr(Box::new(et), n as u32),
                    _ => {
                        self.err(n.span, "array size must be a constant between 1 and 4096");
                        TypeV::Err
                    }
                }
            }
            Type::Uint(_) => TypeV::Int,
            Type::Named(id) => match self.items.get(&id.name) {
                Some(Item::Enum(en)) => {
                    self.reference(id.span, en.name.span);
                    TypeV::Bits(self.enum_width(en))
                }
                _ => {
                    self.err(id.span, format!("unknown type `{}`", id.name));
                    TypeV::Err
                }
            },
        }
    }

    fn enum_width(&mut self, en: &'a EnumDef) -> u32 {
        if let Some(Type::Bits(e, _)) = &en.repr {
            if let ExprKind::Int(w) = e.kind {
                return w as u32;
            }
        }
        if let Some(Type::Bit(_)) = en.repr {
            return 1;
        }
        let n = en.variants.len() as i128;
        clog2(n.max(2)) as u32
    }

    fn enum_value(&mut self, en: &'a EnumDef, variant: &str) -> Option<i128> {
        let mut next = 0i128;
        for (v, val) in &en.variants {
            let x = match val {
                Some(e) => match e.kind {
                    ExprKind::Int(x) => x as i128,
                    _ => next,
                },
                None => next,
            };
            if v.name == variant {
                return Some(x);
            }
            next = x + 1;
        }
        None
    }

    // ---- module elaboration ----

    /// Elaborate `m` as the top module of a fresh RTL.
    pub fn elab_top(&mut self, m: &'a Module, generics: &[i128]) -> bool {
        self.rtl = Rtl::new(&m.name.name);
        self.record_signals = true;
        let scope_g = 0u32;
        let mut inputs = Vec::new();
        // Evaluate generics and port widths in a temporary instance context.
        self.stack.push(Inst { scopes: vec![], thunks: vec![], wires: vec![], regs: vec![], mems: vec![], vals: vec![], group: scope_g, path: String::new(), record: true });
        let sc = self.new_scope(None);
        let gvals = self.bind_generics(m, generics, sc);
        let mut ok = true;
        for (k, p) in m.inputs.iter().enumerate() {
            let t = self.eval_type(&p.ty, sc);
            match t {
                TypeV::Bits(w) => {
                    self.rtl.inputs.push(PortInfo { name: p.name.name.clone(), width: w });
                    let id = self.rtl.input(k as u32, w, p.name.span);
                    inputs.push(Val::Sig(id));
                }
                _ => {
                    self.err(p.ty.span(), "top-level ports must be bit vectors");
                    ok = false;
                    inputs.push(Val::Err);
                }
            }
        }
        self.stack.pop();
        let outs = self.instantiate(m, gvals, inputs, "", 0, m.name.span, true);
        for (name, v) in outs {
            let p = m.outputs.iter().find(|p| p.name.name == name).unwrap();
            let id = match &v {
                Val::Sig(id) => *id,
                _ => {
                    ok = false;
                    self.rtl.konst(0, 1)
                }
            };
            let w = self.rtl.width(id);
            self.rtl.outputs.push((PortInfo { name, width: w }, id));
            let _ = p;
        }
        ok && !self.diags.iter().any(|d| d.severity == crate::syntax::Severity::Error)
    }

    fn bind_generics(&mut self, m: &'a Module, given: &[i128], scope: usize) -> Vec<i128> {
        let mut vals = Vec::new();
        for (k, g) in m.generics.iter().enumerate() {
            let v = if let Some(&v) = given.get(k) {
                v
            } else if let Some(d) = &g.default {
                self.const_int(d, scope).unwrap_or(1)
            } else {
                self.err(g.name.span, format!("generic parameter `{}` needs a value (no default)", g.name.name));
                1
            };
            let i = self.cur().vals.len();
            self.cur().vals.push(Val::Int(v));
            self.bind(scope, &g.name, Bind::Val(i));
            vals.push(v);
        }
        vals
    }

    /// Elaborate an instance of module `m`; returns its outputs.
    fn instantiate(&mut self, m: &'a Module, generics: Vec<i128>, inputs: Vec<Val>, inst_name: &str, parent_group: u32, call_span: Span, record: bool) -> Vec<(String, Val)> {
        self.depth += 1;
        if self.depth > 64 {
            self.err(call_span, "module instantiation is nested too deeply (recursive module?)");
            self.depth -= 1;
            return m.outputs.iter().map(|p| (p.name.name.clone(), Val::Err)).collect();
        }
        let path = match self.stack.last() {
            Some(p) if !p.path.is_empty() => format!("{}.{inst_name}", p.path),
            _ => inst_name.to_string(),
        };
        let group = if inst_name.is_empty() { 0 } else { self.rtl.add_group(&format!("{inst_name}: {}", m.name.name), "module", parent_group) };
        let rec = record && self.stack.last().map_or(true, |i| i.record);
        self.stack.push(Inst { scopes: vec![], thunks: vec![], wires: vec![], regs: vec![], mems: vec![], vals: vec![], group, path, record: rec });
        let sc = self.new_scope(None);
        // Generics.
        for (k, g) in m.generics.iter().enumerate() {
            let v = generics.get(k).copied().unwrap_or(1);
            let i = self.cur().vals.len();
            self.cur().vals.push(Val::Int(v));
            self.bind(sc, &g.name, Bind::Val(i));
            self.def(&g.name, DefKind::Generic, format!("{}: uint = {v}", g.name.name), None, m.span);
        }
        // Inputs.
        for (k, p) in m.inputs.iter().enumerate() {
            let t = self.eval_type(&p.ty, sc);
            let v = inputs.get(k).cloned().unwrap_or(Val::Err);
            let v = match (&t, &v) {
                (TypeV::Bits(w), Val::Sig(id)) => {
                    let have = self.width(*id);
                    if have != *w {
                        self.err(call_span, format!("argument `{}`: expected bits<{w}>, found bits<{have}>", p.name.name));
                        Val::Sig(self.resize(*id, *w, call_span))
                    } else {
                        v.clone()
                    }
                }
                (TypeV::Bits(w), Val::Int(_)) => Val::Sig(self.coerce(&v, *w, call_span)),
                _ => v.clone(),
            };
            if let Val::Sig(id) = v {
                if self.record_signals && self.stack.len() == 1 {
                    self.info.signals.push((p.name.span, p.name.name.clone(), id));
                }
            }
            let i = self.cur().vals.len();
            self.cur().vals.push(v);
            self.bind(sc, &p.name, Bind::Val(i));
            self.def(&p.name, DefKind::Input, format!("input {}: {}", p.name.name, t.show()), None, m.span);
        }
        // Outputs are wires.
        let mut out_wires = Vec::new();
        for p in &m.outputs {
            let t = self.eval_type(&p.ty, sc);
            let w = match t {
                TypeV::Bits(w) => w,
                _ => {
                    self.err(p.ty.span(), "output ports must be bit vectors");
                    1
                }
            };
            let wi = self.cur().wires.len();
            self.cur().wires.push(Wire { name: p.name.name.clone(), width: w, assigns: vec![], value: None, in_progress: false, def: p.name.span, is_output: true });
            self.bind(sc, &p.name, Bind::Wire(wi));
            self.def(&p.name, DefKind::Output, format!("output {}: {}", p.name.name, t.show()), None, m.span);
            out_wires.push(wi);
        }
        self.collect(&m.body, sc, m.span);
        // Force outputs, registers, memories.
        let mut outs = Vec::new();
        for (k, p) in m.outputs.iter().enumerate() {
            let wi = out_wires[k];
            let id = self.force_wire(wi, p.name.span);
            if let Some(id) = id {
                if self.record_signals && self.stack.len() == 1 {
                    self.info.signals.push((p.name.span, p.name.name.clone(), id));
                }
            }
            outs.push((p.name.name.clone(), id.map(Val::Sig).unwrap_or(Val::Err)));
        }
        let nregs = self.cur().regs.len();
        for r in 0..nregs {
            self.force_reg(r);
        }
        let nmems = self.cur().mems.len();
        for mi in 0..nmems {
            self.force_mem(mi);
        }
        // Unused lets: force them so that their errors are reported.
        let nth = self.cur().thunks.len();
        for t in 0..nth {
            self.force_thunk(t);
        }
        let nw = self.cur().wires.len();
        for w in 0..nw {
            let (out, def) = {
                let x = &self.cur().wires[w];
                (x.is_output, x.def)
            };
            if !out {
                self.force_wire(w, def);
            }
        }
        self.stack.pop();
        self.depth -= 1;
        outs
    }

    fn collect(&mut self, stmts: &'a [Stmt], scope: usize, item_span: Span) {
        for st in stmts {
            match st {
                Stmt::Let { pat, ty, value, span } => match (pat, value) {
                    (LetPat::Name(id), Some(v)) => {
                        let ti = self.cur().thunks.len();
                        self.cur().thunks.push(Thunk { expr: v, ty: ty.as_ref(), scope, state: ThunkState::Pending, name: id.name.clone(), def: id.span });
                        self.bind(scope, id, Bind::Thunk(ti));
                        self.def(id, DefKind::Wire, format!("let {}: ?", id.name), None, item_span);
                    }
                    (LetPat::Name(id), None) => {
                        let Some(t) = ty else {
                            self.err(*span, "a `let` without a value needs a type");
                            continue;
                        };
                        let w = match self.eval_type(t, scope) {
                            TypeV::Bits(w) => w,
                            _ => 1,
                        };
                        let wi = self.cur().wires.len();
                        self.cur().wires.push(Wire { name: id.name.clone(), width: w, assigns: vec![], value: None, in_progress: false, def: id.span, is_output: false });
                        self.bind(scope, id, Bind::Wire(wi));
                        self.def(id, DefKind::Wire, format!("let {}: bits<{w}>", id.name), None, item_span);
                    }
                    (LetPat::Tuple(ids), Some(v)) => {
                        let ti = self.cur().thunks.len();
                        self.cur().thunks.push(Thunk { expr: v, ty: None, scope, state: ThunkState::Pending, name: "(tuple)".into(), def: *span });
                        for (k, id) in ids.iter().enumerate() {
                            self.bind(scope, id, Bind::TupleElem(ti, k));
                            self.def(id, DefKind::Wire, format!("let {}: ?", id.name), None, item_span);
                        }
                    }
                    (LetPat::Tuple(_), None) => self.err(*span, "tuple patterns need a value"),
                },
                Stmt::Reg { name, ty, init, span } => {
                    let w = match self.eval_type(ty, scope) {
                        TypeV::Bits(w) => w,
                        _ => {
                            self.err(ty.span(), "registers must be bit vectors (use `mem` for arrays)");
                            1
                        }
                    };
                    let init_v = match init {
                        Some(e) => match self.eval(e, scope, Some(w)) {
                            Val::Int(x) => x,
                            Val::Sig(id) => match self.rtl.const_val(id) {
                                Some(c) => c as i128,
                                None => {
                                    self.err(e.span, "register initial values must be constant");
                                    0
                                }
                            },
                            _ => 0,
                        },
                        None => 0,
                    };
                    let idx = self.rtl.regs.len();
                    let g = self.group();
                    let path = self.cur().path.clone();
                    let full = if path.is_empty() { name.name.clone() } else { format!("{path}.{}", name.name) };
                    let q = self.rtl.reg_q(idx as u32, w, g, name.span);
                    self.rtl.regs.push(crate::rtl::RReg { name: full, width: w, init: (init_v as u64) & mask(w), q, next: q, group: g, span: *span });
                    let ri = self.cur().regs.len();
                    self.cur().regs.push(RegE { idx, width: w, nexts: vec![], def: name.span });
                    self.bind(scope, name, Bind::Reg(ri));
                    self.def(name, DefKind::Reg, format!("reg {}: bits<{w}> = {init_v}", name.name), None, item_span);
                    if self.record_signals && self.stack.len() == 1 {
                        self.info.signals.push((name.span, name.name.clone(), q));
                    }
                }
                Stmt::Mem { name, elem, size, span } => {
                    let w = match self.eval_type(elem, scope) {
                        TypeV::Bits(w) => w,
                        _ => 1,
                    };
                    let n = self.const_int(size, scope).unwrap_or(1).clamp(1, 1024) as usize;
                    let g = self.group();
                    let mg = self.rtl.add_group(&format!("ram {}", name.name), "ram", g);
                    let mut regs = Vec::new();
                    let path = self.cur().path.clone();
                    for k in 0..n {
                        let idx = self.rtl.regs.len();
                        let full = if path.is_empty() { format!("{}[{k}]", name.name) } else { format!("{path}.{}[{k}]", name.name) };
                        let q = self.rtl.reg_q(idx as u32, w, mg, name.span);
                        self.rtl.regs.push(crate::rtl::RReg { name: full, width: w, init: 0, q, next: q, group: mg, span: *span });
                        regs.push(idx);
                    }
                    let mi = self.cur().mems.len();
                    self.cur().mems.push(MemE { regs, width: w, writes: vec![], group: mg, def: name.span });
                    self.bind(scope, name, Bind::Mem(mi));
                    self.def(name, DefKind::Mem, format!("mem {}: bits<{w}>[{n}]", name.name), None, item_span);
                }
                Stmt::Const(c) => {
                    let ti = self.cur().thunks.len();
                    self.cur().thunks.push(Thunk { expr: &c.value, ty: c.ty.as_ref(), scope, state: ThunkState::Pending, name: c.name.name.clone(), def: c.name.span });
                    self.bind(scope, &c.name, Bind::Thunk(ti));
                    self.def(&c.name, DefKind::Const, format!("const {}", c.name.name), c.doc.clone(), item_span);
                }
                Stmt::Assign { target, value, span } => {
                    let base = target.base();
                    match self.lookup(scope, &base.name) {
                        Some((Bind::Wire(wi), def)) => {
                            self.reference(base.span, def);
                            if matches!(target, LValue::Next(_)) {
                                self.err(base.span, format!("`{}` is not a register", base.name));
                                continue;
                            }
                            let width = self.cur().wires[wi].width;
                            let range = match target {
                                LValue::Index(_, idx) => self.const_int(idx, scope).map(|k| (k, k)),
                                LValue::Slice(_, hi, lo) => match (self.const_int(hi, scope), self.const_int(lo, scope)) {
                                    (Some(h), Some(l)) => Some((h, l)),
                                    _ => None,
                                },
                                _ => None,
                            };
                            let range = match (target, range) {
                                (LValue::Name(_), _) => None,
                                (_, Some((hi, lo))) => {
                                    if lo < 0 || hi < lo || hi >= width as i128 {
                                        self.err(*span, format!("bit range [{hi}:{lo}] out of bounds for bits<{width}>"));
                                        continue;
                                    }
                                    Some((hi as u32, lo as u32))
                                }
                                (_, None) => continue,
                            };
                            self.cur().wires[wi].assigns.push(AssignE { lv: target, expr: value, scope, span: *span, range, value: None, busy: false });
                        }
                        Some((Bind::Reg(ri), def)) => {
                            self.reference(base.span, def);
                            if !matches!(target, LValue::Next(_)) {
                                self.err(*span, format!("assign the next state of register `{}` with `{}.next = ...`", base.name, base.name));
                                continue;
                            }
                            self.cur().regs[ri].nexts.push((value, scope, *span));
                        }
                        Some((_, def)) => {
                            self.reference(base.span, def);
                            self.err(base.span, format!("cannot assign to `{}` (only outputs, declared wires and `reg.next` can be assigned)", base.name));
                        }
                        None => self.err(base.span, format!("unknown name `{}`", base.name)),
                    }
                }
                Stmt::Expr(e) => match &e.kind {
                    ExprKind::Method { recv, name, args } if name.name == "write" => {
                        if let ExprKind::Ident(id) = &recv.kind {
                            match self.lookup(scope, &id.name) {
                                Some((Bind::Mem(mi), def)) => {
                                    self.reference(id.span, def);
                                    self.cur().mems[mi].writes.push((args, scope, e.span));
                                }
                                _ => self.err(id.span, format!("`{}` is not a memory", id.name)),
                            }
                        }
                    }
                    _ => {
                        self.eval(e, scope, None);
                        self.warn(e.span, "expression statement has no effect");
                    }
                },
                Stmt::For { var, start, end, inclusive, body, span } => {
                    let (Some(a), Some(b)) = (self.const_int(start, scope), self.const_int(end, scope)) else { continue };
                    let b = if *inclusive { b + 1 } else { b };
                    if b - a > 4096 {
                        self.err(*span, "loop has too many iterations (max 4096)");
                        continue;
                    }
                    self.def(var, DefKind::LoopVar, format!("{}: uint", var.name), None, item_span);
                    for i in a..b {
                        let sc = self.new_scope(Some(scope));
                        let vi = self.cur().vals.len();
                        self.cur().vals.push(Val::Int(i));
                        self.cur().scopes[sc].names.insert(var.name.clone(), (Bind::Val(vi), var.span));
                        self.collect(body, sc, item_span);
                    }
                }
            }
        }
    }

    fn force_thunk(&mut self, ti: usize) -> Val {
        let (expr, ty, scope, def, name) = {
            let t = &mut self.cur().thunks[ti];
            match &t.state {
                ThunkState::Done(v) => return v.clone(),
                ThunkState::InProgress => {
                    let (def, name) = (t.def, t.name.clone());
                    self.err(def, format!("combinational loop through `{name}`"));
                    return Val::Err;
                }
                ThunkState::Pending => {}
            }
            t.state = ThunkState::InProgress;
            (t.expr, t.ty, t.scope, t.def, t.name.clone())
        };
        let tv = ty.map(|t| self.eval_type(t, scope));
        let expected = match &tv {
            Some(TypeV::Bits(w)) => Some(*w),
            _ => None,
        };
        let mut v = self.eval(expr, scope, expected);
        if let Some(w) = expected {
            if !matches!(v, Val::Err | Val::Inst(_) | Val::Tuple(_)) {
                v = Val::Sig(self.coerce(&v, w, expr.span));
            }
        }
        if let Some(TypeV::Int) = tv {
            if !matches!(v, Val::Int(_)) {
                self.err(expr.span, "expected a constant integer");
            }
        }
        let detail = match &v {
            Val::Sig(id) => {
                let w = self.width(*id);
                if ty.is_none() && self.recording() {
                    self.info.inlays.push((def.end, format!(": {}", TypeV::Bits(w).show())));
                }
                format!("let {name}: {}", TypeV::Bits(w).show())
            }
            Val::Int(x) => format!("const {name} = {x}"),
            Val::Inst(_) => format!("{name}: instance"),
            Val::Arr(a) => format!("let {name}: array[{}]", a.len()),
            _ => format!("let {name}"),
        };
        if name != "(tuple)" {
            self.set_detail(def, detail);
        }
        if let Val::Sig(id) = v {
            if self.record_signals && self.stack.len() == 1 && name != "(tuple)" {
                self.info.signals.push((def, name.clone(), id));
            }
        }
        if let Val::Inst(_) = v {
            if let Some(&i) = self.def_index.get(&(def.start, def.end)) {
                self.info.defs[i].kind = DefKind::Instance;
            }
        }
        self.cur().thunks[ti].state = ThunkState::Done(v.clone());
        v
    }

    /// Evaluate one assignment of a wire (memoised, loop-checked).
    fn force_assign(&mut self, wi: usize, k: usize) -> Option<RId> {
        let a = self.cur().wires[wi].assigns[k].clone();
        if let Some(v) = a.value {
            return Some(v);
        }
        if a.busy {
            let name = self.cur().wires[wi].name.clone();
            self.err(a.span, format!("combinational loop through `{name}`"));
            return None;
        }
        self.cur().wires[wi].assigns[k].busy = true;
        let width = self.cur().wires[wi].width;
        let w = match a.range {
            Some((hi, lo)) => hi - lo + 1,
            None => width,
        };
        let v = self.eval(a.expr, a.scope, Some(w));
        let id = self.coerce(&v, w, a.expr.span);
        let x = &mut self.cur().wires[wi].assigns[k];
        x.value = Some(id);
        x.busy = false;
        Some(id)
    }

    /// Read bits [hi:lo] of a partially assigned wire without forcing unrelated bits.
    fn wire_bits(&mut self, wi: usize, hi: u32, lo: u32, span: Span) -> Option<RId> {
        let n = self.cur().wires[wi].assigns.len();
        let mut parts: Vec<(u32, u32, RId)> = Vec::new(); // (lo, hi, source slice)
        for k in 0..n {
            let r = self.cur().wires[wi].assigns[k].range;
            match r {
                None => {
                    let id = self.force_assign(wi, k)?;
                    return Some(self.node(ROp::Slice(id, lo), hi - lo + 1, span));
                }
                Some((ah, al)) => {
                    if ah < lo || al > hi {
                        continue;
                    }
                    let id = self.force_assign(wi, k)?;
                    let (ol, oh) = (al.max(lo), ah.min(hi));
                    let piece = self.node(ROp::Slice(id, ol - al), oh - ol + 1, span);
                    parts.push((ol, oh, piece));
                }
            }
        }
        parts.sort_by_key(|p| std::cmp::Reverse(p.0));
        let covered: u32 = parts.iter().map(|p| p.1 - p.0 + 1).sum();
        if covered != hi - lo + 1 {
            let name = self.cur().wires[wi].name.clone();
            self.err(span, format!("reading bits of `{name}` that are never assigned"));
            return None;
        }
        let ids: Vec<RId> = parts.iter().map(|p| p.2).collect();
        Some(self.node(ROp::Concat(ids), hi - lo + 1, span))
    }

    fn force_wire(&mut self, wi: usize, use_span: Span) -> Option<RId> {
        {
            let w = &self.cur().wires[wi];
            if let Some(v) = w.value {
                return Some(v);
            }
            if w.in_progress {
                let name = w.name.clone();
                self.err(use_span, format!("combinational loop through `{name}`"));
                return None;
            }
        }
        self.cur().wires[wi].in_progress = true;
        let (width, assigns, def, name, is_out) = {
            let w = &self.cur().wires[wi];
            (w.width, w.assigns.clone(), w.def, w.name.clone(), w.is_output)
        };
        let fulls: Vec<usize> = (0..assigns.len()).filter(|&k| assigns[k].range.is_none()).collect();
        let value = if !fulls.is_empty() {
            for &k in fulls.iter().skip(1) {
                self.err(assigns[k].span, format!("`{name}` is assigned more than once"));
            }
            for (k, a) in assigns.iter().enumerate() {
                if a.range.is_some() {
                    self.err(a.span, format!("`{name}` is assigned more than once"));
                }
                let _ = k;
            }
            self.force_assign(wi, fulls[0]).unwrap_or_else(|| self.rtl.konst(0, width))
        } else if assigns.is_empty() {
            if is_out {
                self.err(def, format!("output `{name}` is never assigned"));
            } else {
                self.err(def, format!("`{name}` is declared but never assigned"));
            }
            self.konst(0, width)
        } else {
            let mut owner: Vec<Option<usize>> = vec![None; width as usize];
            for (k, a) in assigns.iter().enumerate() {
                let (hi, lo) = a.range.unwrap();
                for b in lo..=hi {
                    if owner[b as usize].is_some() {
                        self.err(a.span, format!("bit {b} of `{name}` is assigned more than once"));
                    }
                    owner[b as usize] = Some(k);
                }
            }
            let missing: Vec<usize> = owner.iter().enumerate().filter(|(_, o)| o.is_none()).map(|(i, _)| i).collect();
            if !missing.is_empty() {
                self.err(def, format!("bits {missing:?} of `{name}` are never assigned"));
                self.konst(0, width)
            } else {
                self.wire_bits(wi, width - 1, 0, def).unwrap_or_else(|| self.rtl.konst(0, width))
            }
        };
        let w = &mut self.cur().wires[wi];
        w.value = Some(value);
        w.in_progress = false;
        Some(value)
    }

    fn force_reg(&mut self, ri: usize) {
        let (idx, width, nexts, def) = {
            let r = &self.cur().regs[ri];
            (r.idx, r.width, r.nexts.clone(), r.def)
        };
        if nexts.is_empty() {
            self.warn(def, "register is never updated (it keeps its initial value)");
            return;
        }
        if nexts.len() > 1 {
            self.err(nexts[1].2, "register next state is assigned more than once");
        }
        let (e, scope, _) = nexts[0];
        let v = self.eval(e, scope, Some(width));
        let id = self.coerce(&v, width, e.span);
        self.rtl.regs[idx].next = id;
    }

    fn force_mem(&mut self, mi: usize) {
        let (regs, width, writes, g) = {
            let m = &self.cur().mems[mi];
            (m.regs.clone(), m.width, m.writes.clone(), m.group)
        };
        let n = regs.len();
        let aw = clog2(n as i128).max(1) as u32;
        let mut nexts: Vec<RId> = regs.iter().map(|&r| self.rtl.regs[r].q).collect();
        for (args, scope, span) in writes {
            if args.len() != 3 {
                self.err(span, "write(addr, data, enable) takes three arguments");
                continue;
            }
            let av = self.eval(&args[0].value, scope, Some(aw));
            let Some(addr) = self.sig(&av, Some(aw), args[0].value.span) else { continue };
            let dv = self.eval(&args[1].value, scope, Some(width));
            let data = self.coerce(&dv, width, args[1].value.span);
            let ev = self.eval(&args[2].value, scope, Some(1));
            let en = self.coerce(&ev, 1, args[2].value.span);
            let awid = self.width(addr);
            for (k, nx) in nexts.iter_mut().enumerate() {
                let kc = self.rtl.konst(k as u64, awid);
                let hit = self.node_g(ROp::Eq(addr, kc), 1, g, span);
                let sel = self.node_g(ROp::And(hit, en), 1, g, span);
                *nx = self.node_g(ROp::Mux(sel, data, *nx), width, g, span);
            }
        }
        for (k, &r) in regs.iter().enumerate() {
            self.rtl.regs[r].next = nexts[k];
        }
    }

    // ---- expressions ----

    fn eval_block(&mut self, b: &'a Block, scope: usize, expected: Option<u32>) -> Val {
        let sc = self.new_scope(Some(scope));
        for st in &b.stmts {
            if let Stmt::Assign { span, .. } = st {
                self.err(*span, "assignments are not allowed inside expression blocks");
            }
        }
        let item_span = b.span;
        let stmts: Vec<&'a Stmt> = b.stmts.iter().filter(|s| !matches!(s, Stmt::Assign { .. })).collect();
        for st in stmts {
            self.collect(std::slice::from_ref(st), sc, item_span);
        }
        match &b.tail {
            Some(t) => self.eval(t, sc, expected),
            None => {
                self.err(b.span, "block has no value (the last line must be an expression)");
                Val::Err
            }
        }
    }

    fn resolve_ident(&mut self, id: &Ident, scope: usize, expected: Option<u32>) -> Val {
        match self.lookup(scope, &id.name) {
            Some((b, def)) => {
                self.reference(id.span, def);
                match b {
                    Bind::Thunk(t) => self.force_thunk(t),
                    Bind::TupleElem(t, k) => match self.force_thunk(t) {
                        Val::Tuple(v) => v.get(k).cloned().unwrap_or_else(|| {
                            self.err(id.span, "tuple has fewer elements");
                            Val::Err
                        }),
                        Val::Inst(outs) => outs.get(k).map(|o| o.1.clone()).unwrap_or_else(|| {
                            self.err(id.span, "instance has fewer outputs");
                            Val::Err
                        }),
                        Val::Err => Val::Err,
                        _ => {
                            self.err(id.span, "cannot destructure a non-tuple value");
                            Val::Err
                        }
                    },
                    Bind::Wire(w) => self.force_wire(w, id.span).map(Val::Sig).unwrap_or(Val::Err),
                    Bind::Reg(r) => {
                        let idx = self.cur().regs[r].idx;
                        Val::Sig(self.rtl.regs[idx].q)
                    }
                    Bind::Mem(m) => Val::Mem(m),
                    Bind::Val(v) => self.cur().vals[v].clone(),
                }
            }
            None => self.global(id, expected),
        }
    }

    fn global(&mut self, id: &Ident, _expected: Option<u32>) -> Val {
        match self.items.get(&id.name).copied() {
            Some(Item::Const(c)) => {
                self.reference(id.span, c.name.span);
                if let Some(v) = self.global_consts.get(&id.name) {
                    return v.clone();
                }
                if self.global_busy.contains(&id.name) {
                    self.err(id.span, format!("constant `{}` depends on itself", id.name));
                    return Val::Err;
                }
                self.global_busy.push(id.name.clone());
                // Evaluate in a fresh instance context (constants are context-free).
                self.stack.push(Inst { scopes: vec![], thunks: vec![], wires: vec![], regs: vec![], mems: vec![], vals: vec![], group: 0, path: String::new(), record: true });
                let sc = self.new_scope(None);
                let tv = c.ty.as_ref().map(|t| self.eval_type(t, sc));
                let exp = match &tv {
                    Some(TypeV::Bits(w)) => Some(*w),
                    _ => None,
                };
                let mut v = self.eval(&c.value, sc, exp);
                match (&tv, &v) {
                    (Some(TypeV::Bits(w)), Val::Int(_)) => v = Val::Sig(self.coerce(&v, *w, c.value.span)),
                    (Some(TypeV::Arr(et, n)), Val::Arr(items)) => {
                        if items.len() != *n as usize {
                            self.err(c.value.span, format!("expected {n} elements, found {}", items.len()));
                        }
                        if let TypeV::Bits(w) = **et {
                            let items2: Vec<Val> = items.clone().iter().map(|x| Val::Sig(self.coerce(x, w, c.value.span))).collect();
                            v = Val::Arr(items2);
                        }
                    }
                    _ => {}
                }
                self.stack.pop();
                self.global_busy.pop();
                self.global_consts.insert(id.name.clone(), v.clone());
                v
            }
            Some(Item::Module(m)) => {
                self.reference(id.span, m.name.span);
                self.err(id.span, format!("module `{}` must be instantiated with arguments: `{}(...)`", id.name, id.name));
                Val::Err
            }
            _ => {
                self.err(id.span, format!("unknown name `{}`", id.name));
                Val::Err
            }
        }
    }

    pub(crate) fn eval(&mut self, e: &'a Expr, scope: usize, expected: Option<u32>) -> Val {
        let span = e.span;
        match &e.kind {
            ExprKind::Int(v) => Val::Int(*v as i128),
            ExprKind::Bool(b) => Val::Int(*b as i128),
            ExprKind::Error => Val::Err,
            ExprKind::Paren(x) => self.eval(x, scope, expected),
            ExprKind::Ident(id) => self.resolve_ident(id, scope, expected),
            ExprKind::Path(en, v) => match self.items.get(&en.name).copied() {
                Some(Item::Enum(ed)) => {
                    self.reference(en.span, ed.name.span);
                    if let Some((vi, _)) = ed.variants.iter().find(|(x, _)| x.name == v.name) {
                        self.reference(v.span, vi.span);
                    }
                    match self.enum_value(ed, &v.name) {
                        Some(x) => {
                            let w = self.enum_width(ed);
                            Val::Sig(self.konst(x, w))
                        }
                        None => {
                            self.err(v.span, format!("enum `{}` has no variant `{}`", en.name, v.name));
                            Val::Err
                        }
                    }
                }
                _ => {
                    self.err(en.span, format!("unknown enum `{}`", en.name));
                    Val::Err
                }
            },
            ExprKind::Unary(op, x) => {
                let v = self.eval(x, scope, if *op == UnOp::LNot { Some(1) } else { expected });
                match (op, &v) {
                    (_, Val::Err) => Val::Err,
                    (UnOp::Neg, Val::Int(a)) => Val::Int(-a),
                    (UnOp::LNot, Val::Int(a)) => Val::Int((*a == 0) as i128),
                    (UnOp::Not, Val::Int(a)) => match expected {
                        Some(w) => Val::Sig(self.konst(!*a, w)),
                        None => {
                            self.err(span, "cannot infer the width of `~` applied to a literal; add a type annotation");
                            Val::Err
                        }
                    },
                    (UnOp::Not, Val::Sig(id)) => {
                        let w = self.width(*id);
                        Val::Sig(self.node(ROp::Not(*id), w, span))
                    }
                    (UnOp::LNot, Val::Sig(id)) => {
                        let id = self.coerce(&Val::Sig(*id), 1, x.span);
                        Val::Sig(self.node(ROp::Not(id), 1, span))
                    }
                    (UnOp::Neg, Val::Sig(id)) => {
                        let w = self.width(*id);
                        let z = self.konst(0, w);
                        Val::Sig(self.node(ROp::Sub(z, *id), w, span))
                    }
                    _ => {
                        self.err(span, "invalid operand");
                        Val::Err
                    }
                }
            }
            ExprKind::Binary(op, l, r) => self.binary(*op, l, r, scope, expected, span),
            ExprKind::Index(b, i) => {
                if let Some(v) = self.partial_read(b, i, i, scope, span) {
                    return v;
                }
                let bv = self.eval(b, scope, None);
                self.index(bv, i, scope, span)
            }
            ExprKind::Slice(b, hi, lo) => {
                if let Some(v) = self.partial_read(b, hi, lo, scope, span) {
                    return v;
                }
                let bv = self.eval(b, scope, None);
                let (Some(hi), Some(lo)) = (self.const_int(hi, scope), self.const_int(lo, scope)) else { return Val::Err };
                let Some(id) = self.sig(&bv, None, b.span) else { return Val::Err };
                let w = self.width(id) as i128;
                if lo < 0 || hi < lo || hi >= w {
                    self.err(span, format!("slice [{hi}:{lo}] out of range for bits<{w}>"));
                    return Val::Err;
                }
                Val::Sig(self.node(ROp::Slice(id, lo as u32), (hi - lo + 1) as u32, span))
            }
            ExprKind::Field(b, f) => {
                let bv = self.eval(b, scope, None);
                match bv {
                    Val::Inst(outs) => match outs.iter().find(|(n, _)| *n == f.name) {
                        Some((_, v)) => v.clone(),
                        None => {
                            let names: Vec<&str> = outs.iter().map(|o| o.0.as_str()).collect();
                            self.err(f.span, format!("instance has no output `{}` (outputs: {})", f.name, names.join(", ")));
                            Val::Err
                        }
                    },
                    Val::Err => Val::Err,
                    _ => {
                        if f.name == "next" {
                            self.err(f.span, "`.next` can only be assigned, not read");
                        } else {
                            self.err(f.span, format!("no field `{}` on this value", f.name));
                        }
                        Val::Err
                    }
                }
            }
            ExprKind::Method { recv, name, args } => {
                let rv = self.eval(recv, scope, None);
                match (&rv, name.name.as_str()) {
                    (Val::Mem(m), "read") => {
                        if args.len() != 1 {
                            self.err(span, "read(addr) takes one argument");
                            return Val::Err;
                        }
                        self.mem_read(*m, &args[0].value, scope, span)
                    }
                    (Val::Mem(_), "write") => {
                        self.err(span, "`write` is a statement");
                        Val::Err
                    }
                    (Val::Err, _) => Val::Err,
                    _ => {
                        self.err(name.span, format!("unknown method `{}`", name.name));
                        Val::Err
                    }
                }
            }
            ExprKind::Call { callee, generics, args } => self.call(callee, generics, args, scope, expected, span),
            ExprKind::If(c, t, els) => {
                let cv = self.eval(c, scope, Some(1));
                if let Val::Int(x) = cv {
                    // Constant condition: elaborate only the taken branch.
                    return if x != 0 {
                        self.eval_block(t, scope, expected)
                    } else if let Some(e2) = els {
                        self.eval(e2, scope, expected)
                    } else {
                        self.err(span, "`if` without `else` has no value");
                        Val::Err
                    };
                }
                let Some(cid) = self.sig(&cv, Some(1), c.span) else { return Val::Err };
                let cid = self.coerce(&Val::Sig(cid), 1, c.span);
                let tv = self.eval_block(t, scope, expected);
                let Some(e2) = els else {
                    self.err(span, "`if` without `else` has no value");
                    return Val::Err;
                };
                let ev = self.eval(e2, scope, expected);
                let w = self.unify_width(&tv, &ev, expected, span);
                let Some(w) = w else { return Val::Err };
                let a = self.coerce(&tv, w, t.span);
                let b = self.coerce(&ev, w, e2.span);
                Val::Sig(self.node(ROp::Mux(cid, a, b), w, span))
            }
            ExprKind::Match(s, arms) => self.matchx(s, arms, scope, expected, span),
            ExprKind::Block(b) => self.eval_block(b, scope, expected),
            ExprKind::Array(items) => {
                let vs: Vec<Val> = items.iter().map(|x| self.eval(x, scope, expected)).collect();
                Val::Arr(vs)
            }
            ExprKind::Repeat(x, n) => {
                let v = self.eval(x, scope, expected);
                let n = self.const_int(n, scope).unwrap_or(0).clamp(0, 4096);
                Val::Arr(vec![v; n as usize])
            }
            ExprKind::Tuple(items) => Val::Tuple(items.iter().map(|x| self.eval(x, scope, None)).collect()),
        }
    }

    /// `x[hi:lo]` where `x` is a wire with partial assignments: read only those bits.
    fn partial_read(&mut self, b: &'a Expr, hi: &'a Expr, lo: &'a Expr, scope: usize, span: Span) -> Option<Val> {
        let ExprKind::Ident(id) = &b.kind else { return None };
        let (Bind::Wire(wi), def) = self.lookup(scope, &id.name)? else { return None };
        if self.cur().wires[wi].value.is_some() || self.cur().wires[wi].assigns.iter().any(|a| a.range.is_none()) {
            return None;
        }
        // Indices must be constant for the lazy path.
        let h = match &hi.kind {
            ExprKind::Int(_) | ExprKind::Ident(_) | ExprKind::Binary(..) | ExprKind::Paren(_) => {
                let saved = self.diags.len();
                let v = self.eval(hi, scope, None);
                self.diags.truncate(saved);
                match v {
                    Val::Int(x) => x,
                    _ => return None,
                }
            }
            _ => return None,
        };
        let l = if std::ptr::eq(hi, lo) {
            h
        } else {
            match self.eval(lo, scope, None) {
                Val::Int(x) => x,
                _ => return None,
            }
        };
        self.reference(id.span, def);
        let width = self.cur().wires[wi].width as i128;
        if l < 0 || h < l || h >= width {
            self.err(span, format!("bit range [{h}:{l}] out of bounds for bits<{width}>"));
            return Some(Val::Err);
        }
        Some(self.wire_bits(wi, h as u32, l as u32, span).map(Val::Sig).unwrap_or(Val::Err))
    }

    fn unify_width(&mut self, a: &Val, b: &Val, expected: Option<u32>, span: Span) -> Option<u32> {
        let wa = match a {
            Val::Sig(id) => Some(self.width(*id)),
            _ => None,
        };
        let wb = match b {
            Val::Sig(id) => Some(self.width(*id)),
            _ => None,
        };
        match (wa, wb) {
            (Some(x), Some(y)) if x != y => {
                self.err(span, format!("branches have different widths: bits<{x}> and bits<{y}>"));
                Some(x.max(y))
            }
            (Some(x), _) | (_, Some(x)) => Some(x),
            (None, None) => {
                if matches!(a, Val::Err) || matches!(b, Val::Err) {
                    return None;
                }
                match expected {
                    Some(w) => Some(w),
                    None => {
                        let m = |v: &Val| if let Val::Int(x) = v { (128 - (*x).max(0).leading_zeros()).max(1) } else { 1 };
                        Some(m(a).max(m(b)))
                    }
                }
            }
        }
    }

    fn binary(&mut self, op: BinOp, l: &'a Expr, r: &'a Expr, scope: usize, expected: Option<u32>, span: Span) -> Val {
        use BinOp::*;
        let operand_exp = match op {
            LAnd | LOr => Some(1),
            Eq | Ne | Lt | Le | Gt | Ge | Concat | Shl | Shr | Sar => None,
            _ => expected,
        };
        let lv = self.eval(l, scope, operand_exp);
        let rexp = match (&lv, op) {
            (Val::Sig(id), Eq | Ne | Lt | Le | Gt | Ge | Add | Sub | Mul | And | Or | Xor) => Some(self.width(*id)),
            _ => operand_exp,
        };
        let rv = self.eval(r, scope, rexp);
        if matches!(lv, Val::Err) || matches!(rv, Val::Err) {
            return Val::Err;
        }
        // Constant folding on unsized integers.
        if let (Val::Int(a), Val::Int(b)) = (&lv, &rv) {
            let (a, b) = (*a, *b);
            let v = match op {
                Add => a + b,
                Sub => a - b,
                Mul => a.wrapping_mul(b),
                Div => {
                    if b == 0 {
                        self.err(span, "division by zero");
                        0
                    } else {
                        a / b
                    }
                }
                Rem => {
                    if b == 0 {
                        self.err(span, "division by zero");
                        0
                    } else {
                        a % b
                    }
                }
                Shl => a << b.clamp(0, 120),
                Shr | Sar => a >> b.clamp(0, 120),
                And => a & b,
                Or => a | b,
                Xor => a ^ b,
                Eq => (a == b) as i128,
                Ne => (a != b) as i128,
                Lt => (a < b) as i128,
                Le => (a <= b) as i128,
                Gt => (a > b) as i128,
                Ge => (a >= b) as i128,
                LAnd => (a != 0 && b != 0) as i128,
                LOr => (a != 0 || b != 0) as i128,
                Concat => {
                    self.err(span, "cannot concatenate unsized literals; give them a width with zext(x, N)");
                    return Val::Err;
                }
            };
            return Val::Int(v);
        }
        let g = self.group();
        let mk_group = |s: &mut Self, kind: &str, label: &str| -> u32 {
            let line = s.line_of(span);
            s.rtl.add_group(&format!("{label} (line {line})"), kind, g)
        };
        match op {
            Add | Sub | Mul | And | Or | Xor => {
                let Some(w) = self.unify_width(&lv, &rv, expected, span) else { return Val::Err };
                let a = self.coerce(&lv, w, l.span);
                let b = self.coerce(&rv, w, r.span);
                let (o, grp) = match op {
                    Add => (ROp::Add(a, b), Some(mk_group(self, "adder", "adder"))),
                    Sub => (ROp::Sub(a, b), Some(mk_group(self, "adder", "subtractor"))),
                    Mul => (ROp::Mul(a, b), Some(mk_group(self, "multiplier", "multiplier"))),
                    And => (ROp::And(a, b), None),
                    Or => (ROp::Or(a, b), None),
                    _ => (ROp::Xor(a, b), None),
                };
                let grp = grp.unwrap_or(g);
                Val::Sig(self.node_g(o, w, grp, span))
            }
            Div | Rem => {
                self.err(span, "division is only supported between constants");
                Val::Err
            }
            Shl | Shr | Sar => {
                let Some(a) = self.sig(&lv, expected, l.span) else { return Val::Err };
                let w = self.width(a);
                let (b, grp) = match &rv {
                    Val::Int(x) => {
                        if *x < 0 {
                            self.err(r.span, "negative shift amount");
                        }
                        (self.konst((*x).max(0), 8), g)
                    }
                    _ => {
                        let Some(b) = self.sig(&rv, None, r.span) else { return Val::Err };
                        (b, mk_group(self, "shifter", "shifter"))
                    }
                };
                let o = match op {
                    Shl => ROp::Shl(a, b),
                    Shr => ROp::Shr(a, b),
                    _ => ROp::Sar(a, b),
                };
                Val::Sig(self.node_g(o, w, grp, span))
            }
            Concat => {
                let a = self.sig(&lv, None, l.span);
                let b = self.sig(&rv, None, r.span);
                if matches!(lv, Val::Int(_)) || matches!(rv, Val::Int(_)) {
                    self.err(span, "concatenation needs sized operands (use zext(x, N) for literals)");
                }
                let (Some(a), Some(b)) = (a, b) else { return Val::Err };
                let w = self.width(a) + self.width(b);
                if w > 64 {
                    self.err(span, "result wider than 64 bits");
                    return Val::Err;
                }
                Val::Sig(self.node(ROp::Concat(vec![a, b]), w, span))
            }
            Eq | Ne | Lt | Le | Gt | Ge => {
                let Some(w) = self.unify_width(&lv, &rv, None, span) else { return Val::Err };
                let a = self.coerce(&lv, w, l.span);
                let b = self.coerce(&rv, w, r.span);
                let grp = if w > 1 { mk_group(self, "comparator", "comparator") } else { g };
                let x = match op {
                    Eq => self.node_g(ROp::Eq(a, b), 1, grp, span),
                    Ne => {
                        let e = self.node_g(ROp::Eq(a, b), 1, grp, span);
                        self.node_g(ROp::Not(e), 1, grp, span)
                    }
                    Lt => self.node_g(ROp::Ult(a, b), 1, grp, span),
                    Gt => self.node_g(ROp::Ult(b, a), 1, grp, span),
                    Le => {
                        let x = self.node_g(ROp::Ult(b, a), 1, grp, span);
                        self.node_g(ROp::Not(x), 1, grp, span)
                    }
                    _ => {
                        let x = self.node_g(ROp::Ult(a, b), 1, grp, span);
                        self.node_g(ROp::Not(x), 1, grp, span)
                    }
                };
                Val::Sig(x)
            }
            LAnd | LOr => {
                let a = self.coerce(&lv, 1, l.span);
                let b = self.coerce(&rv, 1, r.span);
                let o = if op == LAnd { ROp::And(a, b) } else { ROp::Or(a, b) };
                Val::Sig(self.node(o, 1, span))
            }
        }
    }

    fn line_of(&self, span: Span) -> usize {
        // computed lazily from the source of the file? We only have spans; store offsets.
        span.start as usize
    }

    fn index(&mut self, bv: Val, i: &'a Expr, scope: usize, span: Span) -> Val {
        match bv {
            Val::Arr(items) => {
                let iv = self.eval(i, scope, None);
                match iv {
                    Val::Int(k) => items.get(k as usize).cloned().unwrap_or_else(|| {
                        self.err(span, format!("index {k} out of bounds (array of {})", items.len()));
                        Val::Err
                    }),
                    Val::Sig(sel) => {
                        // ROM / array read: mux tree over elements.
                        let g = self.group();
                        let line = self.line_of(span);
                        let rg = self.rtl.add_group(&format!("rom (line {line})"), "rom", g);
                        let w = items.iter().find_map(|v| if let Val::Sig(id) = v { Some(self.width(*id)) } else { None }).unwrap_or(1);
                        let elems: Vec<RId> = items.iter().map(|v| self.coerce(v, w, span)).collect();
                        Val::Sig(self.mux_tree(sel, &elems, w, rg, span))
                    }
                    _ => Val::Err,
                }
            }
            Val::Mem(m) => self.mem_read(m, i, scope, span),
            Val::Err => Val::Err,
            other => {
                let Some(id) = self.sig(&other, None, span) else { return Val::Err };
                let w = self.width(id);
                let iv = self.eval(i, scope, None);
                match iv {
                    Val::Int(k) => {
                        if k < 0 || k >= w as i128 {
                            self.err(i.span, format!("bit index {k} out of range for bits<{w}>"));
                            return Val::Err;
                        }
                        Val::Sig(self.node(ROp::Slice(id, k as u32), 1, span))
                    }
                    Val::Sig(sel) => {
                        let sh = self.node(ROp::Shr(id, sel), w, span);
                        Val::Sig(self.node(ROp::Slice(sh, 0), 1, span))
                    }
                    _ => Val::Err,
                }
            }
        }
    }

    fn mux_tree(&mut self, sel: RId, elems: &[RId], w: u32, g: u32, span: Span) -> RId {
        // Binary tree on select bits (lsb at the leaves).
        let sw = self.width(sel);
        let mut level: Vec<RId> = elems.to_vec();
        let mut bit = 0u32;
        while level.len() > 1 {
            let s = if bit < sw { self.node_g(ROp::Slice(sel, bit), 1, g, span) } else { self.rtl.konst(0, 1) };
            let mut next = Vec::new();
            for pair in level.chunks(2) {
                if pair.len() == 2 {
                    next.push(self.node_g(ROp::Mux(s, pair[1], pair[0]), w, g, span));
                } else {
                    next.push(pair[0]);
                }
            }
            level = next;
            bit += 1;
        }
        // Out-of-range addresses read element 0's neighbour pattern; fine for power-of-two sizes.
        level.first().copied().unwrap_or_else(|| self.rtl.konst(0, w))
    }

    fn mem_read(&mut self, m: usize, addr: &'a Expr, scope: usize, span: Span) -> Val {
        let (regs, w, g) = {
            let me = &self.cur().mems[m];
            (me.regs.clone(), me.width, me.group)
        };
        let aw = clog2(regs.len() as i128).max(1) as u32;
        let av = self.eval(addr, scope, Some(aw));
        let Some(a) = self.sig(&av, Some(aw), addr.span) else { return Val::Err };
        let qs: Vec<RId> = regs.iter().map(|&r| self.rtl.regs[r].q).collect();
        Val::Sig(self.mux_tree(a, &qs, w, g, span))
    }

    fn matchx(&mut self, s: &'a Expr, arms: &'a [Arm], scope: usize, expected: Option<u32>, span: Span) -> Val {
        let sv = self.eval(s, scope, None);
        let Some(sid) = self.sig(&sv, None, s.span) else { return Val::Err };
        let sw = self.width(sid);
        let g = self.group();
        let line = self.line_of(span);
        let mg = self.rtl.add_group(&format!("mux (line {line})"), "mux", g);
        // Evaluate arm values first to find the result width.
        let vals: Vec<Val> = arms.iter().map(|a| self.eval(&a.value, scope, expected)).collect();
        let mut w = expected;
        for v in &vals {
            if let Val::Sig(id) = v {
                let ww = self.width(*id);
                if let Some(x) = w {
                    if x != ww && expected.is_none() {
                        self.err(span, format!("match arms have different widths: bits<{x}> and bits<{ww}>"));
                    }
                } else {
                    w = Some(ww);
                }
            }
        }
        let w = w.unwrap_or_else(|| vals.iter().map(|v| if let Val::Int(x) = v { (128 - (*x).max(0).leading_zeros()).max(1) } else { 1 }).max().unwrap_or(1));
        // Patterns → value sets (for disjointness) and conditions.
        let mut default: Option<(usize, Span)> = None;
        let mut covered: Vec<(u128, u128, Span)> = Vec::new();
        let mut result: Option<RId> = None;
        let mut conds: Vec<(RId, RId)> = Vec::new();
        for (k, arm) in arms.iter().enumerate() {
            let v = self.coerce(&vals[k], w, arm.value.span);
            let mut ranges = Vec::new();
            if !self.pat_ranges(&arm.pat, sw, &mut ranges) {
                if let Some((_, prev)) = default {
                    self.diags.push(Diag::error(arm.pat.span(), "more than one `_` arm").with_note(prev, "first `_` arm"));
                }
                default = Some((k, arm.pat.span()));
                result = Some(v);
                continue;
            }
            let mut cond: Option<RId> = None;
            for &(lo, hi, ps) in &ranges {
                for &(a, b, s2) in &covered {
                    if lo <= b && a <= hi {
                        self.diags.push(Diag::error(ps, "overlapping match arms (arms must be disjoint; `_` is the default)").with_note(s2, "overlaps with this pattern"));
                    }
                }
                covered.push((lo, hi, ps));
                let c = if lo == hi {
                    let k = self.rtl.konst(lo as u64, sw);
                    self.node_g(ROp::Eq(sid, k), 1, mg, ps)
                } else {
                    let kl = self.rtl.konst(lo as u64, sw);
                    let kh = self.rtl.konst(hi as u64, sw);
                    let below = self.node_g(ROp::Ult(sid, kl), 1, mg, ps);
                    let above = self.node_g(ROp::Ult(kh, sid), 1, mg, ps);
                    let out = self.node_g(ROp::Or(below, above), 1, mg, ps);
                    self.node_g(ROp::Not(out), 1, mg, ps)
                };
                cond = Some(match cond {
                    None => c,
                    Some(p) => self.node_g(ROp::Or(p, c), 1, mg, ps),
                });
            }
            if let Some(c) = cond {
                conds.push((c, v));
            }
        }
        // Exhaustiveness.
        if default.is_none() {
            let total: u128 = if sw >= 64 { u128::MAX } else { 1u128 << sw };
            let mut cv: Vec<(u128, u128)> = covered.iter().map(|&(a, b, _)| (a, b)).collect();
            cv.sort();
            let mut next = 0u128;
            for (a, b) in cv {
                if a > next {
                    break;
                }
                next = next.max(b + 1);
            }
            if next < total {
                self.err(span, format!("non-exhaustive match: value {next} is not covered (add a `_ => ...` arm)"));
            }
        } else if let Some((_, ds)) = default {
            let total: u128 = if sw >= 64 { u128::MAX } else { 1u128 << sw };
            let n: u128 = covered.iter().map(|&(a, b, _)| b - a + 1).sum();
            if n >= total {
                self.warn(ds, "unreachable `_` arm: all values are already covered");
            }
        }
        let mut acc = match result {
            Some(r) => r,
            None => conds.last().map(|c| c.1).unwrap_or_else(|| self.rtl.konst(0, w)),
        };
        // Arms are disjoint, so the order of the mux chain does not matter.
        for &(c, v) in conds.iter().rev() {
            acc = self.node_g(ROp::Mux(c, v, acc), w, mg, span);
        }
        Val::Sig(acc)
    }

    /// Pattern → list of value ranges; returns false for `_`.
    fn pat_ranges(&mut self, p: &'a Pat, sw: u32, out: &mut Vec<(u128, u128, Span)>) -> bool {
        let max: u128 = if sw >= 64 { u64::MAX as u128 } else { (1u128 << sw) - 1 };
        match p {
            Pat::Wild(_) => false,
            Pat::Int(v, s) => {
                if *v > max {
                    self.err(*s, format!("pattern {v} does not fit in bits<{sw}>"));
                }
                out.push((*v, *v, *s));
                true
            }
            Pat::Range(a, b, s) => {
                if a > b || *b > max {
                    self.err(*s, "invalid range pattern");
                }
                out.push((*a, (*b).min(max), *s));
                true
            }
            Pat::Path(en, v) => {
                if let Some(Item::Enum(ed)) = self.items.get(&en.name).copied() {
                    self.reference(en.span, ed.name.span);
                    if let Some((vi, _)) = ed.variants.iter().find(|(x, _)| x.name == v.name) {
                        self.reference(v.span, vi.span);
                    }
                    match self.enum_value(ed, &v.name) {
                        Some(x) => out.push((x as u128, x as u128, p.span())),
                        None => self.err(v.span, format!("enum `{}` has no variant `{}`", en.name, v.name)),
                    }
                } else {
                    self.err(en.span, format!("unknown enum `{}`", en.name));
                }
                true
            }
            Pat::Const(id) => {
                let v = self.global(id, Some(sw));
                match v {
                    Val::Int(x) => out.push((x as u128, x as u128, id.span)),
                    Val::Sig(sid) => match self.rtl.const_val(sid) {
                        Some(x) => out.push((x as u128, x as u128, id.span)),
                        None => self.err(id.span, "patterns must be constants"),
                    },
                    _ => {}
                }
                true
            }
            Pat::Or(ps, _) => {
                for q in ps {
                    if !self.pat_ranges(q, sw, out) {
                        self.err(q.span(), "`_` cannot be part of an or-pattern");
                    }
                }
                true
            }
        }
    }

    fn arg_values(&mut self, args: &'a [Arg], scope: usize, expected: &[Option<u32>]) -> Vec<Val> {
        args.iter().enumerate().map(|(k, a)| self.eval(&a.value, scope, expected.get(k).copied().flatten())).collect()
    }

    fn call(&mut self, callee: &'a Ident, generics: &'a [Expr], args: &'a [Arg], scope: usize, expected: Option<u32>, span: Span) -> Val {
        let name = callee.name.as_str();
        // Built-in functions.
        let builtin = matches!(name, "zext" | "sext" | "trunc" | "cat" | "rep" | "clog2" | "any" | "all" | "parity" | "mux" | "slt" | "sle" | "sgt" | "sge" | "width");
        if builtin && !self.items.contains_key(name) {
            return self.builtin(name, args, scope, expected, span);
        }
        match self.items.get(name).copied() {
            Some(Item::Fn(f)) => {
                self.reference(callee.span, f.name.span);
                if args.len() != f.params.len() {
                    self.err(span, format!("`{}` takes {} arguments, found {}", f.name.name, f.params.len(), args.len()));
                    return Val::Err;
                }
                // New instance-like scope that shares the current instance (inline).
                let sc = self.new_scope(None);
                let gv: Vec<i128> = generics.iter().map(|g| self.const_int(g, scope).unwrap_or(1)).collect();
                for (k, g) in f.generics.iter().enumerate() {
                    let v = gv.get(k).copied().or_else(|| g.default.as_ref().and_then(|d| self.const_int(d, sc))).unwrap_or(1);
                    let i = self.cur().vals.len();
                    self.cur().vals.push(Val::Int(v));
                    self.cur().scopes[sc].names.insert(g.name.name.clone(), (Bind::Val(i), g.name.span));
                }
                for (k, p) in f.params.iter().enumerate() {
                    let t = self.eval_type(&p.ty, sc);
                    let exp = if let TypeV::Bits(w) = t { Some(w) } else { None };
                    let v = self.eval(&args[k].value, scope, exp);
                    let v = match t {
                        TypeV::Bits(w) => Val::Sig(self.coerce(&v, w, args[k].value.span)),
                        _ => v,
                    };
                    let i = self.cur().vals.len();
                    self.cur().vals.push(v);
                    self.cur().scopes[sc].names.insert(p.name.name.clone(), (Bind::Val(i), p.name.span));
                    self.def(&p.name, DefKind::Input, format!("{}: {}", p.name.name, t.show()), None, f.span);
                }
                let rt = f.ret.as_ref().map(|t| self.eval_type(t, sc));
                let exp = match &rt {
                    Some(TypeV::Bits(w)) => Some(*w),
                    _ => expected,
                };
                self.depth += 1;
                if self.depth > 64 {
                    self.err(span, "function calls nested too deeply (recursion?)");
                    self.depth -= 1;
                    return Val::Err;
                }
                let v = self.eval_block(&f.body, sc, exp);
                self.depth -= 1;
                match rt {
                    Some(TypeV::Bits(w)) => Val::Sig(self.coerce(&v, w, f.body.span)),
                    _ => v,
                }
            }
            Some(Item::Module(m)) => {
                self.reference(callee.span, m.name.span);
                let gv: Vec<i128> = generics.iter().map(|g| self.const_int(g, scope).unwrap_or(1)).collect();
                // Resolve generic defaults.
                let mut gvals = gv.clone();
                for g in m.generics.iter().skip(gv.len()) {
                    match &g.default {
                        Some(d) => {
                            let sc = self.new_scope(None);
                            let v = self.const_int(d, sc).unwrap_or(1);
                            gvals.push(v);
                        }
                        None => {
                            self.err(span, format!("missing generic argument `{}` for `{}`", g.name.name, m.name.name));
                            gvals.push(1);
                        }
                    }
                }
                // Map arguments (named or positional) to inputs.
                let mut ordered: Vec<Option<&'a Expr>> = vec![None; m.inputs.len()];
                for (k, a) in args.iter().enumerate() {
                    match &a.name {
                        Some(n) => match m.inputs.iter().position(|p| p.name.name == n.name) {
                            Some(i) => {
                                self.reference(n.span, m.inputs[i].name.span);
                                ordered[i] = Some(&a.value)
                            }
                            None => self.err(n.span, format!("`{}` has no input `{}`", m.name.name, n.name)),
                        },
                        None => {
                            if k < ordered.len() {
                                ordered[k] = Some(&a.value);
                            } else {
                                self.err(a.value.span, "too many arguments");
                            }
                        }
                    }
                }
                // Expected widths for literal arguments: evaluate port types with generics.
                self.stack.push(Inst { scopes: vec![], thunks: vec![], wires: vec![], regs: vec![], mems: vec![], vals: vec![], group: 0, path: String::new(), record: false });
                let tsc = self.new_scope(None);
                for (k, g) in m.generics.iter().enumerate() {
                    let i = self.cur().vals.len();
                    self.cur().vals.push(Val::Int(gvals[k]));
                    self.cur().scopes[tsc].names.insert(g.name.name.clone(), (Bind::Val(i), g.name.span));
                }
                let widths: Vec<Option<u32>> = m.inputs.iter().map(|p| if let TypeV::Bits(w) = self.eval_type(&p.ty, tsc) { Some(w) } else { None }).collect();
                self.stack.pop();
                let mut inputs = Vec::new();
                for (k, o) in ordered.iter().enumerate() {
                    match o {
                        Some(e) => inputs.push(self.eval(e, scope, widths[k])),
                        None => {
                            self.err(span, format!("missing argument `{}`", m.inputs[k].name.name));
                            inputs.push(Val::Err);
                        }
                    }
                }
                let parent = self.group();
                let inst_name = format!("{}{}", m.name.name.to_lowercase(), span.start);
                let inst_label = m.name.name.to_lowercase();
                let _ = inst_name;
                let outs = self.instantiate(m, gvals, inputs, &inst_label, parent, span, true);
                Val::Inst(outs)
            }
            Some(_) => {
                self.err(callee.span, format!("`{name}` is not callable"));
                Val::Err
            }
            None => {
                self.err(callee.span, format!("unknown function or module `{name}`"));
                Val::Err
            }
        }
    }

    fn builtin(&mut self, name: &str, args: &'a [Arg], scope: usize, expected: Option<u32>, span: Span) -> Val {
        let nargs = |s: &mut Self, n: usize| -> bool {
            if args.len() != n {
                s.err(span, format!("`{name}` takes {n} argument(s), found {}", args.len()));
                false
            } else {
                true
            }
        };
        match name {
            "zext" | "sext" | "trunc" => {
                if !nargs(self, 2) {
                    return Val::Err;
                }
                let Some(w) = self.const_int(&args[1].value, scope) else { return Val::Err };
                if !(1..=64).contains(&w) {
                    self.err(args[1].value.span, "width must be between 1 and 64");
                    return Val::Err;
                }
                let w = w as u32;
                let v = self.eval(&args[0].value, scope, None);
                if let Val::Int(x) = v {
                    return Val::Sig(self.coerce(&Val::Int(x), w, args[0].value.span));
                }
                let Some(id) = self.sig(&v, None, args[0].value.span) else { return Val::Err };
                let have = self.width(id);
                match name {
                    "trunc" => {
                        if w > have {
                            self.err(span, format!("trunc to {w} bits from bits<{have}> would widen"));
                        }
                        Val::Sig(self.node(ROp::Slice(id, 0), w.min(have), span))
                    }
                    _ => {
                        if w < have {
                            self.err(span, format!("{name} to {w} bits from bits<{have}> would truncate (use trunc)"));
                            return Val::Sig(self.node(ROp::Slice(id, 0), w, span));
                        }
                        let op = if name == "zext" { ROp::Zext(id) } else { ROp::Sext(id) };
                        Val::Sig(self.node(op, w, span))
                    }
                }
            }
            "cat" => {
                let mut parts = Vec::new();
                let mut w = 0;
                for a in args {
                    let v = self.eval(&a.value, scope, None);
                    if matches!(v, Val::Int(_)) {
                        self.err(a.value.span, "cat needs sized operands");
                        return Val::Err;
                    }
                    let Some(id) = self.sig(&v, None, a.value.span) else { return Val::Err };
                    w += self.width(id);
                    parts.push(id);
                }
                if w > 64 || parts.is_empty() {
                    self.err(span, "cat result must be 1..=64 bits");
                    return Val::Err;
                }
                Val::Sig(self.node(ROp::Concat(parts), w, span))
            }
            "rep" => {
                if !nargs(self, 2) {
                    return Val::Err;
                }
                let Some(n) = self.const_int(&args[1].value, scope) else { return Val::Err };
                let v = self.eval(&args[0].value, scope, None);
                let Some(id) = self.sig(&v, Some(1), args[0].value.span) else { return Val::Err };
                let w = self.width(id) * n as u32;
                if w == 0 || w > 64 {
                    self.err(span, "rep result must be 1..=64 bits");
                    return Val::Err;
                }
                Val::Sig(self.node(ROp::Concat(vec![id; n as usize]), w, span))
            }
            "clog2" => {
                if !nargs(self, 1) {
                    return Val::Err;
                }
                let Some(n) = self.const_int(&args[0].value, scope) else { return Val::Err };
                Val::Int(clog2(n))
            }
            "width" => {
                if !nargs(self, 1) {
                    return Val::Err;
                }
                let v = self.eval(&args[0].value, scope, None);
                match v {
                    Val::Sig(id) => Val::Int(self.width(id) as i128),
                    _ => Val::Err,
                }
            }
            "any" | "all" | "parity" => {
                if !nargs(self, 1) {
                    return Val::Err;
                }
                let v = self.eval(&args[0].value, scope, None);
                let Some(id) = self.sig(&v, None, args[0].value.span) else { return Val::Err };
                let op = match name {
                    "any" => ROp::RedOr(id),
                    "all" => ROp::RedAnd(id),
                    _ => ROp::RedXor(id),
                };
                Val::Sig(self.node(op, 1, span))
            }
            "mux" => {
                if !nargs(self, 3) {
                    return Val::Err;
                }
                let cv = self.eval(&args[0].value, scope, Some(1));
                let c = self.coerce(&cv, 1, args[0].value.span);
                let a = self.eval(&args[1].value, scope, expected);
                let b = self.eval(&args[2].value, scope, expected);
                let Some(w) = self.unify_width(&a, &b, expected, span) else { return Val::Err };
                let a = self.coerce(&a, w, args[1].value.span);
                let b = self.coerce(&b, w, args[2].value.span);
                Val::Sig(self.node(ROp::Mux(c, a, b), w, span))
            }
            "slt" | "sle" | "sgt" | "sge" => {
                if !nargs(self, 2) {
                    return Val::Err;
                }
                let vs = self.arg_values(args, scope, &[None, None]);
                let Some(w) = self.unify_width(&vs[0], &vs[1], None, span) else { return Val::Err };
                let a = self.coerce(&vs[0], w, args[0].value.span);
                let b = self.coerce(&vs[1], w, args[1].value.span);
                let x = match name {
                    "slt" => self.node(ROp::Slt(a, b), 1, span),
                    "sgt" => self.node(ROp::Slt(b, a), 1, span),
                    "sle" => {
                        let t = self.node(ROp::Slt(b, a), 1, span);
                        self.node(ROp::Not(t), 1, span)
                    }
                    _ => {
                        let t = self.node(ROp::Slt(a, b), 1, span);
                        self.node(ROp::Not(t), 1, span)
                    }
                };
                Val::Sig(x)
            }
            _ => Val::Err,
        }
    }
}

fn type_src(t: &Type) -> String {
    match t {
        Type::Bit(_) => "bit".into(),
        Type::Bits(e, _) => format!("bits<{}>", expr_src(e)),
        Type::Array(t, n, _) => format!("{}[{}]", type_src(t), expr_src(n)),
        Type::Named(i) => i.name.clone(),
        Type::Uint(_) => "uint".into(),
    }
}

/// Short textual rendering of simple expressions (for signatures).
pub fn expr_src(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(v) => v.to_string(),
        ExprKind::Ident(i) => i.name.clone(),
        ExprKind::Binary(op, a, b) => format!("{}{}{}", expr_src(a), op.text(), expr_src(b)),
        ExprKind::Call { callee, args, .. } => format!("{}({})", callee.name, args.iter().map(|a| expr_src(&a.value)).collect::<Vec<_>>().join(", ")),
        ExprKind::Paren(x) => format!("({})", expr_src(x)),
        _ => "…".into(),
    }
}

pub fn module_signature(m: &Module) -> String {
    let g = if m.generics.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            m.generics
                .iter()
                .map(|g| match &g.default {
                    Some(d) => format!("{}: uint = {}", g.name.name, expr_src(d)),
                    None => format!("{}: uint", g.name.name),
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let ins: Vec<String> = m.inputs.iter().map(|p| format!("{}: {}", p.name.name, type_src(&p.ty))).collect();
    let outs: Vec<String> = m.outputs.iter().map(|p| format!("{}: {}", p.name.name, type_src(&p.ty))).collect();
    format!("module {}{g}({}) -> ({})", m.name.name, ins.join(", "), outs.join(", "))
}

/// Result of analysing a source file.
pub struct Analysis {
    pub file: File,
    pub diags: Vec<Diag>,
    pub info: SemaInfo,
}

/// Parse and check every module (standalone, with default generics) for diagnostics and
/// language-server information.
pub fn analyze(src: &str) -> Analysis {
    let (file, mut diags) = crate::syntax::parser::parse(src);
    let (sem_diags, info) = {
        let mut e = Elab::new(&file);
        for it in &file.items {
            if let Item::Module(m) = it {
                let ok_generics = m.generics.iter().all(|g| g.default.is_some());
                if ok_generics {
                    e.elab_top(m, &[]);
                }
            }
        }
        // Constants and functions are checked through their uses; check unused constants too.
        for it in &file.items {
            if let Item::Const(c) = it {
                e.global(&c.name, None);
            }
        }
        (e.diags, e.info)
    };
    diags.extend(sem_diags);
    fix_lines(src, &mut diags);
    Analysis { file, diags, info }
}

/// Group labels contain byte offsets as "line" placeholders; nothing to fix for diagnostics,
/// but keep the hook for future use.
fn fix_lines(_src: &str, _d: &mut [Diag]) {}

/// Pick the top module: `#[top]`, else the last module whose generics all have defaults.
pub fn find_top<'f>(file: &'f File, name: Option<&str>) -> Option<&'f Module> {
    let mods: Vec<&Module> = file.items.iter().filter_map(|i| if let Item::Module(m) = i { Some(m) } else { None }).collect();
    if let Some(n) = name {
        return mods.into_iter().find(|m| m.name.name == n);
    }
    if let Some(m) = mods.iter().find(|m| m.attrs.iter().any(|a| a.name.name == "top")) {
        return Some(m);
    }
    mods.into_iter().rev().find(|m| m.generics.iter().all(|g| g.default.is_some()))
}

/// Elaborate the design rooted at `top` into RTL.
pub fn elaborate(src: &str, top: Option<&str>) -> Result<(Rtl, Analysis), Analysis> {
    let mut an = analyze(src);
    if an.diags.iter().any(|d| d.severity == crate::syntax::Severity::Error) {
        return Err(an);
    }
    let Some(m) = find_top(&an.file, top) else {
        an.diags.push(Diag::error(Span::default(), "no top module found (add `#[top]` or a module without required generics)"));
        return Err(an);
    };
    let mut e = Elab::new(&an.file);
    let ok = e.elab_top(m, &[]);
    let mut rtl = std::mem::take(&mut e.rtl);
    // Replace byte-offset "line" placeholders in group names with real line numbers.
    let li = crate::syntax::LineIndex::new(src);
    for g in &mut rtl.groups {
        if let Some(p) = g.name.find("(line ") {
            let num: String = g.name[p + 6..].chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(off) = num.parse::<u32>() {
                let (l, _) = li.line_col(off);
                g.name = format!("{}(line {})", &g.name[..p], l + 1);
            }
        }
    }
    rtl.probes = e.info.signals.iter().map(|(_, n, id)| (n.clone(), *id, 0)).collect();
    let diags = e.diags;
    an.info.signals = e.info.signals;
    if !ok {
        an.diags.extend(diags);
        return Err(an);
    }
    Ok((rtl, an))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rtl::RtlSim;

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

    fn alu_ref(a: u64, b: u64, op: u64) -> (u64, u64, u64) {
        let sub = op == 1;
        let addend = if sub { !b & 0xff } else { b };
        let wide = a + addend + sub as u64;
        let arith = op == 0 || op == 1;
        let res = match op {
            2 => a & b,
            3 => a | b,
            4 => a ^ b,
            6 => a >> 1,
            7 => !a & 0xff,
            _ => wide & 0xff,
        };
        let c = if arith { ((wide >> 8) & 1) ^ sub as u64 } else if op == 6 { a & 1 } else { 0 };
        let v = (arith && (a >> 7) == (addend >> 7) && (res >> 7) != (a >> 7)) as u64;
        (res, c, v)
    }

    #[test]
    fn alu_simulates() {
        let (rtl, an) = elaborate(ALU, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        assert!(an.diags.is_empty(), "{:?}", an.diags);
        let mut s = RtlSim::new(&rtl);
        for a in (0..256).step_by(7) {
            for b in (0..256).step_by(11) {
                for op in 0..8 {
                    let o = s.step(&rtl, &[a, b, op]);
                    let (y, c, v) = alu_ref(a, b, op);
                    assert_eq!((o[0], o[1], o[2]), (y, c, v), "a={a} b={b} op={op}");
                }
            }
        }
    }

    #[test]
    fn counter_and_instances() {
        let src = r#"
module Counter<N: uint = 4>(en: bit) -> (count: bits<N>, wrap: bit) {
  reg r: bits<N> = 0
  r.next = if en { r + 1 } else { r }
  count = r
  wrap = en && r == ~0
}
module Top(en: bit) -> (lo: bits<4>, hi: bits<4>) {
  let c0 = Counter<4>(en: en)
  let c1 = Counter<4>(en: c0.wrap)
  lo = c0.count
  hi = c1.count
}
"#;
        let (rtl, an) = elaborate(src, Some("Top")).unwrap_or_else(|a| panic!("{:?}", a.diags));
        assert!(an.diags.is_empty(), "{:?}", an.diags);
        let mut s = RtlSim::new(&rtl);
        for k in 0..40u64 {
            let o = s.step(&rtl, &[1]);
            assert_eq!(o[0] + 16 * o[1], k % 256);
        }
    }

    #[test]
    fn errors_reported() {
        let src = "module M(a: bits<4>, b: bits<3>) -> (y: bits<4>) {\n  y = a + b\n  let z = q\n}\n";
        let an = analyze(src);
        let msgs: Vec<&str> = an.diags.iter().map(|d| d.message.as_str()).collect();
        assert!(msgs.iter().any(|m| m.contains("width")), "{msgs:?}");
        assert!(msgs.iter().any(|m| m.contains("unknown name `q`")), "{msgs:?}");
    }

    #[test]
    fn loops_and_partial_assign() {
        let src = r#"
fn maj(a: bit, b: bit, c: bit) -> bit { (a & b) | (a & c) | (b & c) }
module Ripple<N: uint = 8>(a: bits<N>, b: bits<N>) -> (s: bits<N>, co: bit) {
  let c: bits<N+1>
  let sum: bits<N>
  c[0] = 0
  for i in 0..N {
    sum[i] = a[i] ^ b[i] ^ c[i]
    c[i+1] = maj(a[i], b[i], c[i])
  }
  s = sum
  co = c[N]
}
"#;
        let (rtl, an) = elaborate(src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        assert!(an.diags.is_empty(), "{:?}", an.diags);
        let mut s = RtlSim::new(&rtl);
        for a in (0..256).step_by(5) {
            for b in (0..256).step_by(9) {
                let o = s.step(&rtl, &[a, b]);
                assert_eq!(o[0] + 256 * o[1], a + b);
            }
        }
    }

    #[test]
    fn comb_loop_detected() {
        let src = "module M(a: bit) -> (y: bit) {\n  let p = q & a\n  let q = p\n  y = q\n}\n";
        let an = analyze(src);
        assert!(an.diags.iter().any(|d| d.message.contains("combinational loop")), "{:?}", an.diags);
    }

    #[test]
    fn memory_and_rom() {
        let src = r#"
const PROG: bits<8>[4] = [10, 20, 30, 40]
module M(we: bit, wa: bits<2>, wd: bits<8>, ra: bits<2>) -> (rd: bits<8>, rom: bits<8>) {
  mem m: bits<8>[4]
  m.write(wa, wd, we)
  rd = m[ra]
  rom = PROG[ra]
}
"#;
        let (rtl, an) = elaborate(src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        assert!(an.diags.is_empty(), "{:?}", an.diags);
        let mut s = RtlSim::new(&rtl);
        s.step(&rtl, &[1, 2, 77, 0]);
        let o = s.step(&rtl, &[0, 0, 0, 2]);
        assert_eq!(o[0], 77);
        assert_eq!(o[1], 30);
    }
}
