//! Place & route: GNL → physical Life circuitry ([`Phys`]).
//!
//! # Geometry
//! The fabric is a grid of square cells ("gcs") in rotated coordinates `u = x + y`,
//! `v = x − y`, pitch [`G`]. Gc `(i, j)` has a *row track* (a SE lane, `x − y = j·G`) and a
//! *column track* (a NE lane, `x + y = i·G`). All logic gliders travel east (SE or NE).
//!
//! # Timing discipline
//! A glider's eastbound wave phase `φ = t − 4x` only changes inside components. Every glider
//! is kept in one of two phase classes (mod 86): rows at `φ ≡ 0`, columns at `φ ≡ 43`, so
//! *any* row/column crossing is separated by ≥ 43 generations and is safe: routing is purely
//! geometric. A routing turn ("class turn") is a duplicator (+37, spare output eaten)
//! followed by a colour-changing reflector (+6): exactly +43.
//!
//! A crossing node takes two row gliders with *equal* phase on adjacent rows `j` (a) and
//! `j−1` (b): b turns up with a bare reflector (+6) and meets a with Δφ = 6, a clean vanish.
//! Phases are tracked as indices `p = φ / 43`; the inputs of a crossing are equalised by
//! routing the earlier one through extra turn pairs.

use crate::gnl::{Gnl, NetId, NodeId, Op};
use crate::phys::{CrossBook, CrossSite, Inst, Leg, Phys, Region, SigKind};
use crate::tech::component::{find, place_on, place_turn, Kind, Placed};
use crate::tech::glider::{Dir, Traj};
use crate::tech::path::{comp_index, LegSpec, PathBuilder};
use goldl_life::Universe;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::OnceLock;

/// Grid pitch in rotated units.
pub const G: i64 = 128;
/// Phase quantum: one class turn.
pub const PH: i64 = 43;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Track {
    Row,
    Col,
}

/// A glider entering gc `(i, j)` on a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct St {
    pub i: i32,
    pub j: i32,
    pub t: Track,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    Straight,
    Turn,
}

pub fn row_lane(j: i32) -> i64 {
    j as i64 * G
}
pub fn col_lane(i: i32) -> i64 {
    -(i as i64) * G
}
/// xy coordinates of the centre of gc (i, j).
pub fn gc_xy(i: i32, j: i32) -> (i64, i64) {
    let (u, v) = (i as i64 * G, j as i64 * G);
    ((u + v) / 2, (u - v) / 2)
}
/// gc containing an xy point.
pub fn xy_gc(x: i64, y: i64) -> (i32, i32) {
    let (u, v) = (x + y, x - y);
    ((u as f64 / G as f64).round() as i32, (v as f64 / G as f64).round() as i32)
}
fn gc_offset_xy(di: i32, dj: i32) -> (i64, i64) {
    gc_xy(di, dj)
}

fn step(s: St, m: Move) -> St {
    match (s.t, m) {
        (Track::Row, Move::Straight) => St { i: s.i + 1, ..s },
        (Track::Col, Move::Straight) => St { j: s.j + 1, ..s },
        (Track::Row, Move::Turn) => St { i: s.i + 1, j: s.j + 1, t: Track::Col },
        (Track::Col, Move::Turn) => St { i: s.i + 1, j: s.j + 1, t: Track::Row },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Use {
    Row,
    Col,
    Full,
}

/// gcs used by a move from state s.
fn move_cells(s: St, m: Move) -> [((i32, i32), Use); 2] {
    match (s.t, m) {
        (Track::Row, Move::Straight) => [((s.i, s.j), Use::Row), ((s.i, s.j), Use::Row)],
        (Track::Col, Move::Straight) => [((s.i, s.j), Use::Col), ((s.i, s.j), Use::Col)],
        (Track::Row, Move::Turn) => [((s.i, s.j), Use::Full), ((s.i + 1, s.j), Use::Full)],
        (Track::Col, Move::Turn) => [((s.i, s.j), Use::Full), ((s.i, s.j + 1), Use::Full)],
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct GcState {
    row: bool,
    col: bool,
    full: bool,
}

#[derive(Default)]
struct Grid {
    cells: HashMap<(i32, i32), GcState>,
    s_min: i32,
    s_max: i32,
    d_min: i32,
    d_max: i32,
}

impl Grid {
    fn get(&self, c: (i32, i32)) -> GcState {
        self.cells.get(&c).copied().unwrap_or_default()
    }
    fn in_bounds(&self, c: (i32, i32)) -> bool {
        let s = c.0 + c.1;
        let d = c.0 - c.1;
        s >= self.s_min && s <= self.s_max && d >= self.d_min && d <= self.d_max
    }
    fn can(&self, c: (i32, i32), u: Use) -> bool {
        if !self.in_bounds(c) {
            return false;
        }
        let g = self.get(c);
        if g.full {
            return false;
        }
        match u {
            Use::Row => !g.row,
            Use::Col => !g.col,
            Use::Full => !g.row && !g.col,
        }
    }
    fn set(&mut self, c: (i32, i32), u: Use, on: bool) {
        let g = self.cells.entry(c).or_default();
        match u {
            Use::Row => g.row = on,
            Use::Col => g.col = on,
            Use::Full => g.full = on,
        }
    }
}

/// A* search from `from` to `to`. If `turns` is `Some(k)`, the path must contain exactly `k`
/// class turns.
fn route(grid: &Grid, from: St, to: St, turns: Option<u32>) -> Option<Vec<Move>> {
    if to.i < from.i || to.j < from.j {
        return None;
    }
    let max_turns = turns.unwrap_or(1000);
    type Key = (St, u32);
    let h = |s: St| ((to.i - s.i) + (to.j - s.j)) as u32;
    let mut best: HashMap<Key, u32> = HashMap::new();
    let mut prev: HashMap<Key, (Key, Move)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    let start = (from, 0u32);
    best.insert(start, 0);
    heap.push(std::cmp::Reverse((h(from), 0u32, start)));
    let mut expansions = 0usize;
    while let Some(std::cmp::Reverse((_, g, key))) = heap.pop() {
        if best.get(&key).copied() != Some(g) {
            continue;
        }
        let (s, k) = key;
        if s == to && turns.map_or(true, |t| t == k) {
            let mut moves = Vec::new();
            let mut cur = key;
            while let Some(&(p, m)) = prev.get(&cur) {
                moves.push(m);
                cur = p;
            }
            moves.reverse();
            return Some(moves);
        }
        expansions += 1;
        if expansions > 300_000 {
            return None;
        }
        for m in [Move::Straight, Move::Turn] {
            let nk = if m == Move::Turn { k + 1 } else { k };
            if nk > max_turns {
                continue;
            }
            let ns = step(s, m);
            if ns.i > to.i || ns.j > to.j {
                continue;
            }
            if let Some(t) = turns {
                let rem = t - nk;
                let dist = ((to.i - ns.i) + (to.j - ns.j)) as u32;
                if rem * 2 > dist {
                    continue;
                }
                // Direction parity: remaining turns must be able to end on the right track.
                let same = ns.t == to.t;
                if same != (rem % 2 == 0) && rem == 0 {
                    continue;
                }
            }
            if !move_cells(s, m).iter().all(|&(c, u)| grid.can(c, u)) {
                continue;
            }
            let cost = g + if m == Move::Turn { 5 } else { 1 };
            let nkey = (ns, nk);
            if best.get(&nkey).map_or(true, |&b| cost < b) {
                best.insert(nkey, cost);
                prev.insert(nkey, (key, m));
                heap.push(std::cmp::Reverse((cost + h(ns), cost, nkey)));
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------------------
// Fragments (macro templates)
// ---------------------------------------------------------------------------------------

/// A piece of circuitry with entry/exit ports, built for anchor gc (0, 0) and translated.
#[derive(Clone, Debug)]
struct Fragment {
    /// Input ports: state, glider trajectory at entry, entry time.
    ins: Vec<(St, Traj, i64)>,
    outs: Vec<(St, Traj, i64)>,
    /// Components and legs, tagged with a role (0.. = inputs, 100+k = output k).
    placed: Vec<(Placed, u8)>,
    legs: Vec<(LegSpec, u8)>,
    footprint: Vec<(i32, i32)>,
    /// gcs just before an input port that the input glider passes straight through: only the
    /// other track is blocked there.
    approach: Vec<(i32, i32, Track)>,
    /// Crossing sites: (translation, t_contact, t_end) — roles 0 and 1.
    crosses: Vec<(i64, i64, i64, i64)>,
}

impl Fragment {
    /// Translate by whole gcs and shift time by `dt`.
    fn moved(&self, di: i32, dj: i32, dt: i64) -> Fragment {
        let (dx, dy) = gc_offset_xy(di, dj);
        let tr = |t: Traj| crate::tech::component::translate(t, dx, dy).delayed(dt);
        let mv = |s: St| St { i: s.i + di, j: s.j + dj, t: s.t };
        Fragment {
            ins: self.ins.iter().map(|&(s, t, e)| (mv(s), tr(t), e + dt)).collect(),
            outs: self.outs.iter().map(|&(s, t, e)| (mv(s), tr(t), e + dt)).collect(),
            placed: self.placed.iter().map(|&(p, r)| (Placed { tx: p.tx + dx, ty: p.ty + dy, dt: p.dt + dt, ..p }, r)).collect(),
            legs: self.legs.iter().map(|&(l, r)| (LegSpec { traj: tr(l.traj), t0: l.t0 + dt, t1: l.t1 + dt }, r)).collect(),
            footprint: self.footprint.iter().map(|&(i, j)| (i + di, j + dj)).collect(),
            approach: self.approach.iter().map(|&(i, j, t)| (i + di, j + dj, t)).collect(),
            crosses: self.crosses.iter().map(|&(x, y, a, b)| (x + dx, y + dy, a + dt, b + dt)).collect(),
        }
    }
}

fn port_traj(s: St, phase: i64) -> Traj {
    match s.t {
        Track::Row => Traj { dir: Dir::SE, lane: row_lane(s.j), phi: phase * PH },
        Track::Col => Traj { dir: Dir::NE, lane: col_lane(s.i), phi: phase * PH },
    }
}

/// Time at which a glider on `tr` reaches the entry edge of gc state `s`.
fn t_at_entry(tr: Traj, s: St) -> i64 {
    let (cx, _) = gc_xy(s.i, s.j);
    tr.time_at_x(cx - G / 4)
}

/// gcs overlapped by a bounding box (with margin).
fn bbox_gcs(b: (i64, i64, i64, i64), margin: i64) -> Vec<(i32, i32)> {
    let (x0, y0, x1, y1) = (b.0 - margin, b.1 - margin, b.2 + margin, b.3 + margin);
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    let us: Vec<i64> = corners.iter().map(|&(x, y)| x + y).collect();
    let vs: Vec<i64> = corners.iter().map(|&(x, y)| x - y).collect();
    let (umin, umax) = (*us.iter().min().unwrap(), *us.iter().max().unwrap());
    let (vmin, vmax) = (*vs.iter().min().unwrap(), *vs.iter().max().unwrap());
    let gi = |u: i64| ((u as f64 / G as f64) + 0.5).floor() as i32;
    let half = G / 2;
    let in_diamond = |cx: i64, cy: i64, x: i64, y: i64| -> bool {
        let (a, b) = ((x - cx) + (y - cy), (x - cx) - (y - cy));
        a.abs() <= half && b.abs() <= half
    };
    let mut v = Vec::new();
    for i in gi(umin)..=gi(umax) {
        for j in gi(vmin)..=gi(vmax) {
            let (cx, cy) = gc_xy(i, j);
            let mut hit = false;
            // sample the box densely enough (boxes are small)
            let mut x = x0;
            while x <= x1 && !hit {
                let mut y = y0;
                while y <= y1 {
                    if in_diamond(cx, cy, x, y) {
                        hit = true;
                        break;
                    }
                    y += 4;
                }
                x += 4;
            }
            for &(x, y) in &[(x1, y0), (x0, y1), (x1, y1)] {
                if in_diamond(cx, cy, x, y) {
                    hit = true;
                }
            }
            if hit {
                v.push((i, j));
            }
        }
    }
    v
}

/// gcs a leg passes through.
fn leg_gcs(l: &LegSpec) -> Vec<(i32, i32)> {
    let mut v = Vec::new();
    let mut t = l.t0;
    while t < l.t1 {
        let (x, y) = l.traj.pos_at(t);
        let c = xy_gc(x as i64 + 1, y as i64 + 1);
        if v.last() != Some(&c) {
            v.push(c);
        }
        t += 16;
    }
    v
}

/// From a glider at time `t`, walk forward along its track until leaving the footprint;
/// returns the port state (first gc after the current one that is not in the footprint).
fn exit_port(tr: Traj, t: i64, fp: &HashSet<(i32, i32)>) -> St {
    let track = if tr.dir == Dir::SE { Track::Row } else { Track::Col };
    let (x, y) = tr.pos_at(t);
    let (mut i, mut j) = xy_gc(x as i64 + 1, y as i64 + 1);
    // snap the cross-track coordinate to the lane's track
    match track {
        Track::Row => j = (tr.lane as f64 / G as f64).round() as i32,
        Track::Col => i = (-tr.lane as f64 / G as f64).round() as i32,
    }
    loop {
        match track {
            Track::Row => i += 1,
            Track::Col => j += 1,
        }
        if !fp.contains(&(i, j)) {
            return St { i, j, t: track };
        }
    }
}

fn finish_fragment(mut f: Fragment, outs_builders: Vec<(PathBuilder, u8)>, extra_fp: &[(i32, i32)]) -> Fragment {
    let mut fp: HashSet<(i32, i32)> = extra_fp.iter().copied().collect();
    for (b, role) in &outs_builders {
        for l in &b.legs {
            f.legs.push((*l, *role));
        }
        for p in &b.placed {
            f.placed.push((*p, *role));
        }
    }
    for (p, _) in &f.placed {
        for c in bbox_gcs(p.bbox(), 4) {
            fp.insert(c);
        }
    }
    for (l, _) in &f.legs {
        for c in leg_gcs(l) {
            fp.insert(c);
        }
    }
    let mut ports = Vec::new();
    for (b, role) in &outs_builders {
        let port = exit_port(b.cur, b.t_cur, &fp);
        let te = t_at_entry(b.cur, port);
        assert!(te >= b.t_cur, "port entry before glider is free");
        let leg = LegSpec { traj: b.cur, t0: b.t_cur, t1: te };
        ports.push((port, b.cur, te, leg, *role));
    }
    for (port, cur, te, leg, role) in ports {
        for c in leg_gcs(&leg) {
            if c != (port.i, port.j) {
                fp.insert(c);
            }
        }
        f.legs.push((leg, role));
        f.outs.push((port, cur, te));
    }
    for &(s, _, _) in &f.ins.clone() {
        let a = match s.t {
            Track::Row => (s.i - 1, s.j),
            Track::Col => (s.i, s.j - 1),
        };
        if fp.remove(&a) {
            f.approach.push((a.0, a.1, s.t));
        }
    }
    let mut v: Vec<(i32, i32)> = fp.into_iter().collect();
    v.sort();
    f.footprint = v;
    f
}

/// Eat side outputs of duplicators (pushes eater paths into the fragment).
fn eat_sides(f: &mut Fragment, b: &PathBuilder, role: u8) {
    for &(side, ts) in &b.side {
        let mut sb = PathBuilder::new(side, ts);
        let (x, y) = side.pos_at(ts + 28);
        let k = sb.k_near(Kind::Eater, None, x as i64, y as i64);
        sb.eat(k);
        for l in &sb.legs {
            f.legs.push((*l, role));
        }
        for p in &sb.placed {
            f.placed.push((*p, role));
        }
    }
}

fn push_builder(f: &mut Fragment, b: &PathBuilder, role: u8) {
    for l in &b.legs {
        f.legs.push((*l, role));
    }
    for p in &b.placed {
        f.placed.push((*p, role));
    }
}

/// Usage of a crossing's outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum OutUse {
    Used,
    Eat,
    Zero,
}

/// Crossing fragment: a enters (0,0) on row 0, b enters (0,-1) on row -1, both phase 0.
fn cross_fragment(ua: OutUse, ub: OutUse) -> Fragment {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<(u8, u8), Fragment>>> = OnceLock::new();
    let key = (ua as u8, ub as u8);
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap().get(&key) {
        return f.clone();
    }
    let sa = St { i: 0, j: 0, t: Track::Row };
    let sb = St { i: 0, j: -1, t: Track::Row };
    let ta = port_traj(sa, 0);
    let tbr = port_traj(sb, 0);
    let ea = t_at_entry(ta, sa);
    let eb = t_at_entry(tbr, sb);
    let mut f = Fragment { ins: vec![(sa, ta, ea), (sb, tbr, eb)], outs: vec![], placed: vec![], legs: vec![], footprint: vec![], approach: vec![], crosses: vec![] };
    let mut bb = PathBuilder::new(tbr, eb);
    bb.turn(Kind::Cc, Dir::NE, col_lane(0)).expect("cross b turn");
    let tb = bb.cur;
    assert_eq!(tb.phi - ta.phi, 6);
    let (_, btc, bte) = *cross_book();
    let tx = (ta.lane - tb.lane) / 2;
    let ty = (-ta.lane - tb.lane) / 2;
    let dt = ta.phi + 4 * tx;
    let tc = btc + dt;
    let te = bte + dt;
    let mut ba = PathBuilder::new(ta, ea);
    ba.finish(tc);
    bb.finish(tc);
    push_builder(&mut f, &ba, 0);
    push_builder(&mut f, &bb, 1);
    f.crosses.push((tx, ty, tc, te));
    let mut outs = Vec::new();
    let mut extra_fp = vec![(0, -1), (0, 0)];
    match ua {
        OutUse::Used => outs.push((PathBuilder::new(ta, tc), 100)),
        OutUse::Eat => {
            let mut b = PathBuilder::new(ta, tc);
            let (x, y) = ta.pos_at(tc + 4 * 24);
            let k = b.k_near(Kind::Eater, None, x as i64, y as i64);
            b.eat(k);
            push_builder(&mut f, &b, 100);
        }
        OutUse::Zero => {}
    }
    match ub {
        OutUse::Used => {
            let mut b = PathBuilder::new(tb, tc);
            let (cx, cy) = gc_xy(0, 1);
            let k = b.k_near(Kind::Dup, Some(Dir::NE), cx, cy);
            b.dup(k).expect("b' dup");
            eat_sides(&mut f, &b, 101);
            b.side.clear();
            extra_fp.push((0, 1));
            outs.push((b, 101));
        }
        OutUse::Eat => {
            let mut b = PathBuilder::new(tb, tc);
            let (x, y) = tb.pos_at(tc + 4 * 24);
            let k = b.k_near(Kind::Eater, None, x as i64, y as i64);
            b.eat(k);
            push_builder(&mut f, &b, 101);
        }
        OutUse::Zero => {}
    }
    let f = finish_fragment(f, outs, &extra_fp);
    cache.lock().unwrap().insert(key, f.clone());
    f
}

/// Split fragment: input enters (0,0) on row 0 at phase 0. Outputs: column (via the main
/// duplicator output) and row (via the side output).
pub fn debug_split() {
    let f = split_fragment();
    eprintln!("split outs {:?} footprint {:?} approach {:?}", f.outs, f.footprint, f.approach);
    let c = cross_fragment(OutUse::Used, OutUse::Used);
    eprintln!("cross outs {:?} footprint {:?} approach {:?}", c.outs, c.footprint, c.approach);
}

fn split_fragment() -> &'static Fragment {
    static F: OnceLock<Fragment> = OnceLock::new();
    F.get_or_init(|| {
        let s0 = St { i: 0, j: 0, t: Track::Row };
        let t0 = port_traj(s0, 0);
        let e0 = t_at_entry(t0, s0);
        let mut best: Option<(i64, Fragment)> = None;
        for dshift in [24i64, 16, 32, 8, 40] {
            let mut b = PathBuilder::new(t0, e0);
            let (cx, cy) = gc_xy(0, 0);
            let k = b.k_near(Kind::Dup, Some(Dir::SE), cx + dshift, cy + dshift);
            b.dup(k).expect("split dup");
            let dup_bb = b.placed[0].bbox();
            let (side, ts) = b.side[0];
            b.side.clear();
            // out1: CC up onto column 1.
            let mut b1 = PathBuilder::new(b.cur, b.t_cur);
            if b1.turn(Kind::Cc, Dir::NE, col_lane(1)).is_none() {
                continue;
            }
            'search: for (k1, k2, row) in [
                (Kind::Cc, Kind::Cc, 2),
                (Kind::Snark, Kind::Snark, 1),
                (Kind::Snark, Kind::Snark, 2),
                (Kind::Snark, Kind::Cc, 2),
                (Kind::Cc, Kind::Snark, 2),
            ] {
                let (o_up, i_up) = find(k1, side.dir, Some(Dir::NE)).unwrap();
                let (o_r, i_r) = find(k2, Dir::NE, Some(Dir::SE)).unwrap();
                for kk in -300..300 {
                    let p = place_on(o_up, side, kk);
                    if p.t_contact() < ts + 20 {
                        continue;
                    }
                    if boxes_close(p.bbox(), dup_bb, 3) {
                        continue;
                    }
                    let up = p.output(i_up);
                    // Optional parity-fixing duplicator on the vertical segment.
                    let mut mid = PathBuilder::new(up, p.t_settled());
                    let (ux, uy) = up.pos_at(p.t_settled() + 4 * 30);
                    let kd = mid.k_near(Kind::Dup, Some(Dir::NE), ux as i64, uy as i64);
                    if mid.dup(kd).is_none() {
                        continue;
                    }
                    let dupv = mid.placed[0];
                    if boxes_close(dupv.bbox(), p.bbox(), 3) || boxes_close(dupv.bbox(), dup_bb, 3) || boxes_close(dupv.bbox(), b1.placed[0].bbox(), 3) {
                        continue;
                    }
                    let up2 = mid.cur;
                    let Some(p2) = place_turn(o_r, up2, i_r, row_lane(row)) else { continue };
                    if boxes_close(p2.bbox(), dup_bb, 3) || boxes_close(p2.bbox(), p.bbox(), 3) || boxes_close(p2.bbox(), dupv.bbox(), 3) {
                        continue;
                    }
                    if boxes_close(p2.bbox(), b1.placed[0].bbox(), 3) || boxes_close(p.bbox(), b1.placed[0].bbox(), 3) {
                        continue;
                    }
                    let fin = p2.output(i_r);
                    if fin.phi.rem_euclid(2 * PH) != 0 {
                        continue;
                    }
                    let pv_min = {
                        let (a, bq, c, d) = p.bbox();
                        [(a, bq), (c, bq), (a, d), (c, d)].iter().map(|&(x, y)| x - y).min().unwrap()
                    };
                    if pv_min < 12 {
                        continue;
                    }
                    let mut f = Fragment { ins: vec![(s0, t0, e0)], outs: vec![], placed: vec![], legs: vec![], footprint: vec![], approach: vec![], crosses: vec![] };
                    f.legs.push((b.legs[0], 0));
                    f.placed.push((b.placed[0], 0));
                    let mut s2 = PathBuilder::new(side, ts);
                    s2.legs.push(LegSpec { traj: side, t0: ts, t1: p.t_contact() });
                    s2.placed.push(p);
                    s2.legs.push(LegSpec { traj: up, t0: p.t_settled(), t1: dupv.t_contact() });
                    s2.placed.push(dupv);
                    s2.legs.push(LegSpec { traj: up2, t0: dupv.t_settled(), t1: p2.t_contact() });
                    s2.placed.push(p2);
                    s2.cur = fin;
                    s2.t_cur = p2.t_settled();
                    eat_sides(&mut f, &mid, 101);
                    let delay = fin.phi;
                    let f = finish_fragment(f, vec![(b1.clone(), 100), (s2, 101)], &[(0, 0)]);
                    if best.as_ref().map_or(true, |(d, _)| delay < *d) {
                        best = Some((delay, f));
                    }
                    break 'search;
                }
            }
            if best.is_some() {
                break;
            }
        }
        best.expect("split fragment").1
    })
}

fn boxes_close(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64), m: i64) -> bool {
    a.0 - m <= b.2 && b.0 - m <= a.2 && a.1 - m <= b.3 && b.1 - m <= a.3
}

/// Canonical crossing flipbook (SE lane 0 φ 0, NE lane 0 φ 6): (book, t_contact, t_end).
fn cross_book() -> &'static (CrossBook, i64, i64) {
    static B: OnceLock<(CrossBook, i64, i64)> = OnceLock::new();
    B.get_or_init(|| {
        let a = Traj { dir: Dir::SE, lane: 0, phi: 0 };
        let b = Traj { dir: Dir::NE, lane: 0, phi: 6 };
        let t_start = -100;
        let mut u = Universe::from_pattern(&a.pattern_at(t_start).union(&b.pattern_at(t_start)));
        let mut frames = Vec::new();
        let mut t_c = None;
        let mut t = t_start;
        loop {
            let cur = u.to_pattern();
            let free = a.pattern_at(t).union(&b.pattern_at(t));
            if t_c.is_none() && cur != free {
                t_c = Some(t);
            }
            if t_c.is_some() {
                if cur.is_empty() {
                    break;
                }
                frames.push(cur.cells);
            }
            u.step(1);
            t += 1;
            assert!(t < 300, "crossing must vanish");
        }
        (CrossBook { frames }, t_c.unwrap(), t)
    })
}

// ---------------------------------------------------------------------------------------
// Layout driver
// ---------------------------------------------------------------------------------------

/// Result of layout, plus useful statistics.
pub struct LayoutResult {
    pub phys: Phys,
    /// Phase index (φ / 43) of every net at its driver.
    pub phase: Vec<i64>,
    pub grid_cells: usize,
}

fn net_used(g: &Gnl, n: NetId) -> bool {
    let net = &g.nets[n as usize];
    !net.always_zero && net.sink.is_some_and(|s| g.nodes[s.node as usize].op != Op::Sink)
}

fn out_use(g: &Gnl, n: NetId) -> OutUse {
    if g.nets[n as usize].always_zero {
        OutUse::Zero
    } else if net_used(g, n) {
        OutUse::Used
    } else {
        OutUse::Eat
    }
}

/// Node instance: its fragment (already translated) or source/sink ports.
#[derive(Clone, Debug)]
enum NodeImpl {
    Frag(Fragment),
    Source { port: St },
    Sink { port: St },
    Virtual,
}

/// Lay out a GNL.
pub fn layout(g: &Gnl) -> Result<LayoutResult, String> {
    let mut last_err = String::new();
    for spacing in [8, 12, 17, 24, 34] {
        match layout_with(g, spacing) {
            Ok(r) => return Ok(r),
            Err(e) => last_err = e,
        }
    }
    Err(format!("layout failed: {last_err}"))
}

fn levels(g: &Gnl) -> Vec<i32> {
    let order = g.topo();
    let mut lv = vec![0i32; g.nodes.len()];
    for &n in &order {
        let node = &g.nodes[n as usize];
        let mut l = 0;
        for &i in &node.ins {
            let d = g.nets[i as usize].driver.node;
            l = l.max(lv[d as usize] + 1);
        }
        lv[n as usize] = l;
    }
    lv
}

fn layout_with(g: &Gnl, spacing: i32) -> Result<LayoutResult, String> {
    let n_nodes = g.nodes.len();
    let lv = levels(g);
    let max_lv = (0..n_nodes)
        .filter(|&i| !matches!(g.nodes[i].op, Op::Out { .. } | Op::RegD { .. } | Op::Sink))
        .map(|i| lv[i])
        .max()
        .unwrap_or(0);
    let sink_level = max_lv + 1;

    // ---- Placement in (s = i + j, d = i - j) ----
    let lat = 8;
    let mut pos: Vec<Option<(i32, i32)>> = vec![None; n_nodes];
    let mut by_level: Vec<Vec<NodeId>> = vec![Vec::new(); (sink_level + 1) as usize];
    for n in 0..n_nodes {
        let op = &g.nodes[n].op;
        if *op == Op::Sink {
            continue;
        }
        let l = if matches!(op, Op::Out { .. } | Op::RegD { .. }) { sink_level } else { lv[n] };
        by_level[l as usize].push(n as NodeId);
    }
    {
        let mut srcs = by_level[0].clone();
        srcs.sort_by_key(|&n| match g.nodes[n as usize].op {
            Op::RegQ { reg, bit } => (0, reg, bit),
            Op::In { port, bit } => (1, port, bit),
            _ => (2, 0, 0),
        });
        for (k, &n) in srcs.iter().enumerate() {
            pos[n as usize] = Some((0, k as i32 * lat));
        }
    }
    for l in 1..by_level.len() {
        let s = l as i32 * spacing;
        let mut keyed: Vec<(f64, u32, NodeId)> = by_level[l]
            .iter()
            .map(|&n| {
                let node = &g.nodes[n as usize];
                let ds: Vec<f64> = node
                    .ins
                    .iter()
                    .filter_map(|&i| pos[g.nets[i as usize].driver.node as usize])
                    .map(|p| p.1 as f64)
                    .collect();
                let bc = if ds.is_empty() { 0.0 } else { ds.iter().sum::<f64>() / ds.len() as f64 };
                (bc, node.group, n)
            })
            .collect();
        if l as i32 == sink_level {
            // Registers in Q order first (top), then outputs.
            keyed.sort_by_key(|k| match g.nodes[k.2 as usize].op {
                Op::RegD { reg, bit } => (0, reg, bit, 0),
                Op::Out { port, bit } => (1, port, bit, 0),
                _ => (2, 0, 0, 0),
            });
        } else {
            keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(a.1.cmp(&b.1)));
        }
        let mut last_d = i32::MIN / 2;
        for &(bc, _, n) in &keyed {
            let mut d = if l as i32 == sink_level { last_d + lat } else { bc.round() as i32 };
            if l as i32 == sink_level && last_d == i32::MIN / 2 {
                d = -lat * 2;
            }
            if d < last_d + lat {
                d = last_d + lat;
            }
            if (s - d).rem_euclid(2) != 0 {
                d += 1;
            }
            pos[n as usize] = Some((s, d));
            last_d = d;
        }
    }
    let to_ij = |p: (i32, i32)| -> (i32, i32) { ((p.0 + p.1) / 2, (p.0 - p.1) / 2) };

    // ---- Instantiate node geometry ----
    let mut grid = Grid::default();
    let mut imps: Vec<NodeImpl> = vec![NodeImpl::Virtual; n_nodes];
    let (mut d_lo, mut d_hi) = (i32::MAX, i32::MIN);
    for n in 0..n_nodes {
        let Some(p) = pos[n] else { continue };
        d_lo = d_lo.min(p.1);
        d_hi = d_hi.max(p.1);
        let (i, j) = to_ij(p);
        let node = &g.nodes[n];
        let imp = match node.op {
            Op::Cross => {
                let f = cross_fragment(out_use(g, node.outs[0]), out_use(g, node.outs[1]));
                NodeImpl::Frag(f.moved(i, j, 0))
            }
            Op::Split => NodeImpl::Frag(split_fragment().moved(i, j, 0)),
            Op::In { .. } | Op::One => NodeImpl::Source { port: St { i, j, t: Track::Col } },
            Op::RegQ { .. } => NodeImpl::Source { port: St { i, j, t: Track::Row } },
            Op::Out { .. } | Op::RegD { .. } => NodeImpl::Sink { port: St { i, j, t: Track::Row } },
            Op::Delay | Op::Sink => NodeImpl::Virtual,
        };
        match &imp {
            NodeImpl::Frag(f) => {
                for &c in &f.footprint {
                    grid.set(c, Use::Full, true);
                }
                for &(i, j, t) in &f.approach {
                    grid.set((i, j), if t == Track::Row { Use::Col } else { Use::Row }, true);
                }
            }
            NodeImpl::Sink { port } => {
                grid.set((port.i, port.j), Use::Full, true);
                grid.set((port.i + 1, port.j), Use::Full, true);
            }
            _ => {}
        }
        imps[n] = imp;
    }
    grid.s_min = -2;
    grid.s_max = sink_level * spacing + 6;
    grid.d_min = d_lo - 2 * spacing - 10;
    grid.d_max = d_hi + 2 * spacing + 10;

    // Port lookup.
    let out_port = |imps: &Vec<NodeImpl>, net: NetId| -> St {
        let drv = g.nets[net as usize].driver;
        match &imps[drv.node as usize] {
            NodeImpl::Frag(f) => f.outs[drv.port as usize].0,
            NodeImpl::Source { port } => *port,
            _ => panic!("no out port"),
        }
    };
    let in_port = |imps: &Vec<NodeImpl>, n: NodeId, k: usize| -> St {
        match &imps[n as usize] {
            NodeImpl::Frag(f) => f.ins[k].0,
            NodeImpl::Sink { port } => *port,
            _ => panic!("no in port"),
        }
    };

    // ---- Routing ----
    let order = g.topo();
    let mut routes: Vec<Option<(St, Vec<Move>)>> = vec![None; g.nets.len()];
    let mut p_start: Vec<i64> = vec![0; g.nets.len()];
    let mut p_sink: Vec<i64> = vec![0; g.nets.len()];
    let split_p = {
        let f = split_fragment();
        (f.outs[0].1.phi / PH, f.outs[1].1.phi / PH)
    };
    let mark = |grid: &mut Grid, from: St, moves: &[Move], on: bool| {
        let mut s = from;
        for &m in moves {
            for (c, u) in move_cells(s, m) {
                grid.set(c, u, on);
            }
            s = step(s, m);
        }
    };
    for &n in &order {
        let node = &g.nodes[n as usize];
        match node.op {
            Op::In { .. } | Op::One => p_start[node.outs[0] as usize] = 1,
            Op::RegQ { .. } => p_start[node.outs[0] as usize] = 0,
            _ => {}
        }
        if node.op == Op::Sink || node.op == Op::Delay || node.ins.is_empty() {
            if node.op == Op::Delay {
                return Err("Delay nodes are not supported by this back end".into());
            }
            continue;
        }
        let mut phases = Vec::new();
        for (k, &net) in node.ins.iter().enumerate() {
            let from = out_port(&imps, net);
            let to = in_port(&imps, n, k);
            let moves = route(&grid, from, to, None).ok_or_else(|| format!("cannot route net {net} into node {n} ({:?}) from {from:?} to {to:?}", node.op))?;
            mark(&mut grid, from, &moves, true);
            let turns = moves.iter().filter(|&&m| m == Move::Turn).count() as i64;
            phases.push(p_start[net as usize] + turns);
            routes[net as usize] = Some((from, moves));
        }
        if node.op == Op::Cross && phases[0] != phases[1] {
            let (k, target, other) = if phases[0] < phases[1] { (0, phases[1], phases[0]) } else { (1, phases[0], phases[1]) };
            let net = node.ins[k];
            let (from, moves) = routes[net as usize].take().unwrap();
            mark(&mut grid, from, &moves, false);
            let base = moves.iter().filter(|&&m| m == Move::Turn).count() as i64;
            let want = (base + target - other) as u32;
            let to = in_port(&imps, n, k);
            match route(&grid, from, to, Some(want)) {
                Some(m2) => {
                    mark(&mut grid, from, &m2, true);
                    routes[net as usize] = Some((from, m2));
                    phases[k] = target;
                }
                None => {
                    mark(&mut grid, from, &moves, true);
                    routes[net as usize] = Some((from, moves));
                    return Err(format!("cannot equalise crossing inputs at node {n} (Δp = {})", target - other));
                }
            }
        }
        for (k, &net) in node.ins.iter().enumerate() {
            p_sink[net as usize] = phases[k];
        }
        match node.op {
            Op::Cross => {
                p_start[node.outs[0] as usize] = phases[0];
                p_start[node.outs[1] as usize] = phases[0] + 1;
            }
            Op::Split => {
                p_start[node.outs[0] as usize] = phases[0] + split_p.0;
                p_start[node.outs[1] as usize] = phases[0] + split_p.1;
            }
            _ => {}
        }
    }

    // ---- Physical construction ----
    let mut em = Emit::default();
    for n in 0..g.nets.len() {
        em.phys.sigs.push(SigKind::Net(n as u32));
    }
    em.phys.grid = G;
    em.phys.books.push(cross_book().0.clone());

    // Glider (trajectory, time) available at each net's driver port.
    let mut net_start: HashMap<NetId, (Traj, i64)> = HashMap::new();
    // Builders that reached their sink port.
    let mut at_sink: HashMap<NetId, PathBuilder> = HashMap::new();

    let route_follow = |net: NetId, start: (Traj, i64), routes: &Vec<Option<(St, Vec<Move>)>>| -> Option<PathBuilder> {
        let (from, moves) = routes[net as usize].as_ref()?;
        let mut b = PathBuilder::new(start.0, start.1);
        let mut s = *from;
        for &m in moves {
            if m == Move::Turn {
                let (cx, cy) = gc_xy(s.i, s.j);
                let k = b.k_near(Kind::Dup, Some(b.cur.dir), cx, cy);
                b.dup(k)?;
                let ns = step(s, m);
                match s.t {
                    Track::Row => b.turn(Kind::Cc, Dir::NE, col_lane(ns.i))?,
                    Track::Col => b.turn(Kind::Cc, Dir::SE, row_lane(ns.j))?,
                };
            }
            s = step(s, m);
        }
        Some(b)
    };

    for &n in &order {
        let node = &g.nodes[n as usize];
        let grp = node.group;
        // Collect input builders.
        let ins: Vec<PathBuilder> = node.ins.iter().filter_map(|i| at_sink.remove(i)).collect();
        match (&node.op, &imps[n as usize]) {
            (Op::In { .. } | Op::One | Op::RegQ { .. }, NodeImpl::Source { port }) => {
                let tr = port_traj(*port, p_start[node.outs[0] as usize]);
                net_start.insert(node.outs[0], (tr, t_at_entry(tr, *port)));
            }
            (Op::Cross | Op::Split, NodeImpl::Frag(f0)) => {
                // Time-shift the fragment to the actual input phase.
                let (_, tin, ein) = f0.ins[0];
                let b0 = &ins[0];
                let dt = b0.cur.phi - tin.phi;
                let f = f0.moved(0, 0, dt);
                for (k, b) in ins.iter().enumerate() {
                    let (_, tk, ek) = f.ins[k];
                    if b.cur != tk {
                        return Err(format!("node {n} input {k}: glider {:?} != port {:?}", b.cur, tk));
                    }
                    let mut b = b.clone();
                    b.finish(ek);
                    em.path(&b, node.ins[k], grp);
                    em.eat_sides(&b, node.ins[k], grp);
                }
                let _ = ein;
                // Fragment contents.
                let role_sig = |r: u8| -> u32 { if r >= 100 { node.outs[(r - 100) as usize] } else { node.ins[r as usize] } };
                for &(p, r) in &f.placed {
                    em.inst(&p, Some(role_sig(r)), grp);
                }
                for &(l, r) in &f.legs {
                    em.phys.legs.push(Leg { traj: l.traj, t0: l.t0, t1: l.t1, sig: role_sig(r) });
                }
                let (_, btc, bte) = *cross_book();
                let _ = (btc, bte);
                for &(tx, ty, t0, t1) in &f.crosses {
                    em.phys.crosses.push(CrossSite { a: node.ins[0], b: node.ins[1], t0, t1, tx, ty, book: 0, group: grp, bbox: (tx - 10, ty - 10, tx + 10, ty + 10) });
                }
                for &c in &f.footprint {
                    em.region_cells.entry(grp).or_default().insert(c);
                }
                // Output nets: start where the fragment hands them over.
                let mut oi = 0;
                for (k, &o) in node.outs.iter().enumerate() {
                    let used = match node.op {
                        Op::Cross => out_use(g, o) == OutUse::Used,
                        _ => true,
                    };
                    if used {
                        let (port, tr, e) = f.outs[oi];
                        oi += 1;
                        if port_traj(port, p_start[o as usize]).phi != tr.phi {
                            return Err(format!("node {n} out {k}: phase {} != expected {}", tr.phi, p_start[o as usize] * PH));
                        }
                        net_start.insert(o, (tr, e));
                    }
                }
            }
            (Op::Out { .. }, NodeImpl::Sink { .. }) => {
                let mut b = ins.into_iter().next().unwrap();
                let (x, y) = b.cur.pos_at(b.t_cur + 4 * 40);
                let k = b.k_near(Kind::Eater, None, x as i64, y as i64);
                b.eat(k);
                em.path(&b, node.ins[0], grp);
                em.eat_sides(&b, node.ins[0], grp);
            }
            (Op::RegD { reg, bit }, NodeImpl::Sink { .. }) => {
                em.d_tails.insert((*reg, *bit), (ins.into_iter().next().unwrap(), node.ins[0], grp));
            }
            _ => {}
        }
        // Follow routes of this node's output nets now that their start is known.
        for &o in &node.outs {
            if let Some(&start) = net_start.get(&o) {
                if routes[o as usize].is_some() {
                    let b = route_follow(o, start, &routes).ok_or_else(|| format!("cannot build route of net {o}"))?;
                    if b.cur.phi != p_sink[o as usize] * PH {
                        return Err(format!("net {o}: phase at sink {} != {}", b.cur.phi, p_sink[o as usize] * PH));
                    }
                    at_sink.insert(o, b);
                }
            }
        }
    }

    // ---- Tapes for In / One ----
    for node in g.nodes.iter() {
        match node.op {
            Op::In { port, bit } => {
                let (tr, e) = net_start[&node.outs[0]];
                let sig = em.phys.sigs.len() as u32;
                em.phys.sigs.push(SigKind::Input { port, bit });
                em.phys.tapes.push((tr, sig));
                em.phys.legs.push(Leg { traj: tr, t0: i64::MIN / 4, t1: e, sig });
            }
            Op::One => {
                let (tr, e) = net_start[&node.outs[0]];
                let sig = em.phys.sigs.len() as u32;
                em.phys.sigs.push(SigKind::One);
                em.phys.tapes.push((tr, sig));
                em.phys.legs.push(Leg { traj: tr, t0: i64::MIN / 4, t1: e, sig });
            }
            _ => {}
        }
    }

    // ---- Register loops ----
    let bb = em.bbox_insts().unwrap_or((0, 0, 0, 0));
    let corners = [(bb.0, bb.1), (bb.2, bb.1), (bb.0, bb.3), (bb.2, bb.3)];
    let u_max = corners.iter().map(|&(x, y)| x + y).max().unwrap() + G;
    let u_min = corners.iter().map(|&(x, y)| x + y).min().unwrap() - G;
    let v_max = corners.iter().map(|&(x, y)| x - y).max().unwrap() + G;
    let mut regs: Vec<(u32, u32, NetId, St)> = Vec::new();
    for (n, node) in g.nodes.iter().enumerate() {
        if let Op::RegQ { reg, bit } = node.op {
            if let NodeImpl::Source { port } = imps[n] {
                regs.push((reg, bit, node.outs[0], port));
            }
        }
    }
    // Outer loops get the lowest rows: sort by Q row descending → increasing offsets.
    regs.sort_by_key(|r| std::cmp::Reverse(r.3.j));
    let mut fabric_t = 0i64;
    for l in &em.phys.legs {
        if l.t1 < i64::MAX / 8 {
            fabric_t = fabric_t.max(l.t1);
        }
    }
    let mut need = fabric_t + 400;
    let mut mins = Vec::new();
    for (k, &(reg, bit, qnet, qport)) in regs.iter().enumerate() {
        let (tail, _, _) = em.d_tails.get(&(reg, bit)).ok_or("missing register D")?;
        let qtr = port_traj(qport, 0);
        let off = 2 * G + 48 * k as i64;
        let b = return_loop(tail, qtr, u_max + off, v_max + off, u_min - off, None)?;
        need = need.max(b.cur.phi - qtr.phi);
        mins.push(off);
        let _ = qnet;
    }
    let period = ((need + 8 * 37 + 400 + 2 * PH - 1) / (2 * PH)) * 2 * PH;
    for (k, &(reg, bit, qnet, qport)) in regs.iter().enumerate() {
        let (tail, dnet, grp) = em.d_tails.get(&(reg, bit)).cloned().ok_or("missing register D")?;
        let qtr = port_traj(qport, 0);
        let off = mins[k];
        let b = return_loop(&tail, qtr, u_max + off, v_max + off, u_min - off, Some(qtr.phi + period))?;
        let sig = em.phys.sigs.len() as u32;
        em.phys.sigs.push(SigKind::RegNext { reg, bit });
        let n_pre = tail.legs.len();
        let np_pre = tail.placed.len();
        let mut pre = b.clone();
        pre.legs.truncate(n_pre);
        pre.placed.truncate(np_pre);
        // The D net's last leg ends at the D port; split there.
        let t_port = t_at_entry(tail.cur, St { i: 0, j: 0, t: Track::Row }).max(tail.t_cur);
        let _ = t_port;
        em.path(&pre, dnet, grp);
        em.eat_sides(&tail, dnet, grp);
        let mut post = b.clone();
        post.legs.drain(..n_pre);
        post.placed.drain(..np_pre);
        let tq = t_at_entry(b.cur, qport);
        post.finish(tq);
        em.path(&post, sig, 0);
        em.eat_sides(&post, sig, 0);
        let _ = qnet;
    }
    em.phys.period = period;

    // ---- Regions, bbox, time window ----
    for (gid, cells) in em.region_cells.iter() {
        let mut v: Vec<(i32, i32)> = cells.iter().copied().collect();
        v.sort();
        em.phys.regions.push(Region { group: *gid, cells: v });
    }
    em.phys.regions.sort_by_key(|r| r.group);
    em.phys.bbox = em.bbox_insts().unwrap_or((0, 0, 0, 0));
    let mut tmin = i64::MAX;
    let mut tmax = i64::MIN;
    for l in &em.phys.legs {
        if l.t0 > i64::MIN / 8 {
            tmin = tmin.min(l.t0);
        }
        tmax = tmax.max(l.t1);
    }
    em.phys.t_min = tmin;
    em.phys.t_max = tmax;
    let grid_cells = grid.cells.len();
    Ok(LayoutResult { phys: em.phys, phase: p_start, grid_cells })
}

#[derive(Default)]
struct Emit {
    phys: Phys,
    region_cells: HashMap<u32, HashSet<(i32, i32)>>,
    d_tails: HashMap<(u32, u32), (PathBuilder, NetId, u32)>,
}

impl Emit {
    fn inst(&mut self, p: &Placed, trigger: Option<u32>, group: u32) {
        let (a, b, c, d) = p.bbox();
        self.region_cells.entry(group).or_default().insert(xy_gc((a + c) / 2, (b + d) / 2));
        self.phys.insts.push(Inst { comp: comp_index(p.o), tx: p.tx, ty: p.ty, dt: p.dt, trigger, group });
    }
    fn path(&mut self, b: &PathBuilder, sig: u32, group: u32) {
        for l in &b.legs {
            self.phys.legs.push(Leg { traj: l.traj, t0: l.t0, t1: l.t1, sig });
        }
        for p in &b.placed {
            self.inst(p, Some(sig), group);
        }
    }
    fn eat_sides(&mut self, b: &PathBuilder, sig: u32, group: u32) {
        for &(side, ts) in &b.side {
            let mut sb = PathBuilder::new(side, ts);
            let (x, y) = side.pos_at(ts + 28);
            let k = sb.k_near(Kind::Eater, None, x as i64, y as i64);
            sb.eat(k);
            self.path(&sb, sig, group);
        }
    }
    fn bbox_insts(&self) -> Option<(i64, i64, i64, i64)> {
        let mut it = self.phys.insts.iter().map(|i| i.bbox());
        let first = it.next()?;
        Some(it.fold(first, |a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))))
    }
}

/// Build a register return loop from the D tail (row glider heading east) to the Q row.
/// Corners (in rotated coordinates): up at `u = ua`, left at `v = vb`, down at `u = uc`.
/// With `target_phi`, the loop is lengthened (steps of 8) to arrive with exactly that phase.
fn return_loop(tail: &PathBuilder, qtr: Traj, ua: i64, vb: i64, uc: i64, target_phi: Option<i64>) -> Result<PathBuilder, String> {
    let kinds_list = [
        [Kind::Cc, Kind::Cc, Kind::Cc, Kind::Cc],
        [Kind::Snark, Kind::Snark, Kind::Cc, Kind::Cc],
        [Kind::Cc, Kind::Snark, Kind::Snark, Kind::Cc],
        [Kind::Cc, Kind::Cc, Kind::Snark, Kind::Snark],
        [Kind::Snark, Kind::Cc, Kind::Snark, Kind::Cc],
        [Kind::Snark, Kind::Cc, Kind::Cc, Kind::Snark],
        [Kind::Snark, Kind::Snark, Kind::Snark, Kind::Snark],
    ];
    let mut best: Option<PathBuilder> = None;
    for nd in 0..8usize {
        for kinds in kinds_list {
            for par in 0..2i64 {
                let build = |extra: i64| -> Option<PathBuilder> {
                    let mut b = tail.clone();
                    b.side.clear();
                    // NE lane = -u.
                    b.turn(kinds[0], Dir::NE, -(ua + par))?;
                    for d in 0..nd {
                        let (x, y) = b.cur.pos_at(b.t_cur + 4 * (40 + 44 * d as i64));
                        let k = b.k_near(Kind::Dup, Some(Dir::NE), x as i64, y as i64);
                        b.dup(k)?;
                    }
                    // NW lane = -v.
                    let vtop = vb;
                    let (mut ok, mut lane2) = (false, -vtop);
                    for adj in 0..2 {
                        let mut bt = b.clone();
                        if bt.turn(kinds[1], Dir::NW, -vtop - adj).is_some() {
                            ok = true;
                            lane2 = -vtop - adj;
                            break;
                        }
                    }
                    if !ok {
                        return None;
                    }
                    b.turn(kinds[1], Dir::NW, lane2)?;
                    // SW lane = u.
                    let mut ok3 = None;
                    for adj in 0..2 {
                        let mut bt = b.clone();
                        if bt.turn(kinds[2], Dir::SW, uc - 2 * extra - adj).is_some() {
                            ok3 = Some(uc - 2 * extra - adj);
                            break;
                        }
                    }
                    b.turn(kinds[2], Dir::SW, ok3?)?;
                    b.turn(kinds[3], Dir::SE, qtr.lane)?;
                    Some(b)
                };
                let Some(b0) = build(0) else {
                    if std::env::var("GOLDL_DEBUG").is_ok() {
                        eprintln!("return_loop: build failed nd={nd} kinds={kinds:?} par={par}");
                    }
                    continue;
                };
                if std::env::var("GOLDL_DEBUG").is_ok() {
                    eprintln!("return_loop: ok nd={nd} kinds={kinds:?} par={par} phi={} target={target_phi:?}", b0.cur.phi);
                }
                match target_phi {
                    None => {
                        if best.as_ref().map_or(true, |bb| b0.cur.phi < bb.cur.phi) {
                            best = Some(b0);
                        }
                    }
                    Some(t) => {
                        let diff = t - b0.cur.phi;
                        if std::env::var("GOLDL_DEBUG").is_ok() {
                            eprintln!("  target diff {diff} (mod 8 = {})", diff.rem_euclid(8));
                        }
                        if diff >= 0 && diff % 8 == 0 {
                            let r1 = build(diff / 8);
                            if std::env::var("GOLDL_DEBUG").is_ok() {
                                eprintln!("  lengthened: {:?}", r1.as_ref().map(|b| (b.cur.phi, b.cur.lane, qtr.lane)));
                            }
                            if let Some(b1) = r1 {
                                if b1.cur.phi == t && b1.cur.lane == qtr.lane {
                                    return Ok(b1);
                                }
                            }
                        }
                    }
                }
            }
        }
        if target_phi.is_none() && best.is_some() {
            break;
        }
    }
    match (target_phi, best) {
        (None, Some(b)) => Ok(b),
        _ => Err("cannot close register loop".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tech::glider::extract_gliders;
    use goldl_life::{Cell, Pattern};

    /// Simulate a fragment's components with the given input gliders present.
    fn sim_fragment(f: &Fragment, present: &[bool], t_end: i64) -> (Vec<Traj>, bool) {
        let t0 = f.ins.iter().map(|x| x.2).min().unwrap() - 20;
        let mut cells: Vec<Cell> = Vec::new();
        for (p, _) in &f.placed {
            cells.extend(p.cells());
        }
        let stat = Pattern::from_cells(cells.clone());
        for (k, &(_, tr, _)) in f.ins.iter().enumerate() {
            if present[k] {
                cells.extend(tr.cells_at(t0));
            }
        }
        let mut u = Universe::from_pattern(&Pattern::from_cells(cells));
        u.step((t_end - t0) as u64);
        let (gl, rest) = extract_gliders(&u.to_pattern(), t_end);
        (gl, rest == stat)
    }

    #[test]
    fn cross_fragment_truth_table() {
        let f = cross_fragment(OutUse::Used, OutUse::Used);
        let t_end = f.outs.iter().map(|o| o.2).max().unwrap() + 40;
        for (pa, pb) in [(false, false), (true, false), (false, true), (true, true)] {
            let (gl, restored) = sim_fragment(&f, &[pa, pb], t_end);
            assert!(restored, "components restored for {pa} {pb}");
            let mut expect = Vec::new();
            if pa && !pb {
                expect.push(f.outs[0].1);
            }
            if pb && !pa {
                expect.push(f.outs[1].1);
            }
            let mut got: Vec<Traj> = gl;
            got.sort();
            expect.sort();
            // compare as positions at t_end (trajectories are exact)
            assert_eq!(got, expect, "a={pa} b={pb}");
        }
        assert_eq!(f.outs[1].1.phi - f.ins[0].1.phi, PH);
    }

    #[test]
    fn split_fragment_works() {
        let f = split_fragment();
        let t_end = f.outs.iter().map(|o| o.2).max().unwrap() + 40;
        let (gl, restored) = sim_fragment(f, &[true], t_end);
        assert!(restored);
        let mut expect: Vec<Traj> = f.outs.iter().map(|o| o.1).collect();
        expect.sort();
        let mut gl = gl;
        gl.sort();
        assert_eq!(gl, expect);
        assert_eq!(f.outs[0].1.phi.rem_euclid(2 * PH), PH);
        assert_eq!(f.outs[1].1.phi.rem_euclid(2 * PH), 0);
    }
}
