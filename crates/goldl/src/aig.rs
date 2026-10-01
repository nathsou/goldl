//! And-inverter graph with latches, and bit-blasting from RTL.

use crate::rtl::{ROp, RId, Rtl};
use std::collections::HashMap;

/// Literal: `node * 2 + complement`.
pub type Lit = u32;
pub const FALSE: Lit = 0;
pub const TRUE: Lit = 1;

#[inline]
pub fn neg(l: Lit) -> Lit {
    l ^ 1
}
#[inline]
pub fn node_of(l: Lit) -> u32 {
    l >> 1
}
#[inline]
pub fn is_neg(l: Lit) -> bool {
    l & 1 == 1
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ANode {
    Const0,
    Input { port: u32, bit: u32 },
    Latch { reg: u32, bit: u32 },
    And(Lit, Lit),
}

#[derive(Clone, Debug)]
pub struct Latch {
    pub reg: u32,
    pub bit: u32,
    pub node: u32,
    pub next: Lit,
    pub init: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Aig {
    pub nodes: Vec<ANode>,
    pub group: Vec<u32>,
    strash: HashMap<(Lit, Lit), u32>,
    pub latches: Vec<Latch>,
    /// (port, bit, literal)
    pub outputs: Vec<(u32, u32, Lit)>,
    cur_group: u32,
}

impl Aig {
    pub fn new() -> Self {
        Aig { nodes: vec![ANode::Const0], group: vec![0], ..Default::default() }
    }

    pub fn input(&mut self, port: u32, bit: u32) -> Lit {
        self.nodes.push(ANode::Input { port, bit });
        self.group.push(0);
        ((self.nodes.len() - 1) as u32) * 2
    }

    pub fn latch(&mut self, reg: u32, bit: u32, init: bool, group: u32) -> Lit {
        self.nodes.push(ANode::Latch { reg, bit });
        self.group.push(group);
        let n = (self.nodes.len() - 1) as u32;
        self.latches.push(Latch { reg, bit, node: n, next: n * 2, init });
        n * 2
    }

    pub fn and(&mut self, a: Lit, b: Lit) -> Lit {
        if a == FALSE || b == FALSE || a == neg(b) {
            return FALSE;
        }
        if a == TRUE {
            return b;
        }
        if b == TRUE || a == b {
            return a;
        }
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        if let Some(&n) = self.strash.get(&(a, b)) {
            return n * 2;
        }
        self.nodes.push(ANode::And(a, b));
        self.group.push(self.cur_group);
        let n = (self.nodes.len() - 1) as u32;
        self.strash.insert((a, b), n);
        n * 2
    }

    /// Existing AND node for (a, b), if any.
    pub fn find_and(&self, a: Lit, b: Lit) -> Option<u32> {
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        self.strash.get(&(a, b)).copied()
    }

    pub fn or(&mut self, a: Lit, b: Lit) -> Lit {
        neg(self.and(neg(a), neg(b)))
    }
    pub fn xor(&mut self, a: Lit, b: Lit) -> Lit {
        // a ^ b = !( !(a & !b) & !(!a & b) )
        let x = self.and(a, neg(b));
        let y = self.and(neg(a), b);
        self.or(x, y)
    }
    pub fn mux(&mut self, s: Lit, a: Lit, b: Lit) -> Lit {
        if a == b {
            return a;
        }
        let x = self.and(s, a);
        let y = self.and(neg(s), b);
        self.or(x, y)
    }
    pub fn maj(&mut self, a: Lit, b: Lit, c: Lit) -> Lit {
        let ab = self.and(a, b);
        let x = self.xor(a, b);
        let xc = self.and(x, c);
        self.or(ab, xc)
    }

    pub fn n_ands(&self) -> usize {
        self.nodes.iter().filter(|n| matches!(n, ANode::And(..))).count()
    }

    /// Evaluate with bit-sliced 64-lane values; `inputs[port][bit]`, `latch_vals[i]`.
    pub fn eval(&self, inputs: &[Vec<u64>], latch_vals: &[u64]) -> Vec<u64> {
        let mut v = vec![0u64; self.nodes.len()];
        let mut lmap: HashMap<u32, u64> = HashMap::new();
        for (i, l) in self.latches.iter().enumerate() {
            lmap.insert(l.node, latch_vals[i]);
        }
        let lit = |v: &Vec<u64>, l: Lit| -> u64 { if is_neg(l) { !v[node_of(l) as usize] } else { v[node_of(l) as usize] } };
        for (i, n) in self.nodes.iter().enumerate() {
            v[i] = match *n {
                ANode::Const0 => 0,
                ANode::Input { port, bit } => inputs.get(port as usize).and_then(|p| p.get(bit as usize)).copied().unwrap_or(0),
                ANode::Latch { .. } => lmap[&(i as u32)],
                ANode::And(a, b) => lit(&v, a) & lit(&v, b),
            };
        }
        v
    }
    pub fn lit_val(v: &[u64], l: Lit) -> u64 {
        if is_neg(l) {
            !v[node_of(l) as usize]
        } else {
            v[node_of(l) as usize]
        }
    }
}

/// Bit-blast an RTL design.
pub fn bitblast(r: &Rtl) -> Aig {
    let mut a = Aig::new();
    let mut bits: Vec<Vec<Lit>> = vec![Vec::new(); r.nodes.len()];
    // Inputs and latches first.
    for (id, n) in r.nodes.iter().enumerate() {
        if let ROp::Input(p) = n.op {
            bits[id] = (0..n.width).map(|b| a.input(p, b)).collect();
        }
    }
    for (k, reg) in r.regs.iter().enumerate() {
        let q = reg.q as usize;
        bits[q] = (0..reg.width).map(|b| a.latch(k as u32, b, (reg.init >> b) & 1 == 1, reg.group)).collect();
    }
    let order = r.live_order();
    for &id in &order {
        let n = &r.nodes[id as usize];
        if matches!(n.op, ROp::Input(_) | ROp::RegQ(_)) {
            continue;
        }
        a.cur_group = n.group;
        let w = n.width as usize;
        let g = |x: RId| bits[x as usize].clone();
        let out: Vec<Lit> = match &n.op {
            ROp::Const(v) => (0..w).map(|b| if (v >> b) & 1 == 1 { TRUE } else { FALSE }).collect(),
            ROp::Not(x) => g(*x).into_iter().map(neg).collect(),
            ROp::And(x, y) => g(*x).iter().zip(g(*y)).map(|(&p, q)| a.and(p, q)).collect(),
            ROp::Or(x, y) => g(*x).iter().zip(g(*y)).map(|(&p, q)| a.or(p, q)).collect(),
            ROp::Xor(x, y) => g(*x).iter().zip(g(*y)).map(|(&p, q)| a.xor(p, q)).collect(),
            ROp::Add(x, y) => add(&mut a, &g(*x), &g(*y), FALSE).0,
            ROp::Sub(x, y) => {
                let ny: Vec<Lit> = g(*y).into_iter().map(neg).collect();
                add(&mut a, &g(*x), &ny, TRUE).0
            }
            ROp::Mul(x, y) => {
                let (xa, ya) = (g(*x), g(*y));
                let mut acc = vec![FALSE; w];
                for (i, &yb) in ya.iter().enumerate().take(w) {
                    let pp: Vec<Lit> = (0..w).map(|k| if k >= i { a.and(xa[k - i], yb) } else { FALSE }).collect();
                    acc = add(&mut a, &acc, &pp, FALSE).0;
                }
                acc
            }
            ROp::Shl(x, y) | ROp::Shr(x, y) | ROp::Sar(x, y) => {
                let xa = g(*x);
                let fill = if matches!(n.op, ROp::Sar(..)) { *xa.last().unwrap() } else { FALSE };
                let left = matches!(n.op, ROp::Shl(..));
                if let Some(c) = r.const_val(*y) {
                    shift_const(&xa, c as usize, left, fill)
                } else {
                    let ya = g(*y);
                    let mut cur = xa.clone();
                    let mut overflow = FALSE;
                    for (k, &sb) in ya.iter().enumerate() {
                        let amt = 1usize << k.min(40);
                        if amt >= w {
                            overflow = a.or(overflow, sb);
                            continue;
                        }
                        let sh = shift_const(&cur, amt, left, fill);
                        cur = cur.iter().zip(sh).map(|(&p, q)| a.mux(sb, q, p)).collect();
                    }
                    if overflow != FALSE {
                        cur = cur.into_iter().map(|b| a.mux(overflow, fill, b)).collect();
                    }
                    cur
                }
            }
            ROp::Eq(x, y) => {
                let mut acc = TRUE;
                for (&p, q) in g(*x).iter().zip(g(*y)) {
                    let e = neg(a.xor(p, q));
                    acc = a.and(acc, e);
                }
                vec![acc]
            }
            ROp::Ult(x, y) => vec![ult(&mut a, &g(*x), &g(*y))],
            ROp::Slt(x, y) => {
                let mut xa = g(*x);
                let mut ya = g(*y);
                let l = xa.len() - 1;
                xa[l] = neg(xa[l]);
                ya[l] = neg(ya[l]);
                vec![ult(&mut a, &xa, &ya)]
            }
            ROp::Mux(s, x, y) => {
                let sb = g(*s)[0];
                g(*x).iter().zip(g(*y)).map(|(&p, q)| a.mux(sb, p, q)).collect()
            }
            ROp::Concat(parts) => {
                let mut v = Vec::new();
                for p in parts.iter().rev() {
                    v.extend(g(*p));
                }
                v
            }
            ROp::Slice(x, lo) => g(*x)[*lo as usize..*lo as usize + w].to_vec(),
            ROp::Zext(x) => {
                let mut v = g(*x);
                v.resize(w, FALSE);
                v
            }
            ROp::Sext(x) => {
                let mut v = g(*x);
                let s = *v.last().unwrap();
                v.resize(w, s);
                v
            }
            ROp::RedAnd(x) => vec![g(*x).into_iter().fold(TRUE, |acc, b| a.and(acc, b))],
            ROp::RedOr(x) => vec![g(*x).into_iter().fold(FALSE, |acc, b| a.or(acc, b))],
            ROp::RedXor(x) => vec![g(*x).into_iter().fold(FALSE, |acc, b| a.xor(acc, b))],
            ROp::Input(_) | ROp::RegQ(_) => unreachable!(),
        };
        debug_assert_eq!(out.len(), w);
        bits[id as usize] = out;
    }
    for (k, reg) in r.regs.iter().enumerate() {
        let nb = bits[reg.next as usize].clone();
        for (b, &l) in nb.iter().enumerate() {
            let li = a.latches.iter().position(|x| x.reg == k as u32 && x.bit == b as u32).unwrap();
            a.latches[li].next = l;
        }
    }
    for (p, (_, o)) in r.outputs.iter().enumerate() {
        for (b, &l) in bits[*o as usize].iter().enumerate() {
            a.outputs.push((p as u32, b as u32, l));
        }
    }
    a
}

fn shift_const(x: &[Lit], amt: usize, left: bool, fill: Lit) -> Vec<Lit> {
    let w = x.len();
    (0..w)
        .map(|i| {
            if left {
                if i >= amt {
                    x[i - amt]
                } else {
                    FALSE
                }
            } else if i + amt < w {
                x[i + amt]
            } else {
                fill
            }
        })
        .collect()
}

fn add(a: &mut Aig, x: &[Lit], y: &[Lit], cin: Lit) -> (Vec<Lit>, Lit) {
    let mut c = cin;
    let mut s = Vec::with_capacity(x.len());
    for (&p, &q) in x.iter().zip(y) {
        let t = a.xor(p, q);
        s.push(a.xor(t, c));
        // carry = (p & q) | (t & c)
        let pq = a.and(p, q);
        let tc = a.and(t, c);
        c = a.or(pq, tc);
    }
    (s, c)
}

/// a < b (unsigned): borrow out of a - b.
fn ult(a: &mut Aig, x: &[Lit], y: &[Lit]) -> Lit {
    // compute a + !b + 1 carry; a < b iff no carry.
    let ny: Vec<Lit> = y.iter().map(|&l| neg(l)).collect();
    let (_, c) = add(a, x, &ny, TRUE);
    neg(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elab::elaborate;
    use crate::rtl::RtlSim;

    #[test]
    fn bitblast_matches_rtl() {
        let src = r#"
module M(a: bits<6>, b: bits<6>, s: bits<3>) -> (y: bits<6>, z: bits<6>, l: bit, q: bits<6>, m: bits<6>, sg: bit) {
  y = a + b
  z = a - b
  l = a < b
  q = a >> s
  m = a * b
  sg = slt(a, b)
}
"#;
        let (rtl, _) = elaborate(src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
        let aig = bitblast(&rtl);
        let mut sim = RtlSim::new(&rtl);
        let mut seed = 12345u64;
        let mut rnd = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        // 64 lanes at once.
        let ins: Vec<Vec<u64>> = vec![(0..6).map(|_| rnd()).collect(), (0..6).map(|_| rnd()).collect(), (0..3).map(|_| rnd()).collect()];
        let v = aig.eval(&ins, &[]);
        for lane in 0..64 {
            let get = |p: usize| -> u64 { (0..ins[p].len()).map(|b| ((ins[p][b] >> lane) & 1) << b).sum() };
            let o = sim.step(&rtl, &[get(0), get(1), get(2)]);
            for &(p, b, l) in &aig.outputs {
                let bitv = (Aig::lit_val(&v, l) >> lane) & 1;
                assert_eq!(bitv, (o[p as usize] >> b) & 1, "output {p} bit {b} lane {lane}");
            }
        }
    }
}
