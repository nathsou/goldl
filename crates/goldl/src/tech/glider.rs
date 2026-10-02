//! Exact glider trajectories.
//!
//! A glider moving in direction `(dx, dy)` (each ±1) is fully described by
//! * `lane  = dy·x − dx·y` of its phase-0 reference position (invariant along its motion), and
//! * `phi   = τ − 4·dx·x`, where `τ` is a generation at which it is in phase 0 at `(x, y)`
//!   (also invariant: every 4 generations τ grows by 4 and x by dx).
//!
//! Phase 0 is the shape `glider(dx, dy)` from `goldl_life`, whose top-left corner is the
//! reference position.

use goldl_life::{glider, Cell, Pattern};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Dir {
    pub dx: i8,
    pub dy: i8,
}

impl Dir {
    pub const SE: Dir = Dir { dx: 1, dy: 1 };
    pub const NE: Dir = Dir { dx: 1, dy: -1 };
    pub const SW: Dir = Dir { dx: -1, dy: 1 };
    pub const NW: Dir = Dir { dx: -1, dy: -1 };
    pub const ALL: [Dir; 4] = [Dir::SE, Dir::NE, Dir::SW, Dir::NW];

    pub fn name(self) -> &'static str {
        match (self.dx, self.dy) {
            (1, 1) => "SE",
            (1, -1) => "NE",
            (-1, 1) => "SW",
            _ => "NW",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Traj {
    pub dir: Dir,
    pub lane: i64,
    pub phi: i64,
}

/// The four phase shapes of a glider in direction `d`, as they evolve from phase 0
/// (not normalised: phase `k` is `glider(d).run(k)`).
pub fn phases(d: Dir) -> [Pattern; 4] {
    let g = glider(d.dx as i64, d.dy as i64);
    [g.clone(), g.run(1), g.run(2), g.run(3)]
}

impl Traj {
    /// Phase-0 reference position at the latest phase-0 generation `τ ≤ t`, plus `k = t − τ`.
    pub fn ref_at(&self, t: i64) -> (i64, i64, usize) {
        let (dx, dy) = (self.dir.dx as i64, self.dir.dy as i64);
        let m = (t - self.phi).div_euclid(4);
        let tau = self.phi + 4 * m;
        let x = dx * m;
        let y = dx * (dy * x - self.lane);
        (x, y, (t - tau) as usize)
    }

    /// Cells of the glider at generation `t`.
    pub fn cells_at(&self, t: i64) -> Vec<Cell> {
        let (x, y, k) = self.ref_at(t);
        let ph = &phases(self.dir)[k];
        ph.cells.iter().map(|&(a, b)| (a + x, b + y)).collect()
    }

    pub fn pattern_at(&self, t: i64) -> Pattern {
        Pattern::from_cells(self.cells_at(t))
    }

    /// Position (top-left of phase-0 bounding box, fractional along motion) at time t.
    pub fn pos_at(&self, t: i64) -> (f64, f64) {
        let (dx, dy) = (self.dir.dx as f64, self.dir.dy as f64);
        let (x, y, k) = self.ref_at(t);
        (
            x as f64 + dx * k as f64 / 4.0,
            y as f64 + dy * k as f64 / 4.0,
        )
    }

    /// Recognise a glider from its cells at generation `t`.
    pub fn recognize(cells: &[Cell], t: i64) -> Option<Traj> {
        if cells.len() != 5 {
            return None;
        }
        let p = Pattern::from_cells(cells.to_vec());
        let (px0, py0, _, _) = p.bbox()?;
        let pn = p.normalized();
        for d in Dir::ALL {
            for (k, ph) in phases(d).iter().enumerate() {
                if ph.normalized() == pn {
                    let (sx, sy, _, _) = ph.bbox().unwrap();
                    let (rx, ry) = (px0 - sx, py0 - sy); // phase-0 reference at τ = t − k
                    let tau = t - k as i64;
                    let (dx, dy) = (d.dx as i64, d.dy as i64);
                    return Some(Traj {
                        dir: d,
                        lane: dy * rx - dx * ry,
                        phi: tau - 4 * dx * rx,
                    });
                }
            }
        }
        None
    }

    /// Generation at which this (eastbound or westbound) glider's reference x equals `x`
    /// (fractional: phase-0 occurs at integer multiples of 4 from phi).
    pub fn time_at_x(&self, x: i64) -> i64 {
        self.phi + 4 * self.dir.dx as i64 * x
    }

    /// Apply a lattice isometry + translation to the trajectory (exact, via cells).
    pub fn transformed(&self, iso: goldl_life::Iso, tx: i64, ty: i64) -> Traj {
        let cells: Vec<Cell> = self
            .cells_at(0)
            .into_iter()
            .map(|c| {
                let (a, b) = iso.apply(c);
                (a + tx, b + ty)
            })
            .collect();
        Traj::recognize(&cells, 0).expect("isometry maps gliders to gliders")
    }

    /// Same trajectory, delayed by `dt` generations.
    pub fn delayed(&self, dt: i64) -> Traj {
        Traj {
            phi: self.phi + dt,
            ..*self
        }
    }
}

/// Split a cell set into 8-connected clusters (cells within Chebyshev distance `gap`).
pub fn clusters(cells: &[Cell], gap: i64) -> Vec<Vec<Cell>> {
    use std::collections::HashMap;
    let mut idx: HashMap<Cell, usize> = HashMap::new();
    for (i, &c) in cells.iter().enumerate() {
        idx.insert(c, i);
    }
    let mut seen = vec![false; cells.len()];
    let mut out = Vec::new();
    for i in 0..cells.len() {
        if seen[i] {
            continue;
        }
        seen[i] = true;
        let mut comp = vec![cells[i]];
        let mut stack = vec![cells[i]];
        while let Some((x, y)) = stack.pop() {
            for dy in -gap..=gap {
                for dx in -gap..=gap {
                    if let Some(&j) = idx.get(&(x + dx, y + dy)) {
                        if !seen[j] {
                            seen[j] = true;
                            comp.push(cells[j]);
                            stack.push(cells[j]);
                        }
                    }
                }
            }
        }
        out.push(comp);
    }
    out
}

/// Separate the gliders out of a pattern at generation `t`; returns (gliders, the rest).
pub fn extract_gliders(p: &Pattern, t: i64) -> (Vec<Traj>, Pattern) {
    let mut gl = Vec::new();
    let mut rest = Vec::new();
    for c in clusters(&p.cells, 1) {
        match Traj::recognize(&c, t) {
            Some(g) => gl.push(g),
            None => rest.extend(c),
        }
    }
    gl.sort();
    (gl, Pattern::from_cells(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_all_dirs_and_times() {
        for d in Dir::ALL {
            for lane in [-3, 0, 7] {
                for phi in [-5, 0, 2, 13] {
                    let tr = Traj { dir: d, lane, phi };
                    for t in [0, 1, 2, 3, 17, 40] {
                        let c = tr.cells_at(t);
                        assert_eq!(Traj::recognize(&c, t), Some(tr), "{tr:?} t={t}");
                        // Consistency with the Life rule.
                        let next = Pattern::from_cells(c).run(1);
                        assert_eq!(next, tr.pattern_at(t + 1));
                    }
                }
            }
        }
    }
}
