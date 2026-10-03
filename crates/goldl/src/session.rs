//! Interactive simulation session: a compiled design, its inputs per clock cycle, and the
//! logic traces (RTL for waveforms, glider netlist for the Life reconstruction).
//!
//! Everything about the Life universe at generation `g` is derived on demand from the trace:
//! exact cells in a viewport, glider positions and component activity for overview
//! rendering, and region outlines for the abstraction overlay.

use crate::driver::{compile, Compiled, Options};
use crate::gnl::{GnlSim, Op};
use crate::phys::{Phys, SigKind};
use crate::rtl::{mask, RtlSim};
use crate::sim::{reconstruct, Rect, Signals};
use crate::syntax::Diag;
use goldl_life::Cell;

pub struct Session {
    pub c: Compiled,
    pub src: String,
    /// Input values per cycle (one u64 per port); later cycles hold the last values.
    inputs: Vec<Vec<u64>>,
    hold: Vec<u64>,
    /// RTL node values per simulated cycle (after combinational evaluation).
    rtl_vals: Vec<Vec<u64>>,
    /// Register values at the start of each cycle.
    reg_vals: Vec<Vec<u64>>,
    /// GNL net values per cycle, bit-packed.
    nets: Vec<Vec<u64>>,
    rsim: RtlSim,
    gsim: GnlSim,
    d_net: Vec<Vec<u32>>,
    init: Vec<Vec<bool>>,
}

/// Signal view of a session limited to its simulated cycles.
struct View<'a>(&'a Session);

impl Signals for View<'_> {
    fn value(&self, sig: SigKind, c: i64) -> bool {
        let s = self.0;
        let net = |c: i64, n: u32| {
            c >= 0
                && (c as usize) < s.nets.len()
                && (s.nets[c as usize][n as usize / 64] >> (n % 64)) & 1 == 1
        };
        match sig {
            SigKind::One => true,
            SigKind::Net(n) => net(c, n),
            SigKind::Input { port, bit, inv } => {
                c >= 0
                    && (c as usize) < s.nets.len()
                    && (((s.inputs[c as usize][port as usize] >> bit) & 1 == 1) != inv)
            }
            SigKind::RegNext { reg, bit } => {
                if c < 0 {
                    c == -1 && s.init[reg as usize][bit as usize]
                } else {
                    net(c, s.d_net[reg as usize][bit as usize])
                }
            }
        }
    }
    fn input_cycles(&self) -> i64 {
        self.0.nets.len() as i64
    }
}

/// One staircase block described for the overlay: what it implements and where.
pub struct BlockInfo {
    pub i0: i32,
    pub j0: i32,
    pub i1: i32,
    pub j1: i32,
    pub group: u32,
    /// GNL node and its RTL origin (u32::MAX if none).
    pub node: u32,
    pub origin: u32,
    /// Short description, e.g. "AND (a ∧ ¬b)", "NOT", "fan-out".
    pub kind: String,
}

/// A physical I/O point of the pattern.
pub struct PortPos {
    /// "in", "out", "one", "reg-q", "reg-d"
    pub kind: &'static str,
    pub name: String,
    pub port: u32,
    pub bit: u32,
    pub inv: bool,
    pub x: f64,
    pub y: f64,
    /// Direction of the glider stream at this point (dx, dy).
    pub dir: (i8, i8),
}

/// A glider currently in flight: position and direction.
pub struct GliderPos {
    pub x: f64,
    pub y: f64,
    pub dx: i8,
    pub dy: i8,
    pub sig: u32,
}

impl Session {
    pub fn new(src: &str, top: Option<&str>) -> Result<Session, Vec<Diag>> {
        let c = compile(
            src,
            &Options {
                top: top.map(String::from),
                layout: true,
            },
        )
        .map_err(|(d, _)| d)?;
        let hold = vec![0; c.rtl.inputs.len()];
        let rsim = RtlSim::new(&c.rtl);
        let gsim = GnlSim::new(&c.gnl);
        let mut d_net: Vec<Vec<u32>> = c
            .gnl
            .regs
            .iter()
            .map(|r| vec![0; r.width as usize])
            .collect();
        for n in &c.gnl.nodes {
            if let Op::RegD { reg, bit } = n.op {
                d_net[reg as usize][bit as usize] = n.ins[0];
            }
        }
        let init = c.gnl.regs.iter().map(|r| r.init.clone()).collect();
        Ok(Session {
            c,
            src: src.to_string(),
            inputs: Vec::new(),
            hold,
            rtl_vals: Vec::new(),
            reg_vals: Vec::new(),
            nets: Vec::new(),
            rsim,
            gsim,
            d_net,
            init,
        })
    }

    pub fn phys(&self) -> Option<&Phys> {
        self.c.layout.as_ref().map(|l| &l.phys)
    }

    pub fn period(&self) -> i64 {
        self.phys().map_or(1, |p| p.period.max(1))
    }

    pub fn cycles(&self) -> usize {
        self.nets.len()
    }

    /// Set input `port` to `value` from cycle `from` on (re-simulating later cycles).
    pub fn set_input(&mut self, port: usize, value: u64, from: usize) {
        if port >= self.hold.len() {
            return;
        }
        let v = value & mask(self.c.rtl.inputs[port].width);
        self.ensure_inputs(from);
        for c in from..self.inputs.len() {
            self.inputs[c][port] = v;
        }
        self.hold[port] = v;
        self.truncate(from);
    }

    /// Input values of cycle `c` (held values for future cycles).
    pub fn inputs_at(&self, c: usize) -> Vec<u64> {
        self.inputs
            .get(c)
            .cloned()
            .unwrap_or_else(|| self.hold.clone())
    }

    fn ensure_inputs(&mut self, n: usize) {
        while self.inputs.len() < n {
            self.inputs.push(self.hold.clone());
        }
    }

    fn truncate(&mut self, from: usize) {
        if from >= self.nets.len() {
            return;
        }
        // Re-simulate from scratch up to `from` (cheap compared to rendering).
        let keep = from;
        self.nets.clear();
        self.rtl_vals.clear();
        self.reg_vals.clear();
        self.rsim = RtlSim::new(&self.c.rtl);
        self.gsim = GnlSim::new(&self.c.gnl);
        self.ensure(keep);
    }

    /// Simulate until at least `n` cycles exist.
    pub fn ensure(&mut self, n: usize) {
        let n = n.min(1 << 22);
        self.ensure_inputs(n);
        while self.nets.len() < n {
            let c = self.nets.len();
            let inp = self.inputs[c].clone();
            self.reg_vals.push(self.rsim.regs.clone());
            self.rsim.step(&self.c.rtl, &inp);
            self.rtl_vals.push(self.rsim.vals.clone());
            let gi: Vec<Vec<u64>> = self
                .c
                .rtl
                .inputs
                .iter()
                .enumerate()
                .map(|(p, port)| (0..port.width).map(|b| (inp[p] >> b) & 1).collect())
                .collect();
            self.gsim.step(&self.c.gnl, &gi);
            let mut packed = vec![0u64; self.gsim.nets.len().div_ceil(64)];
            for (k, &v) in self.gsim.nets.iter().enumerate() {
                if v & 1 == 1 {
                    packed[k / 64] |= 1 << (k % 64);
                }
            }
            self.nets.push(packed);
        }
    }

    /// Cycle being computed at generation `g` (cycle c's logic starts at c·period).
    pub fn cycle_at(&self, g: i64) -> i64 {
        g.div_euclid(self.period())
    }

    fn ensure_gen(&mut self, g: i64) {
        let c = (self.cycle_at(g) + 2).max(1) as usize;
        self.ensure(c);
    }

    /// Output port values of cycle `c` (RTL).
    pub fn outputs(&mut self, c: usize) -> Vec<u64> {
        self.ensure(c + 1);
        self.c
            .rtl
            .outputs
            .iter()
            .map(|(_, o)| self.rtl_vals[c][*o as usize])
            .collect()
    }

    /// Value of RTL node `rid` in cycle `c`.
    pub fn rtl_value(&mut self, rid: u32, c: usize) -> u64 {
        self.ensure(c + 1);
        self.rtl_vals[c].get(rid as usize).copied().unwrap_or(0)
    }

    /// Register values at the start of cycle `c`.
    pub fn regs(&mut self, c: usize) -> Vec<u64> {
        self.ensure(c + 1);
        self.reg_vals[c].clone()
    }

    /// Exact live cells in `rect` at generation `g`.
    pub fn cells(&mut self, g: i64, rect: Rect) -> Vec<Cell> {
        self.ensure_gen(g);
        let mut out = Vec::new();
        if let Some(ph) = self.phys() {
            reconstruct(ph, g, &View(self), rect, &mut out);
        }
        out
    }

    /// Gliders in flight inside `rect` at generation `g`.
    pub fn gliders(&mut self, g: i64, rect: Rect) -> Vec<GliderPos> {
        self.ensure_gen(g);
        let mut out = Vec::new();
        let Some(ph) = self.phys() else { return out };
        let period = ph.period.max(1);
        let view = View(self);
        let max_cycle = view.input_cycles();
        for leg in &ph.legs {
            let lo = (g - leg.t1).div_euclid(period) + 1;
            let mut hi = if leg.t0 <= i64::MIN / 8 {
                i64::MAX / 4
            } else {
                (g - leg.t0).div_euclid(period)
            };
            if leg.t0 <= i64::MIN / 8 {
                hi = hi.min(max_cycle - 1);
                // A tape: only cycles whose glider is near the rectangle matter.
                let dx = leg.traj.dir.dx as i64;
                // Rect::ALL uses sentinel bounds close to i64 limits. Widen before
                // computing the trajectory so full-pattern queries cannot overflow.
                let c_of = |x: i64| {
                    (g as i128 - leg.traj.phi as i128 - 4 * x as i128 * dx as i128) as f64
                        / period as f64
                };
                let (ca, cb) = (c_of(rect.x0 - 4), c_of(rect.x1 + 4));
                let lo2 = ca.min(cb).floor() as i64 - 1;
                let hi2 = ca.max(cb).ceil() as i64 + 1;
                let (lo, hi) = (lo.max(lo2), hi.min(hi2));
                for c in lo..=hi {
                    self.push_glider(&view, ph, leg, g, c, period, rect, &mut out);
                }
                continue;
            }
            for c in lo..=hi {
                self.push_glider(&view, ph, leg, g, c, period, rect, &mut out);
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn push_glider(
        &self,
        view: &View,
        ph: &Phys,
        leg: &crate::phys::Leg,
        g: i64,
        c: i64,
        period: i64,
        rect: Rect,
        out: &mut Vec<GliderPos>,
    ) {
        if !view.value(ph.sigs[leg.sig as usize], c) {
            return;
        }
        let (x, y) = leg.traj.pos_at(g - c * period);
        if x < rect.x0 as f64 || x > rect.x1 as f64 || y < rect.y0 as f64 || y > rect.y1 as f64 {
            return;
        }
        out.push(GliderPos {
            x,
            y,
            dx: leg.traj.dir.dx as i8,
            dy: leg.traj.dir.dy as i8,
            sig: leg.sig,
        });
    }

    /// Components reacting at generation `g` (indices into `phys.insts`).
    pub fn active(&mut self, g: i64, rect: Rect) -> Vec<u32> {
        self.ensure_gen(g);
        let mut out = Vec::new();
        let Some(ph) = self.phys() else { return out };
        let period = ph.period.max(1);
        let view = View(self);
        for (k, inst) in ph.insts.iter().enumerate() {
            let b = inst.bbox();
            if b.0 > rect.x1 || b.2 < rect.x0 || b.1 > rect.y1 || b.3 < rect.y0 {
                continue;
            }
            let Some(s) = inst.trigger else { continue };
            let (tc, ts) = (inst.t_contact(), inst.t_settled());
            // Show a little glow around the reaction.
            let lo = (g - ts - 200).div_euclid(period) + 1;
            let hi = (g - tc + 60).div_euclid(period);
            if (lo..=hi).any(|c| view.value(ph.sigs[s as usize], c)) {
                out.push(k as u32);
            }
        }
        out
    }

    /// Every staircase block with its description.
    pub fn blocks(&self) -> Vec<BlockInfo> {
        let Some(ph) = self.phys() else {
            return Vec::new();
        };
        let g = &self.c.gnl;
        let is_one = |net: u32| {
            matches!(
                g.nodes[g.nets[net as usize].driver.node as usize].op,
                Op::One
            )
        };
        ph.blocks
            .iter()
            .map(|b| {
                let n = &g.nodes[b.node as usize];
                let kind = match &n.op {
                    Op::Cross => {
                        if is_one(n.ins[0]) || is_one(n.ins[1]) {
                            "NOT".to_string()
                        } else {
                            "AND (a ∧ ¬b)".to_string()
                        }
                    }
                    Op::Split => "fan-out".to_string(),
                    Op::Delay { m } => format!("delay (+{} gen)", 86 * m),
                    Op::RegQ { reg, bit } => {
                        format!("register {}[{bit}] output", g.regs[*reg as usize].name)
                    }
                    Op::RegD { reg, bit } => {
                        format!("register {}[{bit}] next", g.regs[*reg as usize].name)
                    }
                    Op::Out { port, bit } => {
                        format!("output {}[{bit}]", g.outputs[*port as usize].name)
                    }
                    op => format!("{op:?}"),
                };
                // Fan-out trees inherit the origin of the signal they distribute.
                let mut origin = n.origin;
                let mut cur = b.node as usize;
                let mut guard = 0;
                while origin == u32::MAX && g.nodes[cur].op == Op::Split && guard < 64 {
                    cur = g.nets[g.nodes[cur].ins[0] as usize].driver.node as usize;
                    origin = g.nodes[cur].origin;
                    guard += 1;
                }
                BlockInfo {
                    i0: b.i0,
                    j0: b.j0,
                    i1: b.i1,
                    j1: b.j1,
                    group: b.group,
                    node: b.node,
                    origin,
                    kind,
                }
            })
            .collect()
    }

    /// Physical I/O: where every input tape enters the circuitry, where every output
    /// leaves, and where the register loops close.
    pub fn ports(&self) -> Vec<PortPos> {
        let Some(ph) = self.phys() else {
            return Vec::new();
        };
        let g = &self.c.gnl;
        let mut out = Vec::new();
        let span = (ph.bbox.3 - ph.bbox.1).max(ph.bbox.2 - ph.bbox.0) as f64;
        for leg in &ph.legs {
            if leg.t0 > i64::MIN / 8 {
                continue;
            }
            let (x1, y1) = leg.traj.pos_at(leg.t1);
            let k = (y1 - ph.bbox.3 as f64).abs().max(0.0) + span * 0.02 + 200.0;
            let (x, y) = (
                x1 - leg.traj.dir.dx as f64 * k,
                y1 - leg.traj.dir.dy as f64 * k,
            );
            let dir = (leg.traj.dir.dx, leg.traj.dir.dy);
            match ph.sigs[leg.sig as usize] {
                SigKind::Input { port, bit, inv } => out.push(PortPos {
                    kind: "in",
                    name: g.inputs[port as usize].name.clone(),
                    port,
                    bit,
                    inv,
                    x,
                    y,
                    dir,
                }),
                SigKind::One => out.push(PortPos {
                    kind: "one",
                    name: "1".into(),
                    port: 0,
                    bit: 0,
                    inv: false,
                    x,
                    y,
                    dir,
                }),
                _ => {}
            }
        }
        let gc = |i: i32, j: i32| crate::layout::gc_xy(i, j);
        for b in &ph.blocks {
            let n = &g.nodes[b.node as usize];
            let (x, y) = gc(b.i0, b.j0);
            let (x, y) = (x as f64, y as f64);
            match n.op {
                Op::Out { port, bit } => {
                    let (x, y) = gc(b.i1, (b.j0 + b.j1) / 2);
                    out.push(PortPos {
                        kind: "out",
                        name: g.outputs[port as usize].name.clone(),
                        port,
                        bit,
                        inv: false,
                        x: x as f64,
                        y: y as f64,
                        dir: (1, 1),
                    })
                }
                Op::RegQ { reg, bit } => out.push(PortPos {
                    kind: "reg-q",
                    name: g.regs[reg as usize].name.clone(),
                    port: reg,
                    bit,
                    inv: false,
                    x,
                    y,
                    dir: (1, 1),
                }),
                Op::RegD { reg, bit } => out.push(PortPos {
                    kind: "reg-d",
                    name: g.regs[reg as usize].name.clone(),
                    port: reg,
                    bit,
                    inv: false,
                    x,
                    y,
                    dir: (1, 1),
                }),
                _ => {}
            }
        }
        out
    }

    /// Value of GNL net `n` in cycle `c`.
    pub fn net_value(&mut self, n: u32, c: usize) -> bool {
        self.ensure(c + 1);
        (self.nets[c][n as usize / 64] >> (n % 64)) & 1 == 1
    }

    /// Hierarchical overlay: for each group, runs of consecutive staircase blocks belonging
    /// to it (or its descendants), as gc rectangles (i0, j0, i1, j1).
    pub fn regions(&self) -> Vec<(u32, Vec<(i32, i32, i32, i32)>)> {
        let Some(ph) = self.phys() else {
            return Vec::new();
        };
        let groups = &self.c.gnl.groups;
        let ancestors = |mut g: u32| {
            let mut v = vec![g];
            while let Some(p) = groups.get(g as usize).and_then(|x| x.parent) {
                v.push(p);
                g = p;
            }
            v
        };
        let mut runs: Vec<Vec<(i32, i32, i32, i32)>> = vec![Vec::new(); groups.len()];
        let mut open: Vec<Option<(i32, i32, i32, i32)>> = vec![None; groups.len()];
        let mut last_seen: Vec<usize> = vec![usize::MAX; groups.len()];
        for (k, b) in ph.blocks.iter().enumerate() {
            for g in ancestors(b.group) {
                let gi = g as usize;
                if gi >= groups.len() {
                    continue;
                }
                match &mut open[gi] {
                    Some(r) if last_seen[gi] + 1 == k => {
                        // Packed blocks can move left or down within the run.
                        r.0 = r.0.min(b.i0);
                        r.1 = r.1.min(b.j0);
                        r.2 = r.2.max(b.i1);
                        r.3 = r.3.max(b.j1);
                    }
                    slot => {
                        if let Some(r) = slot.take() {
                            runs[gi].push(r);
                        }
                        *slot = Some((b.i0, b.j0, b.i1, b.j1));
                    }
                }
                last_seen[gi] = k;
            }
        }
        for (g, o) in open.into_iter().enumerate() {
            if let Some(r) = o {
                runs[g].push(r);
            }
        }
        runs.into_iter()
            .enumerate()
            .filter(|(_, r)| !r.is_empty())
            .map(|(g, r)| (g as u32, r))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{reconstruct_all, TraceSignals};

    #[test]
    fn session_matches_trace() {
        let src = "module C(en: bit) -> (q: bits<2>) {\n  reg r: bits<2> = 0\n  r.next = if en { r + 1 } else { r }\n  q = r\n}\n";
        let mut s = Session::new(src, None).unwrap();
        s.set_input(0, 1, 0);
        s.set_input(0, 0, 3);
        assert_eq!(s.outputs(4), vec![3]);
        let t = s.period();
        let inputs: Vec<Vec<Vec<bool>>> = (0..6).map(|c| vec![vec![c < 3]]).collect();
        let trace = TraceSignals::record(&s.c.gnl, inputs);
        s.ensure(6);
        for g in [17, t + 5, 2 * t + t / 2] {
            let a = goldl_life::Pattern::from_cells(s.cells(g, Rect::ALL));
            let b = reconstruct_all(s.phys().unwrap(), g, &trace);
            assert_eq!(a, b, "generation {g}");
        }
        assert!(!s.gliders(t + 5, Rect::ALL).is_empty());
        assert!(!s.regions().is_empty());
    }
}
