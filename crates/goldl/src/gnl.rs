//! Glider netlist (GNL): the technology-mapped circuit.
//!
//! Every net carries at most one glider per clock cycle (glider = 1). Every net has exactly
//! one driver and at most one consumer; fan-out is explicit (`Split`).
//!
//! * `Cross(a, b)` → `(a ∧ ¬b, b ∧ ¬a)`: two gliders on perpendicular lanes annihilate.
//! * `Split(x)` → `(x, x)`.
//! * `One` → constant 1 (a periodic glider stream).
//! * `In`/`RegQ` sources, `Out`/`RegD`/`Sink` consumers.

use std::fmt;

pub type NetId = u32;
pub type NodeId = u32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Top-level input bit (an external glider stream; `inv` streams the complement).
    In {
        port: u32,
        bit: u32,
        inv: bool,
    },
    /// Register output bit (state at the start of the cycle).
    RegQ {
        reg: u32,
        bit: u32,
    },
    /// Constant-one source.
    One,
    Cross,
    Split,
    /// Pure delay of `2·m` phases (inserted for timing); logically the identity.
    Delay {
        m: u32,
    },
    /// Top-level output bit.
    Out {
        port: u32,
        bit: u32,
    },
    /// Register next-state bit.
    RegD {
        reg: u32,
        bit: u32,
    },
    /// Discard.
    Sink,
}

impl Op {
    pub fn n_in(&self) -> usize {
        match self {
            Op::In { .. } | Op::RegQ { .. } | Op::One => 0,
            Op::Cross => 2,
            Op::Split | Op::Delay { .. } | Op::Out { .. } | Op::RegD { .. } | Op::Sink => 1,
        }
    }
    pub fn n_out(&self) -> usize {
        match self {
            Op::In { .. } | Op::RegQ { .. } | Op::One | Op::Delay { .. } => 1,
            Op::Cross | Op::Split => 2,
            Op::Out { .. } | Op::RegD { .. } | Op::Sink => 0,
        }
    }
    pub fn is_source(&self) -> bool {
        self.n_in() == 0
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub op: Op,
    pub ins: Vec<NetId>,
    pub outs: Vec<NetId>,
    /// Index into `Gnl::groups` (hierarchical provenance: module instance / operator).
    pub group: u32,
    /// RTL node this node implements part of (u32::MAX if none).
    pub origin: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Port {
    pub node: NodeId,
    pub port: u8,
}

#[derive(Clone, Debug)]
pub struct Net {
    pub driver: Port,
    pub sink: Option<Port>,
    /// Statically known to be always 0 (no glider ever), e.g. `a ∧ ¬1`.
    pub always_zero: bool,
}

#[derive(Clone, Debug)]
pub struct RegInfo {
    pub name: String,
    pub width: u32,
    pub init: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct PortInfo {
    pub name: String,
    pub width: u32,
}

/// A provenance group: a node of the design hierarchy (module instance, operator, ...).
#[derive(Clone, Debug)]
pub struct Group {
    pub name: String,
    pub kind: String,
    pub parent: Option<u32>,
    /// Source span (bytes) of the construct, if any.
    pub span: Option<(u32, u32)>,
    /// Short source label, e.g. `r + 1` or `match op`.
    pub label: String,
}

#[derive(Clone, Debug, Default)]
pub struct Gnl {
    pub nodes: Vec<Node>,
    pub nets: Vec<Net>,
    pub inputs: Vec<PortInfo>,
    pub outputs: Vec<PortInfo>,
    pub regs: Vec<RegInfo>,
    pub groups: Vec<Group>,
    /// Scheduled input phase of every node (set by `layout::balance`; empty = ASAP).
    pub sched: Vec<i64>,
}

impl Gnl {
    pub fn new() -> Self {
        let mut g = Gnl::default();
        g.groups.push(Group {
            name: "top".into(),
            kind: "module".into(),
            parent: None,
            span: None,
            label: "top".into(),
        });
        g
    }

    pub fn add(&mut self, op: Op, ins: &[NetId], group: u32) -> NodeId {
        let id = self.nodes.len() as NodeId;
        assert_eq!(ins.len(), op.n_in(), "{op:?}");
        for (k, &n) in ins.iter().enumerate() {
            assert!(
                self.nets[n as usize].sink.is_none(),
                "net {n} already consumed"
            );
            self.nets[n as usize].sink = Some(Port {
                node: id,
                port: k as u8,
            });
        }
        let mut outs = Vec::new();
        for k in 0..op.n_out() {
            let nid = self.nets.len() as NetId;
            self.nets.push(Net {
                driver: Port {
                    node: id,
                    port: k as u8,
                },
                sink: None,
                always_zero: false,
            });
            outs.push(nid);
        }
        self.nodes.push(Node {
            op,
            ins: ins.to_vec(),
            outs,
            group,
            origin: u32::MAX,
        });
        id
    }

    /// Splice a node with one input and one output (e.g. a delay) into `net`, between its
    /// driver and its sink. Returns the new node.
    pub fn splice(&mut self, net: NetId, op: Op) -> NodeId {
        assert!(op.n_in() == 1 && op.n_out() == 1);
        let sink = self.nets[net as usize].sink.take();
        let group = sink.map_or(
            self.nodes[self.nets[net as usize].driver.node as usize].group,
            |s| self.nodes[s.node as usize].group,
        );
        let id = self.add(op, &[net], group);
        let out = self.nodes[id as usize].outs[0];
        self.nets[out as usize].always_zero = self.nets[net as usize].always_zero;
        if let Some(s) = sink {
            self.nets[out as usize].sink = Some(s);
            self.nodes[s.node as usize].ins[s.port as usize] = out;
        }
        id
    }

    pub fn out(&self, node: NodeId, k: usize) -> NetId {
        self.nodes[node as usize].outs[k]
    }

    /// Sink every unconsumed net.
    pub fn sink_dangling(&mut self) {
        for n in 0..self.nets.len() {
            if self.nets[n].sink.is_none() {
                let g = self.nodes[self.nets[n].driver.node as usize].group;
                self.add(Op::Sink, &[n as NetId], g);
            }
        }
    }

    /// Topological order of nodes (sources first). Registers break cycles by construction.
    pub fn topo(&self) -> Vec<NodeId> {
        let mut indeg: Vec<usize> = self.nodes.iter().map(|n| n.ins.len()).collect();
        let mut stack: Vec<NodeId> = (0..self.nodes.len() as NodeId)
            .filter(|&i| indeg[i as usize] == 0)
            .collect();
        stack.reverse();
        let mut order = Vec::with_capacity(self.nodes.len());
        while let Some(n) = stack.pop() {
            order.push(n);
            for &o in &self.nodes[n as usize].outs {
                if let Some(s) = self.nets[o as usize].sink {
                    indeg[s.node as usize] -= 1;
                    if indeg[s.node as usize] == 0 {
                        stack.push(s.node);
                    }
                }
            }
        }
        assert_eq!(
            order.len(),
            self.nodes.len(),
            "GNL has a combinational cycle"
        );
        order
    }

    /// Propagate statically-zero nets (`a ∧ ¬1`, splits/delays of zero nets, ...).
    pub fn mark_zero_nets(&mut self) {
        let order = self.topo();
        let mut one = vec![false; self.nets.len()];
        for &n in &order {
            let node = &self.nodes[n as usize];
            match node.op {
                Op::One => one[node.outs[0] as usize] = true,
                Op::Cross => {
                    let (a, b) = (node.ins[0] as usize, node.ins[1] as usize);
                    let za = self.nets[a].always_zero;
                    let zb = self.nets[b].always_zero;
                    let (oa, ob) = (node.outs[0] as usize, node.outs[1] as usize);
                    self.nets[oa].always_zero = za || one[b];
                    self.nets[ob].always_zero = zb || one[a];
                    one[oa] = one[a] && zb;
                    one[ob] = one[b] && za;
                }
                Op::Split | Op::Delay { .. } => {
                    let i = node.ins[0] as usize;
                    let z = self.nets[i].always_zero;
                    for &o in &node.outs.clone() {
                        self.nets[o as usize].always_zero = z;
                        one[o as usize] = one[i];
                    }
                }
                _ => {}
            }
        }
    }

    pub fn stats(&self) -> GnlStats {
        let mut s = GnlStats::default();
        for n in &self.nodes {
            match n.op {
                Op::Cross => s.cross += 1,
                Op::Split => s.split += 1,
                Op::One => s.one += 1,
                Op::Delay { .. } => s.delay += 1,
                Op::Sink => s.sink += 1,
                _ => {}
            }
        }
        s.nets = self.nets.len();
        s
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GnlStats {
    pub cross: usize,
    pub split: usize,
    pub one: usize,
    pub delay: usize,
    pub sink: usize,
    pub nets: usize,
}

impl fmt::Display for GnlStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} crossings, {} splitters, {} delays, {} sinks, {} nets",
            self.cross, self.split, self.delay, self.sink, self.nets
        )
    }
}

/// Cycle-accurate evaluator of a GNL, 64 independent lanes in parallel (bit-sliced).
pub struct GnlSim {
    order: Vec<NodeId>,
    pub nets: Vec<u64>,
    pub regs: Vec<Vec<u64>>,
}

impl GnlSim {
    pub fn new(g: &Gnl) -> Self {
        let regs = g
            .regs
            .iter()
            .map(|r| r.init.iter().map(|&b| if b { !0u64 } else { 0 }).collect())
            .collect();
        GnlSim {
            order: g.topo(),
            nets: vec![0; g.nets.len()],
            regs,
        }
    }

    /// Evaluate one cycle. `inputs[port][bit]` are bit-sliced lanes. Returns outputs
    /// `[port][bit]` and updates registers.
    pub fn step(&mut self, g: &Gnl, inputs: &[Vec<u64>]) -> Vec<Vec<u64>> {
        let mut outs: Vec<Vec<u64>> = g
            .outputs
            .iter()
            .map(|p| vec![0; p.width as usize])
            .collect();
        // Registers without a D input (unused state) are cleared.
        let mut next: Vec<Vec<u64>> = self.regs.iter().map(|r| vec![0; r.len()]).collect();
        for &n in &self.order {
            let node = &g.nodes[n as usize];
            match node.op {
                Op::In { port, bit, inv } => {
                    let v = inputs[port as usize][bit as usize];
                    self.nets[node.outs[0] as usize] = if inv { !v } else { v };
                }
                Op::RegQ { reg, bit } => {
                    self.nets[node.outs[0] as usize] = self.regs[reg as usize][bit as usize]
                }
                Op::One => self.nets[node.outs[0] as usize] = !0,
                Op::Cross => {
                    let a = self.nets[node.ins[0] as usize];
                    let b = self.nets[node.ins[1] as usize];
                    self.nets[node.outs[0] as usize] = a & !b;
                    self.nets[node.outs[1] as usize] = b & !a;
                }
                Op::Split => {
                    let a = self.nets[node.ins[0] as usize];
                    self.nets[node.outs[0] as usize] = a;
                    self.nets[node.outs[1] as usize] = a;
                }
                Op::Delay { .. } => {
                    self.nets[node.outs[0] as usize] = self.nets[node.ins[0] as usize]
                }
                Op::Out { port, bit } => {
                    outs[port as usize][bit as usize] = self.nets[node.ins[0] as usize]
                }
                Op::RegD { reg, bit } => {
                    next[reg as usize][bit as usize] = self.nets[node.ins[0] as usize]
                }
                Op::Sink => {}
            }
        }
        self.regs = next;
        outs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_gate() {
        let mut g = Gnl::new();
        g.inputs.push(PortInfo {
            name: "a".into(),
            width: 1,
        });
        g.outputs.push(PortInfo {
            name: "y".into(),
            width: 1,
        });
        let a = g.add(
            Op::In {
                port: 0,
                bit: 0,
                inv: false,
            },
            &[],
            0,
        );
        let one = g.add(Op::One, &[], 0);
        let x = g.add(Op::Cross, &[g.out(one, 0), g.out(a, 0)], 0);
        g.add(Op::Out { port: 0, bit: 0 }, &[g.out(x, 0)], 0);
        g.sink_dangling();
        g.mark_zero_nets();
        assert!(g.nets[g.out(x, 1) as usize].always_zero);
        let mut s = GnlSim::new(&g);
        let o = s.step(&g, &[vec![0b1010]]);
        assert_eq!(o[0][0] & 0xf, 0b0101);
    }
}
