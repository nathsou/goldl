//! Physical design database: everything needed to (a) emit the Life pattern and
//! (b) reconstruct the exact cell state at any generation from the logic simulation.
//!
//! Timing convention: every time in this module is relative to the start of cycle 0.
//! An event "in cycle c" happens at `time + c * period`.
//!
//! * A [`Leg`] is a glider following an exact trajectory during `[t0, t1)`; it exists in cycle
//!   `c` iff its signal is 1 in cycle `c`.
//! * An [`Inst`] is a placed stable component. It is quiescent except during
//!   `[t_contact, t_settled)` of a cycle in which its trigger fired; then the characterized
//!   flipbook frame is shown instead.
//! * A [`CrossSite`] is a bare glider/glider annihilation: active iff both signals are 1.

use crate::tech::component::{table, Oriented};
use crate::tech::glider::Traj;
use goldl_life::{Cell, Pattern};

/// What a signal id refers to when evaluating its value for a cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigKind {
    /// A GNL net (combinational value of the cycle).
    Net(u32),
    /// Register bit next-state: value of cycle c is D(c); for c = -1 it is the initial value.
    RegNext { reg: u32, bit: u32 },
    /// Top-level input bit (tapes): value of cycle c from the input log.
    Input { port: u32, bit: u32, inv: bool },
    /// Constant one.
    One,
}

#[derive(Clone, Debug)]
pub struct Leg {
    pub traj: Traj,
    pub t0: i64,
    pub t1: i64,
    pub sig: u32,
}

#[derive(Clone, Debug)]
pub struct Inst {
    pub comp: u16,
    pub tx: i64,
    pub ty: i64,
    pub dt: i64,
    /// Signal whose glider triggers the reaction (`None` for components never triggered).
    pub trigger: Option<u32>,
    pub group: u32,
}

impl Inst {
    pub fn oriented(&self) -> &'static Oriented {
        &table()[self.comp as usize]
    }
    pub fn t_contact(&self) -> i64 {
        self.oriented().t_contact + self.dt
    }
    pub fn t_settled(&self) -> i64 {
        self.oriented().t_settled + self.dt
    }
    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        let (a, b, c, d) = self.oriented().bbox;
        (a + self.tx, b + self.ty, c + self.tx, d + self.ty)
    }
}

/// Flipbook for a glider/glider annihilation, in local coordinates.
#[derive(Clone, Debug)]
pub struct CrossBook {
    pub frames: Vec<Vec<Cell>>,
}

#[derive(Clone, Debug)]
pub struct CrossSite {
    pub a: u32,
    pub b: u32,
    pub t0: i64,
    pub t1: i64,
    /// Translation of the flipbook frames.
    pub tx: i64,
    pub ty: i64,
    pub book: u32,
    pub group: u32,
    pub bbox: (i64, i64, i64, i64),
}

/// A labelled hierarchical region (for the abstraction overlay).
#[derive(Clone, Debug)]
pub struct Region {
    pub group: u32,
    /// Grid cells (gc indices) covered by this group's components.
    pub cells: Vec<(i32, i32)>,
}

/// One staircase block: the gc rectangle `[i0, i1] × [j0, j1]` holding a GNL node with its
/// input zones and launch rows.
#[derive(Clone, Debug)]
pub struct Block {
    pub node: u32,
    pub group: u32,
    pub i0: i32,
    pub j0: i32,
    pub i1: i32,
    pub j1: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Phys {
    /// Generations per clock cycle.
    pub period: i64,
    pub sigs: Vec<SigKind>,
    pub legs: Vec<Leg>,
    pub insts: Vec<Inst>,
    pub crosses: Vec<CrossSite>,
    pub books: Vec<CrossBook>,
    /// Grid pitch in rotated units and region data for the overlay.
    pub grid: i64,
    pub regions: Vec<Region>,
    /// Staircase blocks in placement order.
    pub blocks: Vec<Block>,
    /// Bounding box of the circuitry (excluding input tapes).
    pub bbox: (i64, i64, i64, i64),
    /// For each top-level input bit: the tape trajectory (cycle-0 glider) and its signal id.
    pub tapes: Vec<(Traj, u32)>,
    /// Time window (relative to cycle start) during which signals of a cycle are visible.
    pub t_min: i64,
    pub t_max: i64,
}

impl Phys {
    /// Static (quiescent) cells of every component.
    pub fn static_pattern(&self) -> Pattern {
        let mut cells = Vec::new();
        for i in &self.insts {
            let o = i.oriented();
            cells.extend(o.cells.iter().map(|&(x, y)| (x + i.tx, y + i.ty)));
        }
        Pattern::from_cells(cells)
    }
}
