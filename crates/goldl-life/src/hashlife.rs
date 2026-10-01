//! Gosper's HashLife.
//!
//! Leaves are 8×8 blocks packed in a `u64` (bit `y*8 + x`); internal nodes are hash-consed
//! quadtrees. `advance(node, j)` returns the centre half of a node advanced by `2^j`
//! generations; full-speed results (`j = level - 2`) are memoised on the node, and smaller
//! steps are memoised in a side table, so arbitrary generation counts are reached by binary
//! decomposition.

use crate::universe::{next_row, FastMap};
use crate::{Cell, Pattern};

type Id = u32;

#[derive(Clone, Copy)]
enum Kind {
    Leaf(u64),
    Node([Id; 4]), // nw, ne, sw, se
}

#[derive(Clone, Copy)]
struct Node {
    level: u8,
    kind: Kind,
    pop: u64,
    result: Id, // memo for full-speed step (u32::MAX if unknown)
}

const NONE: Id = u32::MAX;

pub struct HashLife {
    nodes: Vec<Node>,
    leaf_map: FastMap<u64, Id>,
    node_map: FastMap<[Id; 4], Id>,
    step_memo: FastMap<(Id, u8), Id>,
    empties: Vec<Id>, // empty node per level
    root: Id,
    /// Coordinates of the root's top-left corner.
    origin: (i64, i64),
    pub generation: u64,
}

impl Default for HashLife {
    fn default() -> Self {
        Self::new()
    }
}

fn step_rows(rows: &mut [u64], n: usize) {
    for _ in 0..n {
        let mut out = vec![0u64; rows.len()];
        for r in 0..rows.len() {
            let a = if r > 0 { rows[r - 1] } else { 0 };
            let b = if r + 1 < rows.len() { rows[r + 1] } else { 0 };
            out[r] = next_row(a, 0, 0, rows[r], 0, 0, b, 0, 0);
        }
        rows.copy_from_slice(&out);
    }
}

impl HashLife {
    pub fn new() -> Self {
        let mut h = HashLife {
            nodes: Vec::new(),
            leaf_map: FastMap::default(),
            node_map: FastMap::default(),
            step_memo: FastMap::default(),
            empties: Vec::new(),
            root: 0,
            origin: (0, 0),
            generation: 0,
        };
        let e3 = h.leaf(0);
        h.empties = vec![NONE, NONE, NONE, e3];
        h.root = h.empty(4);
        h.origin = (-8, -8);
        h
    }

    fn leaf(&mut self, bits: u64) -> Id {
        if let Some(&id) = self.leaf_map.get(&bits) {
            return id;
        }
        let id = self.nodes.len() as Id;
        self.nodes.push(Node { level: 3, kind: Kind::Leaf(bits), pop: bits.count_ones() as u64, result: NONE });
        self.leaf_map.insert(bits, id);
        id
    }

    fn join(&mut self, q: [Id; 4]) -> Id {
        if let Some(&id) = self.node_map.get(&q) {
            return id;
        }
        let level = self.nodes[q[0] as usize].level + 1;
        let pop = q.iter().map(|&c| self.nodes[c as usize].pop).sum();
        let id = self.nodes.len() as Id;
        self.nodes.push(Node { level, kind: Kind::Node(q), pop, result: NONE });
        self.node_map.insert(q, id);
        id
    }

    fn empty(&mut self, level: u8) -> Id {
        while self.empties.len() <= level as usize {
            let prev = *self.empties.last().unwrap();
            let e = self.join([prev; 4]);
            self.empties.push(e);
        }
        self.empties[level as usize]
    }

    fn children(&self, id: Id) -> [Id; 4] {
        match self.nodes[id as usize].kind {
            Kind::Node(q) => q,
            Kind::Leaf(_) => panic!("leaf has no children"),
        }
    }

    fn level(&self, id: Id) -> u8 {
        self.nodes[id as usize].level
    }

    pub fn population(&self) -> u64 {
        self.nodes[self.root as usize].pop
    }

    pub fn from_pattern(p: &Pattern) -> Self {
        let mut h = HashLife::new();
        h.set_pattern(p);
        h
    }

    pub fn set_pattern(&mut self, p: &Pattern) {
        let Some((x0, y0, x1, y1)) = p.bbox() else {
            return;
        };
        let size = (x1 - x0 + 1).max(y1 - y0 + 1).max(16);
        let mut level = 4u8;
        while (1i64 << level) < size {
            level += 1;
        }
        let (ox, oy) = (x0, y0);
        let mut cells = p.cells.clone();
        self.root = self.build(&mut cells, ox, oy, level);
        self.origin = (ox, oy);
        self.generation = 0;
    }

    fn build(&mut self, cells: &mut [Cell], ox: i64, oy: i64, level: u8) -> Id {
        if cells.is_empty() {
            return self.empty(level);
        }
        if level == 3 {
            let mut bits = 0u64;
            for &(x, y) in cells.iter() {
                bits |= 1 << ((y - oy) * 8 + (x - ox));
            }
            return self.leaf(bits);
        }
        let half = 1i64 << (level - 1);
        let (mx, my) = (ox + half, oy + half);
        // Partition into quadrants.
        let (top, bottom): (Vec<Cell>, Vec<Cell>) = cells.iter().partition(|c| c.1 < my);
        let (mut nw, mut ne): (Vec<Cell>, Vec<Cell>) = top.into_iter().partition(|c| c.0 < mx);
        let (mut sw, mut se): (Vec<Cell>, Vec<Cell>) = bottom.into_iter().partition(|c| c.0 < mx);
        let a = self.build(&mut nw, ox, oy, level - 1);
        let b = self.build(&mut ne, mx, oy, level - 1);
        let c = self.build(&mut sw, ox, my, level - 1);
        let d = self.build(&mut se, mx, my, level - 1);
        self.join([a, b, c, d])
    }

    pub fn to_pattern(&self) -> Pattern {
        let mut out = Vec::new();
        self.collect(self.root, self.origin.0, self.origin.1, &mut out);
        Pattern::from_cells(out)
    }

    fn collect(&self, id: Id, ox: i64, oy: i64, out: &mut Vec<Cell>) {
        let n = &self.nodes[id as usize];
        if n.pop == 0 {
            return;
        }
        match n.kind {
            Kind::Leaf(bits) => {
                let mut w = bits;
                while w != 0 {
                    let b = w.trailing_zeros() as i64;
                    w &= w - 1;
                    out.push((ox + b % 8, oy + b / 8));
                }
            }
            Kind::Node(q) => {
                let half = 1i64 << (n.level - 1);
                self.collect(q[0], ox, oy, out);
                self.collect(q[1], ox + half, oy, out);
                self.collect(q[2], ox, oy + half, out);
                self.collect(q[3], ox + half, oy + half, out);
            }
        }
    }

    /// The 16 rows (bits 8..24) of a level-4 node.
    fn rows16(&self, id: Id) -> [u64; 16] {
        let q = self.children(id);
        let mut rows = [0u64; 16];
        for (i, &c) in q.iter().enumerate() {
            let Kind::Leaf(bits) = self.nodes[c as usize].kind else { unreachable!() };
            let (dx, dy) = ((i % 2) * 8, (i / 2) * 8);
            for y in 0..8 {
                let row = (bits >> (y * 8)) & 0xff;
                rows[dy + y] |= row << (dx + 8); // 8-bit margin keeps shifts clean
            }
        }
        rows
    }

    fn centre_leaf(&mut self, rows: &[u64; 16]) -> Id {
        let mut bits = 0u64;
        for y in 0..8 {
            let row = (rows[4 + y] >> (4 + 8)) & 0xff;
            bits |= row << (y * 8);
        }
        self.leaf(bits)
    }

    /// Level-4 node (16×16) advanced `steps` (≤ 4) generations → centre 8×8 leaf.
    fn base_step(&mut self, id: Id, steps: usize) -> Id {
        let mut rows = self.rows16(id);
        step_rows(&mut rows, steps);
        self.centre_leaf(&rows)
    }

    /// Centre sub-node (level - 1) without time advance.
    fn centre(&mut self, id: Id) -> Id {
        if self.level(id) == 4 {
            let rows = self.rows16(id);
            return self.centre_leaf(&rows);
        }
        let [a, b, c, d] = self.children(id);
        let a3 = self.children(a)[3];
        let b2 = self.children(b)[2];
        let c1 = self.children(c)[1];
        let d0 = self.children(d)[0];
        self.join([a3, b2, c1, d0])
    }

    /// The nine overlapping level-(k-1) sub-squares of a level-k node.
    fn nine(&mut self, id: Id) -> [Id; 9] {
        let [a, b, c, d] = self.children(id);
        let [a0, a1, a2, a3] = self.children(a);
        let [b0, b1, b2, b3] = self.children(b);
        let [c0, c1, c2, c3] = self.children(c);
        let [d0, d1, d2, d3] = self.children(d);
        let _ = (a0, b1, c2, d3);
        [
            a,
            self.join([a1, b0, a3, b2]),
            b,
            self.join([a2, a3, c0, c1]),
            self.join([a3, b2, c1, d0]),
            self.join([b2, b3, d0, d1]),
            c,
            self.join([c1, d0, c3, d2]),
            d,
        ]
    }

    /// Centre half of `id`, advanced `2^j` generations (`j <= level - 2`).
    fn advance(&mut self, id: Id, j: u8) -> Id {
        let level = self.level(id);
        debug_assert!(level >= 4 && j <= level - 2);
        if self.nodes[id as usize].pop == 0 {
            return self.empty(level - 1);
        }
        let full = j == level - 2;
        if full {
            let r = self.nodes[id as usize].result;
            if r != NONE {
                return r;
            }
        } else if let Some(&r) = self.step_memo.get(&(id, j)) {
            return r;
        }
        let result = if level == 4 {
            self.base_step(id, 1 << j)
        } else {
            let n = self.nine(id);
            let mut m = [0; 9];
            for i in 0..9 {
                m[i] = if full { self.advance(n[i], level - 3) } else { self.centre(n[i]) };
            }
            let q0 = self.join([m[0], m[1], m[3], m[4]]);
            let q1 = self.join([m[1], m[2], m[4], m[5]]);
            let q2 = self.join([m[3], m[4], m[6], m[7]]);
            let q3 = self.join([m[4], m[5], m[7], m[8]]);
            let jj = if full { level - 3 } else { j };
            let r0 = self.advance(q0, jj);
            let r1 = self.advance(q1, jj);
            let r2 = self.advance(q2, jj);
            let r3 = self.advance(q3, jj);
            self.join([r0, r1, r2, r3])
        };
        if full {
            self.nodes[id as usize].result = result;
        } else {
            self.step_memo.insert((id, j), result);
        }
        result
    }

    /// Wrap the root in empty space (doubling its size, keeping it centred).
    fn expand(&mut self) {
        let level = self.level(self.root);
        let e = self.empty(level - 1);
        let [a, b, c, d] = self.children(self.root);
        let na = self.join([e, e, e, a]);
        let nb = self.join([e, e, b, e]);
        let nc = self.join([e, c, e, e]);
        let nd = self.join([d, e, e, e]);
        self.root = self.join([na, nb, nc, nd]);
        let quarter = 1i64 << (level - 1);
        self.origin = (self.origin.0 - quarter, self.origin.1 - quarter);
    }

    /// True if all live cells are inside the centre half... of the centre half (safe margin).
    fn well_padded(&mut self) -> bool {
        let root = self.root;
        let level = self.level(root);
        if level < 6 {
            return false;
        }
        let c = self.centre(root);
        let cc = self.centre(c);
        self.nodes[cc as usize].pop == self.nodes[root as usize].pop
    }

    /// Advance exactly `n` generations.
    pub fn step(&mut self, mut n: u64) {
        let mut j: u8 = 0;
        while n > 0 {
            if n & 1 == 1 {
                while !self.well_padded() || self.level(self.root) < j + 3 {
                    self.expand();
                }
                let level = self.level(self.root);
                let r = self.advance(self.root, j);
                let quarter = 1i64 << (level - 2);
                self.origin = (self.origin.0 + quarter, self.origin.1 + quarter);
                self.root = r;
                self.generation += 1 << j;
                if self.nodes.len() > 20_000_000 {
                    self.compact();
                }
            }
            n >>= 1;
            j += 1;
        }
    }

    /// Drop all memoised state (crude garbage collection).
    fn compact(&mut self) {
        let p = self.to_pattern();
        let g = self.generation;
        *self = HashLife::from_pattern(&p);
        self.generation = g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{glider, Universe};

    fn r_pentomino() -> Pattern {
        Pattern::from_cells(vec![(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)])
    }

    #[test]
    fn matches_universe_small_steps() {
        let p = r_pentomino();
        for n in [1u64, 2, 3, 5, 7, 13, 64, 100] {
            let mut h = HashLife::from_pattern(&p);
            h.step(n);
            let mut u = Universe::from_pattern(&p);
            u.step(n);
            assert_eq!(h.to_pattern(), u.to_pattern(), "after {n} gens");
        }
    }

    #[test]
    fn matches_universe_long() {
        let p = r_pentomino();
        let mut h = HashLife::from_pattern(&p);
        h.step(1103);
        let mut u = Universe::from_pattern(&p);
        u.step(1103);
        assert_eq!(h.to_pattern(), u.to_pattern());
        assert_eq!(h.population(), 116);
    }

    #[test]
    fn glider_far() {
        let g = glider(1, 1);
        let mut h = HashLife::from_pattern(&g);
        h.step(4 * 100_000);
        assert_eq!(h.to_pattern(), g.translate(100_000, 100_000));
    }
}
