//! Sparse tiled bit-parallel Life engine.

use crate::{Cell, Pattern};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

const TS: i64 = 64;

/// Tiny multiplicative hasher for `(i32, i32)` keys (no external crates).
#[derive(Default)]
pub struct KeyHasher(u64);
impl Hasher for KeyHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.0 =
            (self.0.rotate_left(5) ^ (i as u32 as u64)).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
pub type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<KeyHasher>>;

#[derive(Clone)]
struct Tile {
    rows: [u64; 64],
    /// Did this tile change during the last generation (or was it just edited)?
    changed: bool,
}

impl Tile {
    fn empty() -> Self {
        Tile {
            rows: [0; 64],
            changed: true,
        }
    }
    fn is_empty(&self) -> bool {
        self.rows.iter().all(|&r| r == 0)
    }
}

/// Sparse Life universe made of 64×64 tiles.
#[derive(Clone)]
pub struct Universe {
    tiles: FastMap<(i32, i32), Tile>,
    pub generation: u64,
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

#[inline]
fn split(v: i64) -> (i32, usize) {
    (v.div_euclid(TS) as i32, v.rem_euclid(TS) as usize)
}

#[inline]
fn full_add(a: u64, b: u64, c: u64) -> (u64, u64) {
    let t = a ^ b;
    (t ^ c, (a & b) | (c & t))
}

/// Next state of a 64-cell row given the rows above (`a`), at (`c`) and below (`b`),
/// with the single neighbouring bits west (`*l`) and east (`*r`) of the word.
#[inline(always)]
pub(crate) fn next_row(
    a: u64,
    al: u64,
    ar: u64,
    c: u64,
    cl: u64,
    cr: u64,
    b: u64,
    bl: u64,
    br: u64,
) -> u64 {
    let aw = (a << 1) | al;
    let ae = (a >> 1) | (ar << 63);
    let cw = (c << 1) | cl;
    let ce = (c >> 1) | (cr << 63);
    let bw = (b << 1) | bl;
    let be = (b >> 1) | (br << 63);
    let (s1, c1) = full_add(aw, a, ae);
    let (s2, c2) = full_add(cw, ce, bw);
    let s3 = b ^ be;
    let c3 = b & be;
    let (ones, c4) = full_add(s1, s2, s3);
    let (t, c5) = full_add(c1, c2, c3);
    let twos = t ^ c4;
    let c6 = t & c4;
    let fours = c5 ^ c6;
    // count == 3, or count == 2 and alive (count 8 aliases to 0: dead either way).
    twos & !fours & (ones | c)
}

impl Universe {
    pub fn new() -> Self {
        Universe {
            tiles: FastMap::default(),
            generation: 0,
        }
    }

    pub fn from_pattern(p: &Pattern) -> Self {
        let mut u = Universe::new();
        u.add_pattern(p);
        u
    }

    pub fn add_pattern(&mut self, p: &Pattern) {
        for &(x, y) in &p.cells {
            self.set(x, y, true);
        }
    }

    /// Insert `p` translated by `(dx, dy)`.
    pub fn add_pattern_at(&mut self, p: &Pattern, dx: i64, dy: i64) {
        for &(x, y) in &p.cells {
            self.set(x + dx, y + dy, true);
        }
    }

    pub fn set(&mut self, x: i64, y: i64, alive: bool) {
        let (tx, bx) = split(x);
        let (ty, by) = split(y);
        let t = self.tiles.entry((tx, ty)).or_insert_with(Tile::empty);
        if alive {
            t.rows[by] |= 1 << bx;
        } else {
            t.rows[by] &= !(1 << bx);
        }
        t.changed = true;
    }

    pub fn get(&self, x: i64, y: i64) -> bool {
        let (tx, bx) = split(x);
        let (ty, by) = split(y);
        self.tiles
            .get(&(tx, ty))
            .is_some_and(|t| t.rows[by] >> bx & 1 == 1)
    }

    pub fn clear(&mut self) {
        self.tiles.clear();
    }

    pub fn population(&self) -> u64 {
        self.tiles
            .values()
            .map(|t| t.rows.iter().map(|r| r.count_ones() as u64).sum::<u64>())
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.values().all(|t| t.is_empty())
    }

    pub fn to_pattern(&self) -> Pattern {
        let mut cells = Vec::new();
        for (&(tx, ty), t) in &self.tiles {
            for (r, &row) in t.rows.iter().enumerate() {
                let mut w = row;
                while w != 0 {
                    let b = w.trailing_zeros() as i64;
                    w &= w - 1;
                    cells.push((tx as i64 * TS + b, ty as i64 * TS + r as i64));
                }
            }
        }
        Pattern::from_cells(cells)
    }

    /// Live cells inside the inclusive rectangle.
    pub fn cells_in(&self, x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<Cell> {
        let mut out = Vec::new();
        let (tx0, _) = split(x0);
        let (ty0, _) = split(y0);
        let (tx1, _) = split(x1);
        let (ty1, _) = split(y1);
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                if let Some(t) = self.tiles.get(&(tx, ty)) {
                    for (r, &row) in t.rows.iter().enumerate() {
                        let y = ty as i64 * TS + r as i64;
                        if y < y0 || y > y1 {
                            continue;
                        }
                        let mut w = row;
                        while w != 0 {
                            let b = w.trailing_zeros() as i64;
                            w &= w - 1;
                            let x = tx as i64 * TS + b;
                            if x >= x0 && x <= x1 {
                                out.push((x, y));
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// Advance `n` generations.
    pub fn step(&mut self, n: u64) {
        for _ in 0..n {
            self.step1();
        }
    }

    fn step1(&mut self) {
        // 1. Make sure every changed tile with live border cells has neighbours to spread into.
        let mut to_create: Vec<(i32, i32)> = Vec::new();
        for (&(tx, ty), t) in &self.tiles {
            if !t.changed {
                continue;
            }
            let top = t.rows[0] != 0;
            let bot = t.rows[63] != 0;
            let mut left = false;
            let mut right = false;
            for &r in &t.rows {
                left |= r & 1 != 0;
                right |= r >> 63 != 0;
            }
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let needed = (dy == -1 && top || dy == 1 && bot || dy == 0)
                        && (dx == -1 && left || dx == 1 && right || dx == 0)
                        && !(dx == 0 && dy == 0);
                    if needed && !self.tiles.contains_key(&(tx + dx, ty + dy)) {
                        to_create.push((tx + dx, ty + dy));
                    }
                }
            }
        }
        for k in to_create {
            self.tiles.entry(k).or_insert_with(|| {
                let mut t = Tile::empty();
                t.changed = true;
                t
            });
        }

        // 2. Candidates: tiles that changed or have a changed neighbour.
        let mut dirty: FastMap<(i32, i32), ()> = FastMap::default();
        for (&(tx, ty), t) in &self.tiles {
            if t.changed {
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let k = (tx + dx, ty + dy);
                        if self.tiles.contains_key(&k) {
                            dirty.insert(k, ());
                        }
                    }
                }
            }
        }

        // 3. Compute next states.
        let empty = [0u64; 64];
        let mut results: Vec<((i32, i32), [u64; 64])> = Vec::with_capacity(dirty.len());
        for &(tx, ty) in dirty.keys() {
            let g = |dx: i32, dy: i32| -> &[u64; 64] {
                self.tiles
                    .get(&(tx + dx, ty + dy))
                    .map(|t| &t.rows)
                    .unwrap_or(&empty)
            };
            let c = g(0, 0);
            let n = g(0, -1);
            let s = g(0, 1);
            let w = g(-1, 0);
            let e = g(1, 0);
            let nw = g(-1, -1);
            let ne = g(1, -1);
            let sw = g(-1, 1);
            let se = g(1, 1);
            let mut rows = [0u64; 66];
            let mut lb = [0u64; 66];
            let mut rb = [0u64; 66];
            rows[0] = n[63];
            lb[0] = nw[63] >> 63;
            rb[0] = ne[63] & 1;
            for r in 0..64 {
                rows[r + 1] = c[r];
                lb[r + 1] = w[r] >> 63;
                rb[r + 1] = e[r] & 1;
            }
            rows[65] = s[0];
            lb[65] = sw[0] >> 63;
            rb[65] = se[0] & 1;
            let mut out = [0u64; 64];
            for r in 0..64 {
                out[r] = next_row(
                    rows[r],
                    lb[r],
                    rb[r],
                    rows[r + 1],
                    lb[r + 1],
                    rb[r + 1],
                    rows[r + 2],
                    lb[r + 2],
                    rb[r + 2],
                );
            }
            results.push(((tx, ty), out));
        }

        // 4. Commit.
        for t in self.tiles.values_mut() {
            t.changed = false;
        }
        for (k, out) in results {
            let t = self.tiles.get_mut(&k).unwrap();
            if t.rows != out {
                t.rows = out;
                t.changed = true;
            }
        }
        self.tiles.retain(|_, t| t.changed || !t.is_empty());
        self.generation += 1;
    }

    /// Number of tiles that will be recomputed next generation (activity measure).
    pub fn active_tiles(&self) -> usize {
        self.tiles.values().filter(|t| t.changed).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glider;

    #[test]
    fn blinker() {
        let mut u = Universe::new();
        for x in 0..3 {
            u.set(63 + x, 10, true); // straddles a tile boundary
        }
        u.step(1);
        let p = u.to_pattern();
        assert_eq!(p.cells, vec![(64, 9), (64, 10), (64, 11)]);
        u.step(1);
        assert_eq!(u.to_pattern().cells, vec![(63, 10), (64, 10), (65, 10)]);
    }

    #[test]
    fn glider_crosses_tiles() {
        let g = glider(-1, -1).translate(5, 5);
        let mut u = Universe::from_pattern(&g);
        u.step(400);
        assert_eq!(u.to_pattern(), g.translate(-100, -100));
    }

    #[test]
    fn still_life_is_free() {
        let block = Pattern::from_cells(vec![(0, 0), (1, 0), (0, 1), (1, 1)]);
        let mut u = Universe::from_pattern(&block);
        u.step(3);
        assert_eq!(u.active_tiles(), 0);
        assert_eq!(u.to_pattern(), block);
    }
}
