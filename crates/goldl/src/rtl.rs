//! Word-level RTL IR: a flat, hash-consed dataflow graph (≤ 64-bit values) with registers.
//! Produced by elaboration, simulated directly (fast path), and bit-blasted for synthesis.

use crate::gnl::{Group, PortInfo};
use crate::syntax::Span;
use std::collections::HashMap;

pub type RId = u32;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ROp {
    Const(u64),
    Input(u32),
    RegQ(u32),
    Not(RId),
    And(RId, RId),
    Or(RId, RId),
    Xor(RId, RId),
    Add(RId, RId),
    Sub(RId, RId),
    Mul(RId, RId),
    Shl(RId, RId),
    Shr(RId, RId),
    Sar(RId, RId),
    Eq(RId, RId),
    Ult(RId, RId),
    Slt(RId, RId),
    Mux(RId, RId, RId),
    /// Concatenation, most significant part first.
    Concat(Vec<RId>),
    Slice(RId, u32),
    Zext(RId),
    Sext(RId),
    RedAnd(RId),
    RedOr(RId),
    RedXor(RId),
}

impl ROp {
    pub fn args(&self) -> Vec<RId> {
        use ROp::*;
        match self {
            Const(_) | Input(_) | RegQ(_) => vec![],
            Not(a) | Slice(a, _) | Zext(a) | Sext(a) | RedAnd(a) | RedOr(a) | RedXor(a) => vec![*a],
            And(a, b) | Or(a, b) | Xor(a, b) | Add(a, b) | Sub(a, b) | Mul(a, b) | Shl(a, b) | Shr(a, b) | Sar(a, b) | Eq(a, b) | Ult(a, b) | Slt(a, b) => vec![*a, *b],
            Mux(s, a, b) => vec![*s, *a, *b],
            Concat(v) => v.clone(),
        }
    }
    pub fn name(&self) -> &'static str {
        use ROp::*;
        match self {
            Const(_) => "const",
            Input(_) => "input",
            RegQ(_) => "reg",
            Not(_) => "not",
            And(..) => "and",
            Or(..) => "or",
            Xor(..) => "xor",
            Add(..) => "add",
            Sub(..) => "sub",
            Mul(..) => "mul",
            Shl(..) => "shl",
            Shr(..) => "shr",
            Sar(..) => "sar",
            Eq(..) => "eq",
            Ult(..) => "ult",
            Slt(..) => "slt",
            Mux(..) => "mux",
            Concat(_) => "concat",
            Slice(..) => "slice",
            Zext(_) => "zext",
            Sext(_) => "sext",
            RedAnd(_) => "and_r",
            RedOr(_) => "or_r",
            RedXor(_) => "xor_r",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RNode {
    pub op: ROp,
    pub width: u32,
    pub group: u32,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct RReg {
    pub name: String,
    pub width: u32,
    pub init: u64,
    pub q: RId,
    pub next: RId,
    pub group: u32,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Rtl {
    pub name: String,
    pub nodes: Vec<RNode>,
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<(PortInfo, RId)>,
    pub regs: Vec<RReg>,
    pub groups: Vec<Group>,
    /// Named internal signals (for probes/waveforms): name, node, group.
    pub probes: Vec<(String, RId, u32)>,
    cse: HashMap<(ROp, u32), RId>,
}

pub fn mask(w: u32) -> u64 {
    if w >= 64 {
        !0
    } else {
        (1u64 << w) - 1
    }
}

fn sext(v: u64, w: u32) -> i64 {
    if w >= 64 {
        v as i64
    } else {
        let s = 64 - w;
        ((v << s) as i64) >> s
    }
}

impl Rtl {
    pub fn new(name: &str) -> Self {
        let mut r = Rtl { name: name.to_string(), ..Default::default() };
        r.groups.push(Group { name: name.to_string(), kind: "module".into(), parent: None });
        r
    }

    pub fn width(&self, id: RId) -> u32 {
        self.nodes[id as usize].width
    }

    pub fn const_val(&self, id: RId) -> Option<u64> {
        match self.nodes[id as usize].op {
            ROp::Const(v) => Some(v),
            _ => None,
        }
    }

    pub fn add_group(&mut self, name: &str, kind: &str, parent: u32) -> u32 {
        self.groups.push(Group { name: name.to_string(), kind: kind.to_string(), parent: Some(parent) });
        (self.groups.len() - 1) as u32
    }

    fn raw(&mut self, op: ROp, width: u32, group: u32, span: Span) -> RId {
        let key = (op.clone(), width);
        if let Some(&id) = self.cse.get(&key) {
            return id;
        }
        let id = self.nodes.len() as RId;
        self.nodes.push(RNode { op, width, group, span });
        if !matches!(self.nodes[id as usize].op, ROp::Input(_) | ROp::RegQ(_)) {
            self.cse.insert(key, id);
        }
        id
    }

    pub fn konst(&mut self, v: u64, width: u32) -> RId {
        self.raw(ROp::Const(v & mask(width)), width, 0, Span::default())
    }

    /// Create a node with constant folding and algebraic simplification.
    pub fn node(&mut self, op: ROp, width: u32, group: u32, span: Span) -> RId {
        use ROp::*;
        let c = |r: &Rtl, x: RId| r.const_val(x);
        let m = mask(width);
        // Constant folding.
        let args = op.args();
        if !args.is_empty() && args.iter().all(|&a| c(self, a).is_some()) && !matches!(op, Input(_) | RegQ(_)) {
            let vals: Vec<u64> = args.iter().map(|&a| c(self, a).unwrap()).collect();
            let ws: Vec<u32> = args.iter().map(|&a| self.width(a)).collect();
            let v = eval_op(&op, &vals, &ws, width);
            return self.konst(v, width);
        }
        match &op {
            And(a, b) => {
                if c(self, *a) == Some(0) || c(self, *b) == Some(0) {
                    return self.konst(0, width);
                }
                if c(self, *a) == Some(m) || a == b {
                    return *b;
                }
                if c(self, *b) == Some(m) {
                    return *a;
                }
            }
            Or(a, b) => {
                if c(self, *a) == Some(0) || a == b {
                    return *b;
                }
                if c(self, *b) == Some(0) {
                    return *a;
                }
                if c(self, *a) == Some(m) || c(self, *b) == Some(m) {
                    return self.konst(m, width);
                }
            }
            Xor(a, b) => {
                if c(self, *a) == Some(0) {
                    return *b;
                }
                if c(self, *b) == Some(0) {
                    return *a;
                }
                if a == b {
                    return self.konst(0, width);
                }
            }
            Not(a) => {
                if let Not(x) = self.nodes[*a as usize].op {
                    return x;
                }
            }
            Add(a, b) | Sub(a, b) => {
                if c(self, *b) == Some(0) {
                    return *a;
                }
                if matches!(op, Add(..)) && c(self, *a) == Some(0) {
                    return *b;
                }
            }
            Shl(a, b) | Shr(a, b) | Sar(a, b) => {
                if c(self, *b) == Some(0) {
                    return *a;
                }
            }
            Mux(s, a, b) => {
                if let Some(v) = c(self, *s) {
                    return if v & 1 == 1 { *a } else { *b };
                }
                if a == b {
                    return *a;
                }
                // mux(s, 1, 0) on bits = s
                if width == 1 && c(self, *a) == Some(1) && c(self, *b) == Some(0) {
                    return *s;
                }
            }
            Slice(a, lo) => {
                if *lo == 0 && self.width(*a) == width {
                    return *a;
                }
            }
            Zext(a) | Sext(a) => {
                if self.width(*a) == width {
                    return *a;
                }
            }
            Concat(v) if v.len() == 1 => return v[0],
            _ => {}
        }
        self.raw(op, width, group, span)
    }

    pub fn input(&mut self, port: u32, width: u32, span: Span) -> RId {
        self.raw(ROp::Input(port), width, 0, span)
    }

    pub fn reg_q(&mut self, reg: u32, width: u32, group: u32, span: Span) -> RId {
        self.raw(ROp::RegQ(reg), width, group, span)
    }

    /// Nodes reachable from outputs and register next-states, in topological order.
    pub fn live_order(&self) -> Vec<RId> {
        let mut seen = vec![false; self.nodes.len()];
        let mut order = Vec::new();
        let mut stack: Vec<(RId, bool)> = Vec::new();
        for (_, o) in &self.outputs {
            stack.push((*o, false));
        }
        for r in &self.regs {
            stack.push((r.next, false));
            stack.push((r.q, false));
        }
        for (_, p, _) in &self.probes {
            stack.push((*p, false));
        }
        while let Some((n, done)) = stack.pop() {
            if done {
                order.push(n);
                continue;
            }
            if seen[n as usize] {
                continue;
            }
            seen[n as usize] = true;
            stack.push((n, true));
            for a in self.nodes[n as usize].op.args() {
                if !seen[a as usize] {
                    stack.push((a, false));
                }
            }
        }
        order
    }
}

/// Evaluate an operator on constant operands.
pub fn eval_op(op: &ROp, v: &[u64], ws: &[u32], width: u32) -> u64 {
    use ROp::*;
    let m = mask(width);
    let r = match op {
        Const(x) => *x,
        Input(_) | RegQ(_) => 0,
        Not(_) => !v[0],
        And(..) => v[0] & v[1],
        Or(..) => v[0] | v[1],
        Xor(..) => v[0] ^ v[1],
        Add(..) => v[0].wrapping_add(v[1]),
        Sub(..) => v[0].wrapping_sub(v[1]),
        Mul(..) => v[0].wrapping_mul(v[1]),
        Shl(..) => {
            if v[1] >= 64 {
                0
            } else {
                v[0] << v[1]
            }
        }
        Shr(..) => {
            if v[1] >= 64 {
                0
            } else {
                v[0] >> v[1]
            }
        }
        Sar(..) => {
            let s = sext(v[0], ws[0]);
            (s >> v[1].min(63)) as u64
        }
        Eq(..) => (v[0] == v[1]) as u64,
        Ult(..) => (v[0] < v[1]) as u64,
        Slt(..) => (sext(v[0], ws[0]) < sext(v[1], ws[1])) as u64,
        Mux(..) => {
            if v[0] & 1 == 1 {
                v[1]
            } else {
                v[2]
            }
        }
        Concat(_) => {
            let mut acc = 0u64;
            for (x, &w) in v.iter().zip(ws) {
                acc = if w >= 64 { 0 } else { acc << w };
                acc |= x & mask(w);
            }
            acc
        }
        Slice(_, lo) => v[0] >> lo,
        Zext(_) => v[0],
        Sext(_) => sext(v[0], ws[0]) as u64,
        RedAnd(_) => (v[0] & mask(ws[0]) == mask(ws[0])) as u64,
        RedOr(_) => (v[0] & mask(ws[0]) != 0) as u64,
        RedXor(_) => ((v[0] & mask(ws[0])).count_ones() & 1) as u64,
    };
    r & m
}

/// Compiled RTL simulator over `u64` values.
#[derive(Clone)]
pub struct RtlSim {
    order: Vec<RId>,
    pub vals: Vec<u64>,
    pub regs: Vec<u64>,
    pub cycle: u64,
}

impl RtlSim {
    pub fn new(r: &Rtl) -> Self {
        RtlSim { order: r.live_order(), vals: vec![0; r.nodes.len()], regs: r.regs.iter().map(|x| x.init).collect(), cycle: 0 }
    }

    pub fn reset(&mut self, r: &Rtl) {
        self.regs = r.regs.iter().map(|x| x.init).collect();
        self.cycle = 0;
    }

    /// Evaluate the combinational logic for the current register state and inputs.
    pub fn eval(&mut self, r: &Rtl, inputs: &[u64]) {
        let mut v = Vec::with_capacity(4);
        let mut ws = Vec::with_capacity(4);
        for &id in &self.order {
            let n = &r.nodes[id as usize];
            let x = match &n.op {
                ROp::Const(c) => *c,
                ROp::Input(p) => inputs.get(*p as usize).copied().unwrap_or(0) & mask(n.width),
                ROp::RegQ(k) => self.regs[*k as usize],
                op => {
                    v.clear();
                    ws.clear();
                    for a in op.args() {
                        v.push(self.vals[a as usize]);
                        ws.push(r.nodes[a as usize].width);
                    }
                    eval_op(op, &v, &ws, n.width)
                }
            };
            self.vals[id as usize] = x;
        }
    }

    pub fn outputs(&self, r: &Rtl) -> Vec<u64> {
        r.outputs.iter().map(|(_, o)| self.vals[*o as usize]).collect()
    }

    /// Evaluate and advance one clock cycle; returns the outputs of the evaluated cycle.
    pub fn step(&mut self, r: &Rtl, inputs: &[u64]) -> Vec<u64> {
        self.eval(r, inputs);
        let outs = self.outputs(r);
        for (k, reg) in r.regs.iter().enumerate() {
            self.regs[k] = self.vals[reg.next as usize] & mask(reg.width);
        }
        self.cycle += 1;
        outs
    }
}
