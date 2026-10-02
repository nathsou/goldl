//! Characterized stable components.
//!
//! Every component prototype is an RLE containing the stable circuitry plus one input
//! glider. Characterization (done once, lazily, with our own Life engine) records, for each
//! of the 8 orientations:
//! * the static cells, the input trajectory and the output trajectories,
//! * the reaction window `[t_contact, t_settled)` and a flipbook of the full local pattern
//!   for every generation of that window (used for exact reconstruction).

use super::glider::{extract_gliders, Dir, Traj};
use goldl_life::{Cell, Iso, Pattern, Universe};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Snark: 90° colour-preserving reflector.
    Snark,
    /// Bandersnatch + Snark: 90° colour-changing reflector.
    Cc,
    /// Syringe-based glider duplicator.
    Dup,
    /// Eater 1.
    Eater,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Snark, Kind::Cc, Kind::Dup, Kind::Eater];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Snark => "snark",
            Kind::Cc => "cc-reflector",
            Kind::Dup => "duplicator",
            Kind::Eater => "eater",
        }
    }
    fn rle(self) -> &'static str {
        match self {
            Kind::Snark => include_str!("../../tech/snark.rle"),
            Kind::Cc => include_str!("../../tech/bandersnatch.rle"),
            Kind::Dup => include_str!("../../tech/duplicator.rle"),
            Kind::Eater => "x = 4, y = 4\n2o$obo$2bo$2b2o!",
        }
    }
}

/// One orientation of a characterized component, at translation (0, 0).
#[derive(Clone, Debug)]
pub struct Oriented {
    pub kind: Kind,
    pub iso: Iso,
    pub cells: Vec<Cell>,
    pub input: Traj,
    pub outputs: Vec<Traj>,
    /// First generation at which the input glider deviates from free flight.
    pub t_contact: i64,
    /// First generation from which the pattern is `static + free output gliders`.
    pub t_settled: i64,
    /// Full local pattern (static + gliders in the reaction) for t in [t_contact, t_settled).
    pub frames: Vec<Vec<Cell>>,
    /// Static bounding box (inclusive).
    pub bbox: (i64, i64, i64, i64),
}

/// Eater-1 test glider: a SE glider on lane 0 aimed at an eater we position by search.
fn eater_with_glider() -> Pattern {
    // Canonical eater 1, oriented to eat a glider arriving from the north-west (moving SE).
    // Found by search (see tests); glider placed far up-left on the eaten lane.
    let eater = Pattern::parse_rle(Kind::Eater.rle()).unwrap();
    let g = goldl_life::glider(1, 1);
    // Offsets found by `find_eater_placement`.
    let (gx, gy) = EATER_GLIDER_OFFSET;
    eater.union(&g.translate(gx, gy))
}

/// Glider offset relative to the canonical eater such that a SE glider is cleanly eaten.
pub const EATER_GLIDER_OFFSET: (i64, i64) = (-6, -6);

fn characterize(kind: Kind, iso: Iso) -> Oriented {
    let base = match kind {
        Kind::Eater => eater_with_glider(),
        _ => Pattern::parse_rle(kind.rle()).expect("component RLE"),
    };
    let p = base.transform(iso);
    let (ins, stat) = extract_gliders(&p, 0);
    assert_eq!(
        ins.len(),
        1,
        "{} must contain exactly one input glider",
        kind.name()
    );
    let input = ins[0];
    assert_eq!(
        stat.run(1),
        stat,
        "{} static part must be still",
        kind.name()
    );
    let mut u = Universe::from_pattern(&p);
    let mut free = Universe::from_pattern(&input.pattern_at(0).union(&stat));
    let _ = &mut free;
    let mut t_contact = None;
    let mut frames_full: Vec<Vec<Cell>> = Vec::new();
    let horizon = 1200i64;
    for t in 0..horizon {
        let cur = u.to_pattern();
        let expect_free = stat.union(&input.pattern_at(t));
        if t_contact.is_none() && cur != expect_free {
            t_contact = Some(t);
        }
        if t_contact.is_some() {
            frames_full.push(cur.cells.clone());
        }
        u.step(1);
    }
    let t_contact = t_contact.expect("component never reacts");
    let (outputs, rest) = extract_gliders(&u.to_pattern(), horizon);
    assert_eq!(rest, stat, "{} must restore itself", kind.name());
    // Find the settle time: earliest t after which pattern == static + free outputs.
    let mut t_settled = horizon;
    for (k, f) in frames_full.iter().enumerate().rev() {
        let t = t_contact + k as i64;
        let mut expect = stat.clone();
        for o in &outputs {
            expect = expect.union(&o.pattern_at(t));
        }
        if Pattern::from_cells(f.clone()) == expect {
            t_settled = t;
        } else {
            break;
        }
    }
    let frames = frames_full[..(t_settled - t_contact) as usize].to_vec();
    let bbox = stat.bbox().unwrap();
    Oriented {
        kind,
        iso,
        cells: stat.cells,
        input,
        outputs,
        t_contact,
        t_settled,
        frames,
        bbox,
    }
}

static TABLE: OnceLock<Vec<Oriented>> = OnceLock::new();

/// All orientations of all components (characterized on first use).
pub fn table() -> &'static [Oriented] {
    TABLE.get_or_init(|| {
        let mut v = Vec::new();
        for k in Kind::ALL {
            for iso in Iso::ALL {
                v.push(characterize(k, iso));
            }
        }
        v
    })
}

/// Find an orientation of `kind` with the given input direction and an output in `out_dir`
/// (the first matching output index is returned).
pub fn find(kind: Kind, in_dir: Dir, out_dir: Option<Dir>) -> Option<(&'static Oriented, usize)> {
    for o in table()
        .iter()
        .filter(|o| o.kind == kind && o.input.dir == in_dir)
    {
        match out_dir {
            None => return Some((o, 0)),
            Some(d) => {
                if let Some(i) = o.outputs.iter().position(|t| t.dir == d) {
                    return Some((o, i));
                }
            }
        }
    }
    None
}

/// Translate a trajectory by (tx, ty) (same timing in absolute generations).
pub fn translate(t: Traj, tx: i64, ty: i64) -> Traj {
    let (dx, dy) = (t.dir.dx as i64, t.dir.dy as i64);
    Traj {
        dir: t.dir,
        lane: t.lane + dy * tx - dx * ty,
        phi: t.phi - 4 * dx * tx,
    }
}

/// A placed component: orientation, translation and time shift (delay relative to the
/// canonical characterization run).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub o: &'static Oriented,
    pub tx: i64,
    pub ty: i64,
    pub dt: i64,
}

impl Placed {
    pub fn input(&self) -> Traj {
        translate(self.o.input, self.tx, self.ty).delayed(self.dt)
    }
    pub fn output(&self, i: usize) -> Traj {
        translate(self.o.outputs[i], self.tx, self.ty).delayed(self.dt)
    }
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        self.o
            .cells
            .iter()
            .map(move |&(x, y)| (x + self.tx, y + self.ty))
    }
    pub fn t_contact(&self) -> i64 {
        self.o.t_contact + self.dt
    }
    pub fn t_settled(&self) -> i64 {
        self.o.t_settled + self.dt
    }
    pub fn bbox(&self) -> (i64, i64, i64, i64) {
        let (a, b, c, d) = self.o.bbox;
        (a + self.tx, b + self.ty, c + self.tx, d + self.ty)
    }
}

/// Place component orientation `o` so that it receives glider `g` (must have the same
/// direction), sliding it along the lane by `k` diagonal steps from the canonical solution.
/// Returns the placement; `k` selects the position along the lane.
pub fn place_on(o: &'static Oriented, g: Traj, k: i64) -> Placed {
    assert_eq!(o.input.dir, g.dir);
    let (dx, dy) = (g.dir.dx as i64, g.dir.dy as i64);
    // lane(o.input translated by (tx,ty)) = o.input.lane + dy*tx - dx*ty = g.lane.
    // Base solution with ty = 0: tx = dy * (g.lane - o.input.lane).
    let need = g.lane - o.input.lane;
    let (tx, ty) = (dy * need + k * dx, k * dy);
    let tin = translate(o.input, tx, ty);
    debug_assert_eq!(tin.lane, g.lane);
    Placed {
        o,
        tx,
        ty,
        dt: g.phi - tin.phi,
    }
}

/// Place `o` on glider `g` such that output `out` lands on lane `out_lane`.
/// Returns `None` if the lane parity is unreachable.
pub fn place_turn(o: &'static Oriented, g: Traj, out: usize, out_lane: i64) -> Option<Placed> {
    let p0 = place_on(o, g, 0);
    let p1 = place_on(o, g, 1);
    let l0 = p0.output(out).lane;
    let step = p1.output(out).lane - l0;
    if step == 0 {
        return if l0 == out_lane { Some(p0) } else { None };
    }
    let diff = out_lane - l0;
    if diff % step != 0 {
        return None;
    }
    Some(place_on(o, g, diff / step))
}

/// Vanish timings for a SE glider `a` and a NE glider `b` whose lanes sum to an even number
/// (parity-0 crossing): `b.phi - a.phi` values that annihilate both gliders within a few
/// generations. Verified by `tests::vanish_table`.
pub const VANISH_P0: [i64; 14] = [-15, -14, -13, -8, -7, -6, -5, 5, 6, 7, 8, 13, 14, 15];

/// Minimum |Δφ| for two perpendicular eastbound gliders to cross without interacting.
pub const PASS_MIN: i64 = 19;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_characterize() {
        for o in table() {
            assert!(
                !o.outputs.is_empty() || o.kind == Kind::Eater,
                "{:?}",
                o.kind
            );
            assert!(o.t_settled > o.t_contact);
        }
        let (snark, i) = find(Kind::Snark, Dir::SE, Some(Dir::NE)).unwrap();
        assert_eq!(snark.outputs[i].phi - snark.input.phi, 4);
        let (cc, i) = find(Kind::Cc, Dir::SE, Some(Dir::NE)).unwrap();
        assert_eq!(cc.outputs[i].phi - cc.input.phi, 6);
        // Eater: no outputs.
        assert!(table()
            .iter()
            .filter(|o| o.kind == Kind::Eater)
            .all(|o| o.outputs.is_empty()));
    }

    #[test]
    fn placement_matches_simulation() {
        let (cc, i) = find(Kind::Cc, Dir::SE, Some(Dir::NE)).unwrap();
        let g = Traj {
            dir: Dir::SE,
            lane: 10,
            phi: 37,
        };
        let p = place_turn(cc, g, i, -200).unwrap();
        assert_eq!(p.output(i).lane, -200);
        // Simulate: static cells + glider well before contact.
        let t0 = p.t_contact() - 40;
        let mut pat: Vec<Cell> = p.cells().collect();
        pat.extend(g.cells_at(t0));
        let mut u = Universe::from_pattern(&Pattern::from_cells(pat));
        let t1 = p.t_settled() + 50;
        u.step((t1 - t0) as u64);
        let (outs, _) = extract_gliders(&u.to_pattern(), t1);
        assert_eq!(outs, vec![p.output(i)]);
    }

    #[test]
    fn vanish_table() {
        for dphi in -24i64..=24 {
            let a = Traj {
                dir: Dir::SE,
                lane: 0,
                phi: 0,
            };
            let b = Traj {
                dir: Dir::NE,
                lane: 0,
                phi: dphi,
            };
            let p = a.pattern_at(-80).union(&b.pattern_at(-80));
            let mut u = Universe::from_pattern(&p);
            u.step(140);
            if VANISH_P0.contains(&dphi) {
                assert_eq!(u.population(), 0, "dphi={dphi} should vanish");
            }
            if dphi.abs() >= PASS_MIN {
                assert_eq!(u.population(), 10, "dphi={dphi} should pass");
            }
        }
    }
}
