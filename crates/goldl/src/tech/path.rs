//! Exact glider path construction: place components along a glider's trajectory and
//! record the resulting glider legs.

use super::component::{find, place_on, place_turn, table, Kind, Oriented, Placed};
use super::glider::{Dir, Traj};

/// A glider leg: the glider follows `traj` during generations `[t0, t1)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LegSpec {
    pub traj: Traj,
    pub t0: i64,
    pub t1: i64,
}

/// Index of an oriented component in `component::table()`.
pub fn comp_index(o: &'static Oriented) -> u16 {
    let base = table().as_ptr();
    // SAFETY-free pointer arithmetic: both pointers come from the same slice.
    ((o as *const Oriented as usize - base as usize) / std::mem::size_of::<Oriented>()) as u16
}

/// Rotated coordinates of a point.
pub fn uv(x: i64, y: i64) -> (i64, i64) {
    (x + y, x - y)
}

/// Builds a single glider's path through components.
#[derive(Clone, Debug)]
pub struct PathBuilder {
    pub cur: Traj,
    pub t_cur: i64,
    pub legs: Vec<LegSpec>,
    pub placed: Vec<Placed>,
    /// Side outputs produced by duplicators: (trajectory, start time).
    pub side: Vec<(Traj, i64)>,
}

impl PathBuilder {
    pub fn new(start: Traj, t0: i64) -> Self {
        PathBuilder { cur: start, t_cur: t0, legs: Vec::new(), placed: Vec::new(), side: Vec::new() }
    }

    fn enter(&mut self, p: Placed) {
        self.legs.push(LegSpec { traj: self.cur, t0: self.t_cur, t1: p.t_contact() });
        self.placed.push(p);
    }

    /// Turn with a reflector of `kind` into direction `out_dir`, landing exactly on `out_lane`.
    pub fn turn(&mut self, kind: Kind, out_dir: Dir, out_lane: i64) -> Option<Placed> {
        let (o, i) = find(kind, self.cur.dir, Some(out_dir))?;
        let p = place_turn(o, self.cur, i, out_lane)?;
        self.enter(p);
        self.cur = p.output(i);
        self.t_cur = p.t_settled();
        Some(p)
    }

    /// Turn with a reflector placed at diagonal offset `k` along the lane (free position).
    pub fn turn_at(&mut self, kind: Kind, out_dir: Dir, k: i64) -> Option<Placed> {
        let (o, i) = find(kind, self.cur.dir, Some(out_dir))?;
        let p = place_on(o, self.cur, k);
        self.enter(p);
        self.cur = p.output(i);
        self.t_cur = p.t_settled();
        Some(p)
    }

    /// Pass through a duplicator placed at diagonal offset `k`, continuing on the output
    /// with the same direction; the other output is pushed to `side`.
    pub fn dup(&mut self, k: i64) -> Option<Placed> {
        let d = self.cur.dir;
        // Choose the orientation whose same-direction output is fastest (+37).
        let (o, i) = table()
            .iter()
            .filter(|o| o.kind == Kind::Dup && o.input.dir == d)
            .filter_map(|o| o.outputs.iter().position(|t| t.dir == d).map(|i| (o, i)))
            .min_by_key(|(o, i)| o.outputs[*i].phi - o.input.phi)?;
        let p = place_on(o, self.cur, k);
        self.enter(p);
        for j in 0..o.outputs.len() {
            if j != i {
                self.side.push((p.output(j), p.t_settled()));
            }
        }
        self.cur = p.output(i);
        self.t_cur = p.t_settled();
        Some(p)
    }

    /// End the path in an eater placed at diagonal offset `k`.
    pub fn eat(&mut self, k: i64) -> Placed {
        let (o, _) = find(Kind::Eater, self.cur.dir, None).expect("eater orientation");
        let p = place_on(o, self.cur, k);
        self.enter(p);
        p
    }

    /// End the path (the glider continues to `t_end`, e.g. into another structure).
    pub fn finish(&mut self, t_end: i64) {
        self.legs.push(LegSpec { traj: self.cur, t0: self.t_cur, t1: t_end });
    }

    /// Diagonal offset `k` for which a component placed with `place_on(o, cur, k)` has its
    /// bounding-box centre closest to `(x, y)`.
    pub fn k_near(&self, kind: Kind, out_dir: Option<Dir>, x: i64, y: i64) -> i64 {
        let (o, _) = find(kind, self.cur.dir, out_dir).expect("orientation");
        let p0 = place_on(o, self.cur, 0);
        let (a, b, c, d) = p0.bbox();
        let (cx, cy) = ((a + c) / 2, (b + d) / 2);
        let (dx, dy) = (self.cur.dir.dx as i64, self.cur.dir.dy as i64);
        // Moving k steps shifts the component by (k*dx, k*dy).
        ((x - cx) * dx + (y - cy) * dy) / 2
    }
}

/// The eastbound direction pair used by the fabric.
pub fn is_east(d: Dir) -> bool {
    d.dx == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tech::glider::extract_gliders;
    use goldl_life::{Cell, Pattern, Universe};

    /// Simulate a built path from its first glider and check the final glider is where the
    /// builder says it is.
    pub fn check_path(b: &PathBuilder, start: Traj, t_start: i64, t_end: i64) -> Vec<Traj> {
        let mut cells: Vec<Cell> = Vec::new();
        for p in &b.placed {
            cells.extend(p.cells());
        }
        cells.extend(start.cells_at(t_start));
        let mut u = Universe::from_pattern(&Pattern::from_cells(cells));
        u.step((t_end - t_start) as u64);
        let mut stat: Vec<Cell> = Vec::new();
        for p in &b.placed {
            stat.extend(p.cells());
        }
        let (gl, rest) = extract_gliders(&u.to_pattern(), t_end);
        assert_eq!(rest, Pattern::from_cells(stat), "components must be restored");
        gl
    }

    #[test]
    fn class_turn_adds_43() {
        // Row glider → CC turn up → duplicator on the column (eat the side output).
        let start = Traj { dir: Dir::SE, lane: 0, phi: 0 };
        let mut b = PathBuilder::new(start, -200);
        let cc = b.turn(Kind::Cc, Dir::NE, -256).unwrap();
        let _ = cc;
        // Turn point is (128, 128); put the duplicator ~50 cells further up the column.
        let k = b.k_near(Kind::Dup, Some(Dir::NE), 178, 78);
        b.dup(k).unwrap();
        let (side, ts) = b.side[0];
        let mut sb = PathBuilder::new(side, ts);
        let (sx, sy) = side.pos_at(ts + 60);
        let ke = sb.k_near(Kind::Eater, None, sx as i64, sy as i64);
        sb.eat(ke);
        let end = b.t_cur + 100;
        b.finish(end);
        let mut all = b.clone();
        all.placed.extend(sb.placed.iter().cloned());
        let gl = check_path(&all, start, -200, end);
        assert_eq!(gl, vec![b.cur]);
        assert_eq!(b.cur.phi - start.phi, 43);
        assert_eq!(b.cur.dir, Dir::NE);
    }
}
