//! Technology mapping: AIG → glider netlist (inhibit logic).
//!
//! A crossing computes `x ∧ ¬y` on x's lane and `y ∧ ¬x` on y's lane. An AND node
//! `l1 ∧ l2` is therefore `CROSS(sig(l1), sig(¬l2))`, and its free second output is the
//! AND node `¬l1 ∧ ¬l2` (reused when it exists in the AIG). Negations cost one crossing
//! with a constant-one glider. Fan-out is materialised as balanced splitter trees.

use crate::aig::{is_neg, neg, node_of, ANode, Aig, Lit, FALSE, TRUE};
use crate::gnl::{Gnl, NetId, Op, PortInfo, RegInfo};
use crate::rtl::Rtl;
use std::collections::HashMap;

/// Virtual signal: output `port` of IR node `node`.
type Vs = (usize, u8);

#[derive(Clone, Debug)]
enum Ir {
    One,
    In { port: u32, bit: u32, inv: bool },
    RegQ { reg: u32, bit: u32 },
    Cross(Vs, Vs),
    /// Two zeros crossing: a statically-zero source (only used for constant-0 register inputs).
    Zero,
}

struct Mapper<'a> {
    aig: &'a Aig,
    ir: Vec<(Ir, u32)>,
    memo: HashMap<Lit, Vs>,
    one: Option<Vs>,
    latch_of_node: HashMap<u32, (u32, u32)>,
}

impl<'a> Mapper<'a> {
    fn push(&mut self, n: Ir, group: u32) -> usize {
        self.ir.push((n, group));
        self.ir.len() - 1
    }

    /// A fresh constant-one supply stream (one per use).
    fn one(&mut self) -> Vs {
        let i = self.push(Ir::One, 0);
        self.one = Some((i, 0));
        (i, 0)
    }

    /// Rough cost of obtaining a glider signal for `l` (0 if already available).
    fn cost(&self, l: Lit) -> u32 {
        if self.memo.contains_key(&l) || matches!(self.aig.nodes[node_of(l) as usize], ANode::Input { .. }) {
            return 0;
        }
        if l == TRUE {
            return 0;
        }
        if is_neg(l) {
            2
        } else {
            1
        }
    }

    fn sig(&mut self, l: Lit) -> Vs {
        // Inputs (either polarity) and constant one are external streams: one per use.
        if l == TRUE {
            return self.one();
        }
        if let ANode::Input { port, bit } = self.aig.nodes[node_of(l) as usize] {
            let i = self.push(Ir::In { port, bit, inv: is_neg(l) }, 0);
            return (i, 0);
        }
        if let Some(&v) = self.memo.get(&l) {
            return v;
        }
        let v = if l == TRUE {
            self.one()
        } else if l == FALSE {
            panic!("constant-zero literal must be handled by the caller")
        } else if is_neg(l) {
            // NOT x = x' where ONE crosses x.
            let x = self.sig(neg(l));
            let o = self.one();
            let g = self.aig.group[node_of(l) as usize];
            let i = self.push(Ir::Cross(o, x), g);
            (i, 0)
        } else {
            let n = node_of(l);
            match self.aig.nodes[n as usize] {
                ANode::Input { .. } => unreachable!(),
                ANode::Latch { reg, bit } => {
                    let g = self.aig.group[n as usize];
                    let i = self.push(Ir::RegQ { reg, bit }, g);
                    self.latch_of_node.insert(n, (reg, bit));
                    (i, 0)
                }
                ANode::And(l1, l2) => {
                    // Choose which operand inhibits.
                    let ca = self.cost(l1) + self.cost(neg(l2));
                    let cb = self.cost(l2) + self.cost(neg(l1));
                    let (x, y) = if ca <= cb { (l1, neg(l2)) } else { (l2, neg(l1)) };
                    // Constant operands.
                    if y == FALSE {
                        // x ∧ ¬0 = x
                        let v = self.sig(x);
                        self.memo.insert(l, v);
                        return v;
                    }
                    let xs = self.sig(x);
                    let ys = self.sig(y);
                    let g = self.aig.group[n as usize];
                    let i = self.push(Ir::Cross(xs, ys), g);
                    // Second output: y ∧ ¬x = AND(¬x, y) — register it if that node exists.
                    if let Some(m) = self.aig.find_and(neg(x), y) {
                        self.memo.entry(m * 2).or_insert((i, 1));
                    }
                    (i, 0)
                }
                ANode::Const0 => unreachable!(),
            }
        };
        self.memo.insert(l, v);
        v
    }
}

/// Map an AIG (from `rtl`) to a glider netlist.
pub fn map(aig: &Aig, rtl: &Rtl) -> Gnl {
    let mut m = Mapper { aig, ir: Vec::new(), memo: HashMap::new(), one: None, latch_of_node: HashMap::new() };
    let mut outs: Vec<(u32, u32, Option<Vs>)> = Vec::new();
    for &(p, b, l) in &aig.outputs {
        let v = if l == FALSE { None } else { Some(m.sig(l)) };
        outs.push((p, b, v));
    }
    // Registers: only those whose Q is used by the logic need a loop; iterate to a fixed
    // point because register D logic may use other registers.
    let mut regd: Vec<(u32, u32, Option<Vs>)> = Vec::new();
    let mut done: HashMap<(u32, u32), bool> = HashMap::new();
    loop {
        let used: Vec<(u32, (u32, u32))> = m.latch_of_node.iter().map(|(&n, &rb)| (n, rb)).collect();
        let mut progress = false;
        for (n, (reg, bit)) in used {
            if done.contains_key(&(reg, bit)) {
                continue;
            }
            done.insert((reg, bit), true);
            progress = true;
            let l = aig.latches.iter().find(|x| x.node == n).unwrap();
            let v = if l.next == FALSE {
                let i = m.push(Ir::Zero, aig.group[n as usize]);
                Some((i, 0))
            } else {
                Some(m.sig(l.next))
            };
            regd.push((reg, bit, v));
        }
        if !progress {
            break;
        }
    }

    // ---- Emit GNL with fan-out trees ----
    let mut g = Gnl::new();
    g.groups = rtl.groups.clone();
    g.inputs = rtl.inputs.clone();
    g.outputs = rtl.outputs.iter().map(|(p, _)| PortInfo { name: p.name.clone(), width: p.width }).collect();
    g.regs = rtl.regs.iter().map(|r| RegInfo { name: r.name.clone(), width: r.width, init: (0..r.width).map(|b| (r.init >> b) & 1 == 1).collect() }).collect();
    // Use counts.
    let mut uses: HashMap<Vs, usize> = HashMap::new();
    for (n, _) in &m.ir {
        match n {
            Ir::Cross(a, b) => {
                *uses.entry(*a).or_default() += 1;
                *uses.entry(*b).or_default() += 1;
            }
            Ir::Zero => {}
            _ => {}
        }
    }
    for (_, _, v) in outs.iter().chain(regd.iter()) {
        if let Some(v) = v {
            *uses.entry(*v).or_default() += 1;
        }
    }
    let mut avail: HashMap<Vs, Vec<NetId>> = HashMap::new();
    let mut ir_node: Vec<u32> = Vec::new();
    for (k, (n, grp)) in m.ir.iter().enumerate() {
        let take = |avail: &mut HashMap<Vs, Vec<NetId>>, v: Vs| -> NetId { avail.get_mut(&v).and_then(|s| s.pop()).expect("signal used more often than counted") };
        let node = match n {
            Ir::One => g.add(Op::One, &[], 0),
            Ir::In { port, bit, inv } => g.add(Op::In { port: *port, bit: *bit, inv: *inv }, &[], 0),
            Ir::RegQ { reg, bit } => g.add(Op::RegQ { reg: *reg, bit: *bit }, &[], *grp),
            Ir::Cross(a, b) => {
                let na = take(&mut avail, *a);
                let nb = take(&mut avail, *b);
                g.add(Op::Cross, &[na, nb], *grp)
            }
            Ir::Zero => {
                // Two constant-one streams annihilate: both outputs are statically zero.
                let o1 = g.add(Op::One, &[], 0);
                let o2 = g.add(Op::One, &[], 0);
                let (na, nb) = (g.out(o1, 0), g.out(o2, 0));
                g.add(Op::Cross, &[na, nb], *grp)
            }
        };
        ir_node.push(node);
        let nouts = match n {
            Ir::Cross(..) | Ir::Zero => 2,
            _ => 1,
        };
        for port in 0..nouts {
            let v: Vs = (k, port as u8);
            let cnt = uses.get(&v).copied().unwrap_or(0);
            let net = g.out(node, port);
            let leaves = fanout(&mut g, net, cnt, *grp);
            avail.insert(v, leaves);
        }
    }
    let take = |avail: &mut HashMap<Vs, Vec<NetId>>, v: Vs| -> NetId { avail.get_mut(&v).and_then(|s| s.pop()).expect("signal count") };
    for (p, b, v) in &outs {
        if let Some(v) = v {
            let n = take(&mut avail, *v);
            g.add(Op::Out { port: *p, bit: *b }, &[n], 0);
        }
    }
    for (reg, bit, v) in &regd {
        if let Some(v) = v {
            let n = take(&mut avail, *v);
            let grp = rtl.regs[*reg as usize].group;
            g.add(Op::RegD { reg: *reg, bit: *bit }, &[n], grp);
        }
    }
    g.sink_dangling();
    g.mark_zero_nets();
    g
}

/// Build a balanced split tree giving `n` copies of `net` (0 copies → the net stays
/// dangling and gets sunk later).
fn fanout(g: &mut Gnl, net: NetId, n: usize, group: u32) -> Vec<NetId> {
    match n {
        0 => vec![],
        1 => vec![net],
        _ => {
            let s = g.add(Op::Split, &[net], group);
            let (a, b) = (g.out(s, 0), g.out(s, 1));
            let left = n / 2;
            let mut v = fanout(g, a, n - left, group);
            v.extend(fanout(g, b, left, group));
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aig::bitblast;
    use crate::elab::elaborate;
    use crate::gnl::GnlSim;
    use crate::rtl::RtlSim;

    pub fn cosim(src: &str, cycles: usize) {
        let (rtl, _) = elaborate(src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        let aig = bitblast(&rtl);
        let gnl = map(&aig, &rtl);
        let mut gs = GnlSim::new(&gnl);
        let mut rs: Vec<RtlSim> = (0..64).map(|_| RtlSim::new(&rtl)).collect();
        let mut seed = 0x9E3779B97F4A7C15u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for c in 0..cycles {
            let ins: Vec<Vec<u64>> = rtl.inputs.iter().map(|p| (0..p.width).map(|_| rnd()).collect()).collect();
            let go = gs.step(&gnl, &ins);
            for lane in 0..64 {
                let iv: Vec<u64> = ins.iter().map(|p| p.iter().enumerate().map(|(b, w)| ((w >> lane) & 1) << b).sum()).collect();
                let ro = rs[lane].step(&rtl, &iv);
                for (p, &v) in ro.iter().enumerate() {
                    let gv: u64 = go[p].iter().enumerate().map(|(b, w)| ((w >> lane) & 1) << b).sum();
                    assert_eq!(gv, v, "cycle {c} lane {lane} output {p}");
                }
            }
        }
    }

    #[test]
    fn alu_cosim() {
        cosim(
            r#"
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
"#,
            8,
        );
    }

    #[test]
    fn sequential_cosim() {
        cosim(
            r#"
module Lfsr(en: bit, load: bit, seed: bits<8>) -> (q: bits<8>, z: bit) {
  reg r: bits<8> = 1
  let fb: bit = r[7] ^ r[5] ^ r[4] ^ r[3]
  r.next = if load { seed } else if en { r[6:0] ++ fb } else { r }
  q = r
  z = r == 0
}
"#,
            40,
        );
    }
}
