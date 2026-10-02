//! Hierarchical simulation: exact cell-state reconstruction from logic values.
//!
//! Given the physical database ([`Phys`]) and a source of per-cycle signal values, the cell
//! state at any generation `g` is assembled from:
//! * gliders on legs whose signal is 1 in the relevant cycle (positions are analytic),
//! * components: quiescent cells, or a characterized flipbook frame while reacting,
//! * crossing sites: flipbook frames while both gliders annihilate.
//!
//! Cost is proportional to the number of objects in the requested rectangle, independent
//! of `g`.

use crate::phys::{Phys, SigKind};
use goldl_life::{Cell, Pattern};

/// Per-cycle signal values.
pub trait Signals {
    fn value(&self, sig: SigKind, cycle: i64) -> bool;
    /// Number of cycles for which inputs (tapes) exist; tape gliders beyond are not shown.
    fn input_cycles(&self) -> i64;
}

/// Trivial signal source (everything 0 except constant one), mostly for tests.
pub struct ConstSignals {
    pub value: bool,
    pub cycles: i64,
}

impl ConstSignals {
    pub fn zero() -> Self {
        ConstSignals { value: false, cycles: 0 }
    }
}

impl Signals for ConstSignals {
    fn value(&self, sig: SigKind, _cycle: i64) -> bool {
        matches!(sig, SigKind::One) || self.value
    }
    fn input_cycles(&self) -> i64 {
        self.cycles
    }
}

/// Inclusive rectangle filter.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl Rect {
    pub const ALL: Rect = Rect { x0: i64::MIN / 4, y0: i64::MIN / 4, x1: i64::MAX / 4, y1: i64::MAX / 4 };
    fn hits(&self, b: (i64, i64, i64, i64)) -> bool {
        b.0 <= self.x1 && b.2 >= self.x0 && b.1 <= self.y1 && b.3 >= self.y0
    }
    fn contains(&self, (x, y): Cell) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

fn cycles_for(t0: i64, t1: i64, g: i64, period: i64) -> (i64, i64) {
    // cycles c with t0 + c*T <= g < t1 + c*T
    let lo = (g - t1).div_euclid(period) + 1;
    let hi = if t0 <= i64::MIN / 8 { i64::MAX / 4 } else { (g - t0).div_euclid(period) };
    (lo, hi)
}

/// Reconstruct the live cells in `rect` at generation `g`.
pub fn reconstruct(ph: &Phys, g: i64, sig: &dyn Signals, rect: Rect, out: &mut Vec<Cell>) {
    let period = ph.period.max(1);
    let max_cycle = sig.input_cycles();
    // Glider legs.
    for leg in &ph.legs {
        let (lo, mut hi) = cycles_for(leg.t0, leg.t1, g, period);
        let kind = ph.sigs[leg.sig as usize];
        if leg.t0 <= i64::MIN / 8 {
            // Tape: only cycles that exist.
            hi = hi.min(max_cycle - 1);
        }
        if hi < lo {
            continue;
        }
        // For long tapes, restrict cycles to those whose glider is inside rect.
        let (lo, hi) = if hi - lo > 64 && rect.x0 > i64::MIN / 8 { clip_cycles_to_rect(leg.traj, g, period, lo, hi, rect) } else { (lo, hi) };
        for c in lo..=hi {
            if !sig.value(kind, c) {
                continue;
            }
            let t = g - c * period;
            for cell in leg.traj.cells_at(t) {
                if rect.contains(cell) {
                    out.push(cell);
                }
            }
        }
    }
    // Components.
    for inst in &ph.insts {
        if !rect.hits(inst.bbox()) {
            continue;
        }
        let o = inst.oriented();
        let mut drawn = false;
        if let Some(s) = inst.trigger {
            let (tc, ts) = (inst.t_contact(), inst.t_settled());
            let (lo, hi) = cycles_for(tc, ts, g, period);
            for c in lo..=hi {
                if sig.value(ph.sigs[s as usize], c) {
                    let k = (g - c * period - tc) as usize;
                    for &(x, y) in &o.frames[k] {
                        let cell = (x + inst.tx, y + inst.ty);
                        if rect.contains(cell) {
                            out.push(cell);
                        }
                    }
                    drawn = true;
                    break;
                }
            }
        }
        if !drawn {
            for &(x, y) in &o.cells {
                let cell = (x + inst.tx, y + inst.ty);
                if rect.contains(cell) {
                    out.push(cell);
                }
            }
        }
    }
    // Crossing sites.
    for cs in &ph.crosses {
        if !rect.hits(cs.bbox) {
            continue;
        }
        let (lo, hi) = cycles_for(cs.t0, cs.t1, g, period);
        for c in lo..=hi {
            let a = sig.value(ph.sigs[cs.a as usize], c);
            let b = sig.value(ph.sigs[cs.b as usize], c);
            if a && b {
                let k = (g - c * period - cs.t0) as usize;
                for &(x, y) in &ph.books[cs.book as usize].frames[k] {
                    let cell = (x + cs.tx, y + cs.ty);
                    if rect.contains(cell) {
                        out.push(cell);
                    }
                }
            }
        }
    }
}

/// Restrict a cycle range of a straight leg to cycles whose glider lies in `rect` at `g`.
fn clip_cycles_to_rect(tr: crate::tech::glider::Traj, g: i64, period: i64, lo: i64, hi: i64, rect: Rect) -> (i64, i64) {
    // Glider x position at time t: x(t) ≈ dx * (t - phi) / 4. Cycle c → t = g - c*T.
    let dx = tr.dir.dx as i64;
    // x = dx*(g - cT - phi)/4  in [x0-4, x1+4]
    let (x0, x1) = (rect.x0 - 4, rect.x1 + 4);
    // Solve for c: g - cT - phi = 4 x / dx
    let c_of = |x: i64| -> f64 { (g - tr.phi - 4 * x * dx) as f64 / period as f64 };
    let (ca, cb) = (c_of(x0), c_of(x1));
    let (cmin, cmax) = (ca.min(cb).floor() as i64 - 1, ca.max(cb).ceil() as i64 + 1);
    (lo.max(cmin), hi.min(cmax))
}

/// Whole pattern at generation `g`.
pub fn reconstruct_all(ph: &Phys, g: i64, sig: &dyn Signals) -> Pattern {
    let mut v = Vec::new();
    reconstruct(ph, g, sig, Rect::ALL, &mut v);
    Pattern::from_cells(v)
}

/// Signal values recorded from a logic simulation (one entry per cycle).
pub struct TraceSignals {
    /// nets[c][n]: value of GNL net n in cycle c.
    pub nets: Vec<Vec<bool>>,
    /// inputs[c][port][bit]
    pub inputs: Vec<Vec<Vec<bool>>>,
    /// D net of each register bit.
    pub d_net: Vec<Vec<u32>>,
    /// Initial register values.
    pub init: Vec<Vec<bool>>,
}

impl TraceSignals {
    /// Run the GNL for `inputs.len()` cycles (lane 0 of the bit-sliced simulator).
    pub fn record(g: &crate::gnl::Gnl, inputs: Vec<Vec<Vec<bool>>>) -> Self {
        let mut s = crate::gnl::GnlSim::new(g);
        let mut nets = Vec::new();
        for c in 0..inputs.len() {
            let inp: Vec<Vec<u64>> = inputs[c].iter().map(|p| p.iter().map(|&b| b as u64).collect()).collect();
            s.step(g, &inp);
            nets.push(s.nets.iter().map(|&v| v & 1 == 1).collect());
        }
        let mut d_net: Vec<Vec<u32>> = g.regs.iter().map(|r| vec![0; r.width as usize]).collect();
        for n in &g.nodes {
            if let crate::gnl::Op::RegD { reg, bit } = n.op {
                d_net[reg as usize][bit as usize] = n.ins[0];
            }
        }
        let init = g.regs.iter().map(|r| r.init.clone()).collect();
        TraceSignals { nets, inputs, d_net, init }
    }
}

impl Signals for TraceSignals {
    fn value(&self, sig: SigKind, c: i64) -> bool {
        match sig {
            SigKind::One => true,
            SigKind::Net(n) => c >= 0 && (c as usize) < self.nets.len() && self.nets[c as usize][n as usize],
            SigKind::Input { port, bit, inv } => c >= 0 && (c as usize) < self.inputs.len() && (self.inputs[c as usize][port as usize][bit as usize] != inv),
            SigKind::RegNext { reg, bit } => {
                if c < 0 {
                    c == -1 && self.init[reg as usize][bit as usize]
                } else if (c as usize) < self.nets.len() {
                    self.nets[c as usize][self.d_net[reg as usize][bit as usize] as usize]
                } else {
                    false
                }
            }
        }
    }
    fn input_cycles(&self) -> i64 {
        self.inputs.len() as i64
    }
}
