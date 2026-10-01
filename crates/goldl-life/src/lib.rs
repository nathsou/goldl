//! Game of Life primitives for GoLDL.
//!
//! * [`Pattern`]: a finite set of live cells with RLE I/O and the 8 isometries.
//! * [`Universe`]: a sparse, tiled, bit-parallel B3/S23 engine (64×64 tiles, one `u64` per row)
//!   that only recomputes tiles whose neighbourhood changed in the previous generation, so
//!   still-life circuitry costs nothing per generation.
//! * [`hashlife`]: Gosper's HashLife for very long jumps (ground truth for verification).

pub mod hashlife;
mod universe;

pub use universe::Universe;

use std::collections::HashSet;
use std::fmt::Write as _;

/// A cell coordinate. `y` grows downwards (Golly convention).
pub type Cell = (i64, i64);

/// One of the 8 isometries of the square lattice, applied as `(x, y) -> (a*x + b*y, c*x + d*y)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Iso {
    pub a: i8,
    pub b: i8,
    pub c: i8,
    pub d: i8,
}

impl Iso {
    pub const IDENTITY: Iso = Iso { a: 1, b: 0, c: 0, d: 1 };
    /// Mirror across the vertical axis: x -> -x.
    pub const FLIP_X: Iso = Iso { a: -1, b: 0, c: 0, d: 1 };
    /// Mirror across the horizontal axis: y -> -y.
    pub const FLIP_Y: Iso = Iso { a: 1, b: 0, c: 0, d: -1 };
    /// Rotate 90° clockwise (screen coordinates, y down): (x, y) -> (-y, x).
    pub const ROT_CW: Iso = Iso { a: 0, b: -1, c: 1, d: 0 };
    pub const ROT_180: Iso = Iso { a: -1, b: 0, c: 0, d: -1 };
    pub const ROT_CCW: Iso = Iso { a: 0, b: 1, c: -1, d: 0 };
    /// Transpose: (x, y) -> (y, x).
    pub const TRANSPOSE: Iso = Iso { a: 0, b: 1, c: 1, d: 0 };
    pub const ANTI_TRANSPOSE: Iso = Iso { a: 0, b: -1, c: -1, d: 0 };

    pub const ALL: [Iso; 8] = [
        Iso::IDENTITY,
        Iso::ROT_CW,
        Iso::ROT_180,
        Iso::ROT_CCW,
        Iso::FLIP_X,
        Iso::FLIP_Y,
        Iso::TRANSPOSE,
        Iso::ANTI_TRANSPOSE,
    ];

    #[inline]
    pub fn apply(&self, (x, y): Cell) -> Cell {
        (
            self.a as i64 * x + self.b as i64 * y,
            self.c as i64 * x + self.d as i64 * y,
        )
    }

    /// `self ∘ other` (apply `other` first).
    pub fn compose(&self, o: &Iso) -> Iso {
        Iso {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
        }
    }

    pub fn inverse(&self) -> Iso {
        // Orthogonal matrix: inverse = transpose.
        Iso { a: self.a, b: self.c, c: self.b, d: self.d }
    }
}

/// A finite set of live cells.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pattern {
    pub cells: Vec<Cell>,
}

impl Pattern {
    pub fn new() -> Self {
        Pattern { cells: Vec::new() }
    }

    pub fn from_cells(mut cells: Vec<Cell>) -> Self {
        cells.sort_unstable_by_key(|&(x, y)| (y, x));
        cells.dedup();
        Pattern { cells }
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Inclusive bounding box `(min_x, min_y, max_x, max_y)`.
    pub fn bbox(&self) -> Option<(i64, i64, i64, i64)> {
        let mut it = self.cells.iter();
        let &(x0, y0) = it.next()?;
        let (mut a, mut b, mut c, mut d) = (x0, y0, x0, y0);
        for &(x, y) in it {
            a = a.min(x);
            b = b.min(y);
            c = c.max(x);
            d = d.max(y);
        }
        Some((a, b, c, d))
    }

    pub fn translate(&self, dx: i64, dy: i64) -> Pattern {
        Pattern { cells: self.cells.iter().map(|&(x, y)| (x + dx, y + dy)).collect() }
    }

    pub fn transform(&self, iso: Iso) -> Pattern {
        Pattern::from_cells(self.cells.iter().map(|&c| iso.apply(c)).collect())
    }

    /// Translate so that the bounding box starts at (0, 0).
    pub fn normalized(&self) -> Pattern {
        match self.bbox() {
            Some((x0, y0, _, _)) => Pattern::from_cells(self.translate(-x0, -y0).cells),
            None => Pattern::new(),
        }
    }

    pub fn union(&self, other: &Pattern) -> Pattern {
        let mut v = self.cells.clone();
        v.extend_from_slice(&other.cells);
        Pattern::from_cells(v)
    }

    pub fn to_set(&self) -> HashSet<Cell> {
        self.cells.iter().copied().collect()
    }

    /// Parse an RLE pattern (header line optional; `#` comment lines ignored).
    /// Any state letter other than `b`/`.` is treated as alive.
    pub fn parse_rle(src: &str) -> Result<Pattern, String> {
        let mut cells = Vec::new();
        let (mut x, mut y) = (0i64, 0i64);
        let mut count: i64 = 0;
        'lines: for line in src.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t.starts_with("x ") || t.starts_with("x=") {
                continue;
            }
            for ch in t.chars() {
                match ch {
                    '0'..='9' => count = count * 10 + (ch as i64 - '0' as i64),
                    'b' | '.' => {
                        x += count.max(1);
                        count = 0;
                    }
                    '$' => {
                        y += count.max(1);
                        x = 0;
                        count = 0;
                    }
                    '!' => break 'lines,
                    c if c.is_whitespace() => {}
                    c if c.is_ascii_alphabetic() => {
                        for _ in 0..count.max(1) {
                            cells.push((x, y));
                            x += 1;
                        }
                        count = 0;
                    }
                    c => return Err(format!("unexpected character {c:?} in RLE")),
                }
            }
        }
        Ok(Pattern::from_cells(cells))
    }

    /// Encode as RLE (B3/S23), lines wrapped at 70 characters.
    pub fn to_rle(&self) -> String {
        let Some((x0, y0, x1, y1)) = self.bbox() else {
            return "x = 0, y = 0, rule = B3/S23\n!\n".to_string();
        };
        let mut rows: Vec<Vec<i64>> = vec![Vec::new(); (y1 - y0 + 1) as usize];
        for &(x, y) in &self.cells {
            rows[(y - y0) as usize].push(x - x0);
        }
        let mut body = String::new();
        let mut pending_newlines = 0i64;
        let push_run = |body: &mut String, n: i64, c: char| {
            if n == 1 {
                body.push(c);
            } else if n > 1 {
                let _ = write!(body, "{n}{c}");
            }
        };
        for row in rows.iter_mut() {
            if row.is_empty() {
                pending_newlines += 1;
                continue;
            }
            row.sort_unstable();
            if !body.is_empty() || pending_newlines > 0 {
                if !body.is_empty() {
                    push_run(&mut body, pending_newlines + 1, '$');
                } else {
                    push_run(&mut body, pending_newlines, '$');
                }
            }
            pending_newlines = 0;
            let mut cur = 0i64;
            let mut i = 0;
            while i < row.len() {
                let start = row[i];
                let mut j = i;
                while j + 1 < row.len() && row[j + 1] == row[j] + 1 {
                    j += 1;
                }
                push_run(&mut body, start - cur, 'b');
                push_run(&mut body, row[j] - start + 1, 'o');
                cur = row[j] + 1;
                i = j + 1;
            }
        }
        body.push('!');
        let mut out = format!("x = {}, y = {}, rule = B3/S23\n", x1 - x0 + 1, y1 - y0 + 1);
        let mut line_len = 0;
        let mut token = String::new();
        for ch in body.chars() {
            token.push(ch);
            if !ch.is_ascii_digit() {
                if line_len + token.len() > 70 {
                    out.push('\n');
                    line_len = 0;
                }
                out.push_str(&token);
                line_len += token.len();
                token.clear();
            }
        }
        out.push('\n');
        out
    }

    /// Step this pattern `n` generations with a fresh [`Universe`].
    pub fn run(&self, n: u64) -> Pattern {
        let mut u = Universe::from_pattern(self);
        u.step(n);
        u.to_pattern()
    }
}

/// The canonical glider moving south-east (+1, +1) every 4 generations.
pub fn glider_se() -> Pattern {
    Pattern::from_cells(vec![(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)])
}

/// Glider moving in direction `(dx, dy)` with `dx, dy ∈ {-1, 1}`, phase 0 of the canonical shape.
/// Its 3×3 bounding box has its top-left corner at the origin.
pub fn glider(dx: i64, dy: i64) -> Pattern {
    let g = glider_se();
    let mut iso = Iso::IDENTITY;
    if dx < 0 {
        iso = Iso::FLIP_X.compose(&iso);
    }
    if dy < 0 {
        iso = Iso::FLIP_Y.compose(&iso);
    }
    g.transform(iso).normalized()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_roundtrip() {
        let p = Pattern::parse_rle("x = 3, y = 3\nbo$2bo$3o!").unwrap();
        assert_eq!(p, glider_se());
        let q = Pattern::parse_rle(&p.to_rle()).unwrap();
        assert_eq!(p, q);
    }

    #[test]
    fn glider_moves() {
        for (dx, dy) in [(1, 1), (-1, 1), (1, -1), (-1, -1)] {
            let g = glider(dx, dy);
            let g4 = g.run(4);
            assert_eq!(g4, g.translate(dx, dy), "glider ({dx},{dy})");
        }
    }

    #[test]
    fn iso_group() {
        for a in Iso::ALL {
            assert_eq!(a.compose(&a.inverse()), Iso::IDENTITY);
        }
    }
}
