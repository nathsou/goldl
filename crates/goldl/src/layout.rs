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
use goldl_life::{Cell, Pattern, Universe};
use std::collections::{HashMap, HashSet};
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
    /// Empty flight is a run, not one allocation per grid cell.
    Straight(i32),
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
    (
        (u as f64 / G as f64).round() as i32,
        (v as f64 / G as f64).round() as i32,
    )
}
fn gc_offset_xy(di: i32, dj: i32) -> (i64, i64) {
    gc_xy(di, dj)
}

fn step(s: St, m: Move) -> St {
    match (s.t, m) {
        (Track::Row, Move::Straight(n)) => St { i: s.i + n, ..s },
        (Track::Col, Move::Straight(n)) => St { j: s.j + n, ..s },
        (Track::Row, Move::Turn) => St {
            i: s.i + 1,
            j: s.j + 1,
            t: Track::Col,
        },
        (Track::Col, Move::Turn) => St {
            i: s.i + 1,
            j: s.j + 1,
            t: Track::Row,
        },
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
        (Track::Row, Move::Straight(_)) => [((s.i, s.j), Use::Row), ((s.i, s.j), Use::Row)],
        (Track::Col, Move::Straight(_)) => [((s.i, s.j), Use::Col), ((s.i, s.j), Use::Col)],
        (Track::Row, Move::Turn) => [((s.i, s.j), Use::Full), ((s.i + 1, s.j), Use::Full)],
        (Track::Col, Move::Turn) => [((s.i, s.j), Use::Full), ((s.i, s.j + 1), Use::Full)],
    }
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
        let mv = |s: St| St {
            i: s.i + di,
            j: s.j + dj,
            t: s.t,
        };
        Fragment {
            ins: self
                .ins
                .iter()
                .map(|&(s, t, e)| (mv(s), tr(t), e + dt))
                .collect(),
            outs: self
                .outs
                .iter()
                .map(|&(s, t, e)| (mv(s), tr(t), e + dt))
                .collect(),
            placed: self
                .placed
                .iter()
                .map(|&(p, r)| {
                    (
                        Placed {
                            tx: p.tx + dx,
                            ty: p.ty + dy,
                            dt: p.dt + dt,
                            ..p
                        },
                        r,
                    )
                })
                .collect(),
            legs: self
                .legs
                .iter()
                .map(|&(l, r)| {
                    (
                        LegSpec {
                            traj: tr(l.traj),
                            t0: l.t0 + dt,
                            t1: l.t1 + dt,
                        },
                        r,
                    )
                })
                .collect(),
            footprint: self
                .footprint
                .iter()
                .map(|&(i, j)| (i + di, j + dj))
                .collect(),
            approach: self
                .approach
                .iter()
                .map(|&(i, j, t)| (i + di, j + dj, t))
                .collect(),
            crosses: self
                .crosses
                .iter()
                .map(|&(x, y, a, b)| (x + dx, y + dy, a + dt, b + dt))
                .collect(),
        }
    }
}

fn port_traj(s: St, phase: i64) -> Traj {
    match s.t {
        Track::Row => Traj {
            dir: Dir::SE,
            lane: row_lane(s.j),
            phi: phase * PH,
        },
        Track::Col => Traj {
            dir: Dir::NE,
            lane: col_lane(s.i),
            phi: phase * PH,
        },
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
    let track = if tr.dir == Dir::SE {
        Track::Row
    } else {
        Track::Col
    };
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

fn finish_fragment(
    mut f: Fragment,
    outs_builders: Vec<(PathBuilder, u8)>,
    extra_fp: &[(i32, i32)],
) -> Fragment {
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
        let leg = LegSpec {
            traj: b.cur,
            t0: b.t_cur,
            t1: te,
        };
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
    let sa = St {
        i: 0,
        j: 0,
        t: Track::Row,
    };
    let sb = St {
        i: 0,
        j: -1,
        t: Track::Row,
    };
    let ta = port_traj(sa, 0);
    let tbr = port_traj(sb, 0);
    let ea = t_at_entry(ta, sa);
    let eb = t_at_entry(tbr, sb);
    let mut f = Fragment {
        ins: vec![(sa, ta, ea), (sb, tbr, eb)],
        outs: vec![],
        placed: vec![],
        legs: vec![],
        footprint: vec![],
        approach: vec![],
        crosses: vec![],
    };
    let mut bb = PathBuilder::new(tbr, eb);
    bb.turn(Kind::Cc, Dir::NE, col_lane(0))
        .expect("cross b turn");
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
    eprintln!(
        "split outs {:?} footprint {:?} approach {:?}",
        f.outs, f.footprint, f.approach
    );
    let c = cross_fragment(OutUse::Used, OutUse::Used);
    eprintln!(
        "cross outs {:?} footprint {:?} approach {:?}",
        c.outs, c.footprint, c.approach
    );
}

fn split_fragment() -> &'static Fragment {
    static F: OnceLock<Fragment> = OnceLock::new();
    F.get_or_init(|| {
        let s0 = St {
            i: 0,
            j: 0,
            t: Track::Row,
        };
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
                    if boxes_close(dupv.bbox(), p.bbox(), 3)
                        || boxes_close(dupv.bbox(), dup_bb, 3)
                        || boxes_close(dupv.bbox(), b1.placed[0].bbox(), 3)
                    {
                        continue;
                    }
                    let up2 = mid.cur;
                    let Some(p2) = place_turn(o_r, up2, i_r, row_lane(row)) else {
                        continue;
                    };
                    if boxes_close(p2.bbox(), dup_bb, 3)
                        || boxes_close(p2.bbox(), p.bbox(), 3)
                        || boxes_close(p2.bbox(), dupv.bbox(), 3)
                    {
                        continue;
                    }
                    if boxes_close(p2.bbox(), b1.placed[0].bbox(), 3)
                        || boxes_close(p.bbox(), b1.placed[0].bbox(), 3)
                    {
                        continue;
                    }
                    let fin = p2.output(i_r);
                    if fin.phi.rem_euclid(2 * PH) != 0 {
                        continue;
                    }
                    let pv_min = {
                        let (a, bq, c, d) = p.bbox();
                        [(a, bq), (c, bq), (a, d), (c, d)]
                            .iter()
                            .map(|&(x, y)| x - y)
                            .min()
                            .unwrap()
                    };
                    if pv_min < 12 {
                        continue;
                    }
                    let mut f = Fragment {
                        ins: vec![(s0, t0, e0)],
                        outs: vec![],
                        placed: vec![],
                        legs: vec![],
                        footprint: vec![],
                        approach: vec![],
                        crosses: vec![],
                    };
                    f.legs.push((b.legs[0], 0));
                    f.placed.push((b.placed[0], 0));
                    let mut s2 = PathBuilder::new(side, ts);
                    s2.legs.push(LegSpec {
                        traj: side,
                        t0: ts,
                        t1: p.t_contact(),
                    });
                    s2.placed.push(p);
                    s2.legs.push(LegSpec {
                        traj: up,
                        t0: p.t_settled(),
                        t1: dupv.t_contact(),
                    });
                    s2.placed.push(dupv);
                    s2.legs.push(LegSpec {
                        traj: up2,
                        t0: dupv.t_settled(),
                        t1: p2.t_contact(),
                    });
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

/// Delay fragment: row 0 in at gc (0, 0), row 1 out, delay exactly `2·m` phases (86·m gens).
fn delay_fragment(m: i64) -> Option<Fragment> {
    static CACHE: OnceLock<std::sync::Mutex<HashMap<i64, Option<Fragment>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    if let Some(f) = cache.lock().unwrap().get(&m) {
        return f.clone();
    }
    let f = build_delay(m);
    cache.lock().unwrap().insert(m, f.clone());
    f
}

/// Minimum delay (in phase pairs) the delay fragment supports.
pub const DELAY_MIN_M: i64 = 14;

fn build_delay(m: i64) -> Option<Fragment> {
    let s0 = St {
        i: 0,
        j: 0,
        t: Track::Row,
    };
    let t0 = port_traj(s0, 0);
    let e0 = t_at_entry(t0, s0);
    let target = 2 * PH * m;
    let kinds = [Kind::Cc, Kind::Snark];
    let configs = [0usize, 1, 2, 3].iter().flat_map(|&d| {
        [30i64, 40, 55]
            .iter()
            .flat_map(move |&gap| (0..8).map(move |q| (d, gap, 30 + 15 * q)))
    });
    for (with_dup, gap, h) in configs {
        if with_dup == 0 && gap != 30 {
            continue;
        }
        for &k1 in &kinds {
            for &k2 in &kinds {
                for &k3 in &kinds {
                    for &k4 in &kinds {
                        let build = |l: i64| -> Option<PathBuilder> {
                            let mut b = PathBuilder::new(t0, e0);
                            let mut ok = false;
                            for off in [0, 1, -1] {
                                let mut bt = b.clone();
                                if bt.turn(k1, Dir::NE, col_lane(0) + off).is_some() {
                                    b = bt;
                                    ok = true;
                                    break;
                                }
                            }
                            if !ok {
                                return None;
                            }
                            for d in 0..with_dup {
                                let (x, y) = b.cur.pos_at(b.t_cur + 4 * (45 + gap * d as i64));
                                let k = b.k_near(Kind::Dup, Some(Dir::NE), x as i64, y as i64);
                                b.dup(k)?;
                            }
                            let mut ok = false;
                            for off in [0, 1, -1] {
                                let mut bt = b.clone();
                                // NW lane of the current position (lane = y - x), decreasing ahead.
                                let (x, y) = b.cur.pos_at(b.t_cur);
                                let lane_here = (y - x) as i64;
                                // The NW leg must lie beyond row 1 (x - y > G) with room for the last turn.
                                let lane_nw = lane_here.min(-(G + 50)) - (h - 30);
                                if bt.turn(k2, Dir::NW, lane_nw + off).is_some() {
                                    b = bt;
                                    ok = true;
                                    break;
                                }
                            }
                            if !ok {
                                return None;
                            }
                            let mut ok = false;
                            for off in [0, -1] {
                                let mut bt = b.clone();
                                let (x, y) = b.cur.pos_at(b.t_cur);
                                let lane_here = (x + y) as i64;
                                if bt.turn(k3, Dir::SW, lane_here - l + off).is_some() {
                                    b = bt;
                                    ok = true;
                                    break;
                                }
                            }
                            if !ok {
                                return None;
                            }
                            b.turn(k4, Dir::SE, row_lane(1))?;
                            Some(b)
                        };
                        let Some(b0) = build(40) else {
                            if std::env::var("GOLDL_DEBUG").is_ok() {
                                eprintln!(
                                    "delay: build failed {with_dup} {k1:?}{k2:?}{k3:?}{k4:?}"
                                );
                            }
                            continue;
                        };
                        let diff = target - b0.cur.phi;
                        if std::env::var("GOLDL_DEBUG").is_ok() {
                            eprintln!(
                                "delay: {with_dup} {k1:?}{k2:?}{k3:?}{k4:?} base {} diff {diff}",
                                b0.cur.phi
                            );
                        }
                        if diff < 0 || diff % 8 != 0 {
                            continue;
                        }
                        let Some(b) = build(40 + 2 * (diff / 8)) else {
                            continue;
                        };
                        if b.cur.phi != target || b.cur.lane != row_lane(1) {
                            if std::env::var("GOLDL_DEBUG").is_ok() {
                                eprintln!("delay: miss phi {} lane {}", b.cur.phi, b.cur.lane);
                            }
                            continue;
                        }
                        // Components must not interfere.
                        let ps = &b.placed;
                        let mut clash = false;
                        for x in 0..ps.len() {
                            for y in x + 1..ps.len() {
                                if boxes_close(ps[x].bbox(), ps[y].bbox(), 3) {
                                    if std::env::var("GOLDL_DEBUG").is_ok() {
                                        eprintln!(
                                            "delay: clash {with_dup} {gap} {h} {x} {y} {:?} {:?}",
                                            ps[x].bbox(),
                                            ps[y].bbox()
                                        );
                                    }
                                    clash = true;
                                }
                            }
                        }
                        if clash {
                            if std::env::var("GOLDL_DEBUG").is_ok() {
                                eprintln!("delay: clash");
                            }
                            continue;
                        }
                        let mut f = Fragment {
                            ins: vec![(s0, t0, e0)],
                            outs: vec![],
                            placed: vec![],
                            legs: vec![],
                            footprint: vec![],
                            approach: vec![],
                            crosses: vec![],
                        };
                        let mut b = b;
                        eat_sides(&mut f, &b, 100);
                        b.side.clear();
                        // Legs before the first component belong to the input; the rest to the output.
                        f.legs.push((b.legs[0], 0));
                        let mut rest = PathBuilder::new(b.cur, b.t_cur);
                        rest.legs = b.legs[1..].to_vec();
                        rest.placed = b.placed.clone();
                        let f = finish_fragment(f, vec![(rest, 100)], &[(0, 0)]);
                        if fragment_simulates(&f) {
                            return Some(f);
                        }
                        if std::env::var("GOLDL_DEBUG").is_ok() {
                            eprintln!("delay: simulation mismatch {with_dup} {gap} {h} {k1:?}{k2:?}{k3:?}{k4:?} {:?}", f.placed.iter().map(|(p, _)| p.bbox()).collect::<Vec<_>>());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Single-input fragment check by cell-level simulation: the input glider must come out on
/// the output port and every component must be restored.
fn fragment_simulates(f: &Fragment) -> bool {
    let t0 = f.ins[0].2 - 20;
    let mut cells: Vec<Cell> = Vec::new();
    for (p, _) in &f.placed {
        cells.extend(p.cells());
    }
    let stat = Pattern::from_cells(cells.clone());
    cells.extend(f.ins[0].1.cells_at(t0));
    // Run well past the exit: the output must also clear the fragment's own components.
    let t_end = f.outs[0].2 + 1500;
    let mut u = Universe::from_pattern(&Pattern::from_cells(cells));
    u.step((t_end - t0) as u64);
    let (gl, rest) = crate::tech::glider::extract_gliders(&u.to_pattern(), t_end);
    if std::env::var("GOLDL_DEBUG").is_ok() && !(rest == stat && gl == vec![f.outs[0].1]) {
        let extra: Vec<_> = rest
            .cells
            .iter()
            .filter(|c| !stat.cells.contains(c))
            .take(8)
            .collect();
        let missing: Vec<_> = stat
            .cells
            .iter()
            .filter(|c| !rest.cells.contains(c))
            .take(8)
            .collect();
        eprintln!(
            "  sim: gliders {gl:?} want {:?}; extra {extra:?} missing {missing:?}",
            f.outs[0].1
        );
    }
    rest == stat && gl == vec![f.outs[0].1]
}

fn boxes_close(a: (i64, i64, i64, i64), b: (i64, i64, i64, i64), m: i64) -> bool {
    a.0 - m <= b.2 && b.0 - m <= a.2 && a.1 - m <= b.3 && b.1 - m <= a.3
}

/// Canonical crossing flipbook (SE lane 0 φ 0, NE lane 0 φ 6): (book, t_contact, t_end).
fn cross_book() -> &'static (CrossBook, i64, i64) {
    static B: OnceLock<(CrossBook, i64, i64)> = OnceLock::new();
    B.get_or_init(|| {
        let a = Traj {
            dir: Dir::SE,
            lane: 0,
            phi: 0,
        };
        let b = Traj {
            dir: Dir::NE,
            lane: 0,
            phi: 6,
        };
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
    !net.always_zero
        && net
            .sink
            .is_some_and(|s| g.nodes[s.node as usize].op != Op::Sink)
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
    Source {
        port: St,
    },
    #[allow(dead_code)]
    Sink {
        port: St,
    },
    /// External glider stream entering from below (start chosen by the router).
    Tape,
    Virtual,
}

/// Fragment template (anchored at gc (0, 0)) implementing node `n`.
fn template(g: &Gnl, n: NodeId) -> Fragment {
    let node = &g.nodes[n as usize];
    match node.op {
        Op::Cross => cross_fragment(out_use(g, node.outs[0]), out_use(g, node.outs[1])),
        Op::Split => split_fragment().clone(),
        Op::Delay { m } => delay_fragment(m as i64).expect("feasible delay"),
        _ => unreachable!("no fragment for {:?}", node.op),
    }
}

/// Port at which a fragment hands over output `port` of its node.
fn out_port(g: &Gnl, n: NodeId, tpl: &Fragment, port: usize) -> Option<St> {
    let node = &g.nodes[n as usize];
    if node.op == Op::Cross {
        let mut k = 0;
        for (pi, &o) in node.outs.iter().enumerate() {
            let used = out_use(g, o) == OutUse::Used;
            if pi == port {
                return used.then(|| tpl.outs[k].0);
            }
            if used {
                k += 1;
            }
        }
        None
    } else {
        Some(tpl.outs[port].0)
    }
}

/// Delay lengths `m` the delay fragment can realise, ascending.
fn delay_feasible(m: i64) -> bool {
    m >= DELAY_MIN_M || (m >= 8 && m % 2 == 0)
}

/// Phase-balancing pass.
///
/// Every crossing needs its two inputs at exactly the same phase. Each node `n` gets a
/// scheduled input phase `P(n)`; an edge from output port `q` of `d` into `n` must absorb
/// `P(n) - P(d) - off(d, q) ≥ min_turns` with extra turns or, when the slack is large, a
/// spliced-in delay fragment. Input tapes, constants and register outputs have a free
/// phase. The schedule minimises the total slack (an LP over difference constraints,
/// approximated by relaxation from the ASAP solution); fan-out trees are rebuilt so that
/// late consumers sit deep in the tree. The schedule is stored in `g.sched`.
pub fn balance(g: &mut Gnl) {
    const MIN_EXTRA: i64 = 16;
    let tpls = templates(g);
    let p = schedule(g, &tpls);
    restructure_splits(g, &p, &tpls);
    let mut p = schedule(g, &tpls);
    for (d, c, n, k, net) in sched_edges(g, &tpls) {
        let slack = p[n as usize] - p[d as usize] - c;
        if slack < MIN_EXTRA {
            continue;
        }
        let (_, off, t, so) = drv_info(g, &tpls, net).unwrap();
        let q = tpls[n as usize].as_ref().unwrap().ins[k].0;
        // Through a delay: P(d) + c(d → delay) + c(delay → n) ≤ P(n).
        let fits = |m: i64| -> Option<i64> {
            let f = delay_fragment(m)?;
            let c1 = edge_c(off, t);
            let o = f.outs[0].0;
            let _ = (so, q);
            let c2 = edge_c(2 * m, o.t);
            (p[d as usize] + c1 + c2 <= p[n as usize]).then_some(c1)
        };
        let Some((m, c1)) = (8..=slack / 2)
            .rev()
            .filter(|&m| delay_feasible(m))
            .find_map(|m| fits(m).map(|c1| (m, c1)))
        else {
            continue;
        };
        if std::env::var("GOLDL_DEBUG").is_ok() {
            let gn = |x: NodeId| g.groups[g.nodes[x as usize].group as usize].name.clone();
            eprintln!(
                "balance: slack {slack} on {d}:{:?}[{}] -> {n}:{:?}[{}] (P {} -> {}), delay {m}",
                g.nodes[d as usize].op,
                gn(d),
                g.nodes[n as usize].op,
                gn(n),
                p[d as usize],
                p[n as usize]
            );
        }
        g.splice(net, Op::Delay { m: m as u32 });
        p.push(p[d as usize] + c1);
    }
    g.sched = p;
}

/// Minimum phase difference P(n) - P(d) for a connection from an output port with phase
/// offset `off` on track `t_o` (staircase routing: launch, column, input row).
fn edge_c(off: i64, t_o: Track) -> i64 {
    off + route_tmin(t_o)
}

fn split_phases() -> (i64, i64) {
    let f = split_fragment();
    (f.outs[0].1.phi / PH, f.outs[1].1.phi / PH)
}

fn templates(g: &Gnl) -> Vec<Option<Fragment>> {
    (0..g.nodes.len() as NodeId)
        .map(|n| {
            matches!(
                g.nodes[n as usize].op,
                Op::Cross | Op::Split | Op::Delay { .. }
            )
            .then(|| template(g, n))
        })
        .collect()
}

/// Phase offset and track of the output port driving `net` (None: free-phase source).
fn drv_info(g: &Gnl, tpls: &[Option<Fragment>], net: NetId) -> Option<(NodeId, i64, Track, i32)> {
    let p = g.nets[net as usize].driver;
    let node = &g.nodes[p.node as usize];
    let tpl = tpls.get(p.node as usize)?.as_ref()?;
    let st = out_port(g, p.node, tpl, p.port as usize)?;
    let t = st.t;
    let off = match node.op {
        Op::Cross => p.port as i64,
        Op::Split => {
            let sp = split_phases();
            if p.port == 0 {
                sp.0
            } else {
                sp.1
            }
        }
        Op::Delay { m } => 2 * m as i64,
        _ => return None,
    };
    Some((p.node, off, t, st.i + st.j))
}

/// Constrained edges: (driver, offset + min turns, sink, input index, net).
fn sched_edges(g: &Gnl, tpls: &[Option<Fragment>]) -> Vec<(NodeId, i64, NodeId, usize, NetId)> {
    let mut v = Vec::new();
    for (n, node) in g.nodes.iter().enumerate() {
        let Some(Some(tpl)) = tpls.get(n) else {
            continue;
        };
        for (k, &net) in node.ins.iter().enumerate() {
            if let Some((d, off, t, so)) = drv_info(g, tpls, net) {
                debug_assert_eq!(tpl.ins[k].0.t, Track::Row);
                let _ = so;
                v.push((d, edge_c(off, t), n as NodeId, k, net));
            }
        }
    }
    v
}

fn schedule(g: &Gnl, tpls: &[Option<Fragment>]) -> Vec<i64> {
    let order = g.topo();
    let n_nodes = g.nodes.len();
    let mut ins_of: Vec<Vec<(NodeId, i64)>> = vec![Vec::new(); n_nodes];
    let mut outs_of: Vec<Vec<(NodeId, i64)>> = vec![Vec::new(); n_nodes];
    for (d, c, n, _, _) in sched_edges(g, tpls) {
        ins_of[n as usize].push((d, c));
        outs_of[d as usize].push((n, c));
    }
    // ASAP.
    let mut p = vec![0i64; n_nodes];
    for &n in &order {
        if let Some(lb) = ins_of[n as usize]
            .iter()
            .map(|&(d, c)| p[d as usize] + c)
            .max()
        {
            p[n as usize] = lb;
        }
    }
    // ALAP: free-input cones follow their consumers (nodes without consumers stay ASAP).
    for &n in order.iter().rev() {
        if let Some(ub) = outs_of[n as usize]
            .iter()
            .map(|&(s, c)| p[s as usize] - c)
            .min()
        {
            p[n as usize] = ub;
        }
    }
    // Relaxation: move each node to the cheaper end of its feasible interval.
    for pass in 0..16 {
        let it: Vec<NodeId> = if pass % 2 == 0 {
            order.iter().rev().copied().collect()
        } else {
            order.clone()
        };
        for n in it {
            let nu = n as usize;
            if tpls[nu].is_none() {
                continue;
            }
            let lb = ins_of[nu].iter().map(|&(d, c)| p[d as usize] + c).max();
            let ub = outs_of[nu].iter().map(|&(s, c)| p[s as usize] - c).min();
            let (a, b) = (ins_of[nu].len(), outs_of[nu].len());
            // Ties go late: free-phase cones then follow their consumers.
            p[nu] = match (lb, ub) {
                (Some(l), _) if a > b => l,
                (_, Some(u)) => u,
                (Some(l), None) => l,
                (None, None) => p[nu],
            };
        }
    }
    p
}

/// Rebuild every fan-out tree (maximal tree of splits) so that consumers needing their
/// input late sit deep in the tree (greedy, Huffman-like on required phases). The split
/// nodes are reused, only rewired.
fn restructure_splits(g: &mut Gnl, p: &[i64], tpls: &[Option<Fragment>]) {
    const FREE: i64 = 1 << 40;
    enum Tree {
        Leaf(crate::gnl::Port),
        Node(Box<Tree>, Box<Tree>),
    }
    let sp = split_phases();
    let split_in = split_fragment().ins[0].0.t;
    let (t0, t1) = (split_fragment().outs[0].0.t, split_fragment().outs[1].0.t);
    for root in 0..g.nodes.len() {
        if g.nodes[root].op == Op::Split {
            continue;
        }
        for rnet in g.nodes[root].outs.clone() {
            let Some(s) = g.nets[rnet as usize].sink else {
                continue;
            };
            if g.nodes[s.node as usize].op != Op::Split {
                continue;
            }
            let mut splits = Vec::new();
            let mut leaves = Vec::new();
            let mut stack = vec![s.node];
            while let Some(x) = stack.pop() {
                splits.push(x);
                for &o in &g.nodes[x as usize].outs {
                    let sk = g.nets[o as usize].sink.expect("sinked");
                    if g.nodes[sk.node as usize].op == Op::Split {
                        stack.push(sk.node);
                    } else {
                        leaves.push(sk);
                    }
                }
            }
            let mut items: Vec<(i64, Track, Tree)> = leaves
                .iter()
                .map(|&l| match &tpls[l.node as usize] {
                    Some(tpl) => (
                        p[l.node as usize],
                        tpl.ins[l.port as usize].0.t,
                        Tree::Leaf(l),
                    ),
                    None => (FREE, Track::Row, Tree::Leaf(l)),
                })
                .collect();
            while items.len() > 1 {
                items.sort_by_key(|x| x.0);
                let (rx, tx, x) = items.pop().unwrap();
                let (ry, ty, y) = items.pop().unwrap();
                let _ = (tx, ty);
                let ra = (rx - sp.0 - route_tmin(t0)).min(ry - sp.1 - route_tmin(t1));
                let rb = (ry - sp.0 - route_tmin(t0)).min(rx - sp.1 - route_tmin(t1));
                let node = if ra >= rb {
                    (ra, Tree::Node(Box::new(x), Box::new(y)))
                } else {
                    (rb, Tree::Node(Box::new(y), Box::new(x)))
                };
                items.push((node.0.min(FREE), split_in, node.1));
            }
            fn assign(g: &mut Gnl, t: Tree, net: NetId, splits: &mut Vec<NodeId>) {
                let port = match t {
                    Tree::Leaf(port) => port,
                    Tree::Node(a, b) => {
                        let sn = splits.pop().expect("split count");
                        let (o0, o1) = (g.nodes[sn as usize].outs[0], g.nodes[sn as usize].outs[1]);
                        assign(g, *a, o0, splits);
                        assign(g, *b, o1, splits);
                        crate::gnl::Port { node: sn, port: 0 }
                    }
                };
                g.nets[net as usize].sink = Some(port);
                g.nodes[port.node as usize].ins[port.port as usize] = net;
            }
            let tree = items.pop().unwrap().2;
            assign(g, tree, rnet, &mut splits);
            debug_assert!(splits.is_empty());
        }
    }
}

/// Balance and lay out a GNL. Returns the balanced netlist (with delay nodes) that the
/// physical design implements.
pub fn layout(g: &Gnl) -> Result<(Gnl, LayoutResult), String> {
    let mut gb = g.clone();
    self_sustain(&mut gb);
    balance(&mut gb);
    let r = layout_diag(&gb)?;
    Ok((gb, r))
}

/// Make the circuit independent of external glider streams, except for its inputs.
///
/// * Inverted input streams (`¬x` tapes) become `NOT x`, so that every input lane carries
///   the plain bit: when an input tape runs out, the circuit sees that input as 0.
/// * Every constant-one stream is fed by a single gun: a one-bit register that holds 1 (its
///   next state is a copy of itself). Its glider circles the circuit once per clock period
///   and a tree of duplicators hands one copy per cycle to every former constant source.
///   The pattern then needs no generator of gliders from outside.
pub fn self_sustain(g: &mut Gnl) {
    // ¬x tapes → Cross(1, x).
    for n in 0..g.nodes.len() {
        let Op::In {
            port,
            bit,
            inv: true,
        } = g.nodes[n].op
        else {
            continue;
        };
        let grp = g.nodes[n].group;
        let origin = g.nodes[n].origin;
        let out = g.nodes[n].outs[0];
        let Some(sink) = g.nets[out as usize].sink.take() else {
            continue;
        };
        g.nodes[n].op = Op::In {
            port,
            bit,
            inv: false,
        };
        let one = g.add(Op::One, &[], grp);
        let x = g.add(Op::Cross, &[g.out(one, 0), out], grp);
        g.nodes[x as usize].origin = origin;
        let nx = g.out(x, 0);
        g.nets[nx as usize].sink = Some(sink);
        g.nodes[sink.node as usize].ins[sink.port as usize] = nx;
        // The other output (x ∧ ¬1) never carries a glider.
        let z = g.out(x, 1);
        g.nets[z as usize].always_zero = true;
        g.add(Op::Sink, &[z], grp);
    }
    // Constant sources → one gun register and a duplicator tree.
    let ones: Vec<usize> = (0..g.nodes.len())
        .filter(|&n| g.nodes[n].op == Op::One && g.nets[g.nodes[n].outs[0] as usize].sink.is_some())
        .collect();
    let dead: Vec<bool> = (0..g.nodes.len())
        .map(|n| g.nodes[n].op == Op::One)
        .collect();
    if ones.is_empty() {
        if dead.iter().any(|&d| d) {
            let sinks: Vec<usize> = (0..g.nodes.len())
                .filter(|&n| {
                    g.nodes[n].op == Op::Sink
                        && dead[g.nets[g.nodes[n].ins[0] as usize].driver.node as usize]
                })
                .collect();
            let mut dead = dead;
            for s in sinks {
                dead[s] = true;
            }
            g.remove_nodes(&dead);
        }
        return;
    }
    let reg = g.regs.len() as u32;
    g.regs.push(crate::gnl::RegInfo {
        name: "gun".into(),
        width: 1,
        init: vec![true],
    });
    let q = g.add(Op::RegQ { reg, bit: 0 }, &[], 0);
    // Balanced duplicator tree with one leaf per consumer (and one for the loop itself).
    let mut leaves = std::collections::VecDeque::from([g.out(q, 0)]);
    while leaves.len() < ones.len() + 1 {
        let x = leaves.pop_front().unwrap();
        let s = g.add(Op::Split, &[x], 0);
        leaves.push_back(g.out(s, 0));
        leaves.push_back(g.out(s, 1));
    }
    let d = leaves.pop_front().unwrap();
    g.add(Op::RegD { reg, bit: 0 }, &[d], 0);
    for &n in &ones {
        let o = g.nodes[n].outs[0];
        let sink = g.nets[o as usize].sink.take().unwrap();
        let l = leaves.pop_front().unwrap();
        g.nets[l as usize].sink = Some(sink);
        g.nodes[sink.node as usize].ins[sink.port as usize] = l;
        // Leaves inherit the consumer's provenance for the overlay.
        let drv = g.nets[l as usize].driver.node as usize;
        if g.nodes[drv].group == 0 {
            g.nodes[drv].group = g.nodes[sink.node as usize].group;
        }
    }
    // Remove the old sources (and sinks of unused ones).
    let mut dead = dead;
    dead.resize(g.nodes.len(), false);
    for n in 0..g.nodes.len() {
        if g.nodes[n].op == Op::Sink
            && dead[g.nets[g.nodes[n].ins[0] as usize].driver.node as usize]
        {
            dead[n] = true;
        }
    }
    g.remove_nodes(&dead);
}

/// Zig-zag unit (two turns, +2 phases) and the columns/rows it advances.
const ZZ_UNIT: &[Move] = &[Move::Turn, Move::Turn];
const ZZ_W: i32 = 2;

/// Turns of the shortest staircase connection from an output port on track `t`.
fn route_tmin(t: Track) -> i64 {
    match t {
        Track::Row => 2,
        Track::Col => 3,
    }
}

/// Order of the blocks along the staircase: register outputs, then the logic in a
/// topological order that keeps provenance groups together, then register inputs and outputs.
fn block_order(g: &Gnl, node_p: &[i64]) -> Vec<NodeId> {
    let n_nodes = g.nodes.len();
    let is_frag = |n: usize| matches!(g.nodes[n].op, Op::Cross | Op::Split | Op::Delay { .. });
    let mut out: Vec<NodeId> = (0..n_nodes as NodeId)
        .filter(|&n| matches!(g.nodes[n as usize].op, Op::RegQ { .. }))
        .collect();
    out.sort_by_key(|&n| match g.nodes[n as usize].op {
        Op::RegQ { reg, bit } => (reg, bit),
        _ => (0, 0),
    });
    // Kahn over fixed (fragment → fragment) edges, preferring to stay in the same group.
    let mut indeg = vec![0usize; n_nodes];
    for n in 0..n_nodes {
        if !is_frag(n) {
            continue;
        }
        for &i in &g.nodes[n].ins {
            if is_frag(g.nets[i as usize].driver.node as usize) {
                indeg[n] += 1;
            }
        }
    }
    let mut ready: Vec<NodeId> = (0..n_nodes as NodeId)
        .filter(|&n| is_frag(n as usize) && indeg[n as usize] == 0)
        .collect();
    let mut last_group = u32::MAX;
    while !ready.is_empty() {
        let pick = ready
            .iter()
            .enumerate()
            .min_by_key(|(_, &n)| {
                (
                    g.nodes[n as usize].group != last_group,
                    node_p[n as usize],
                    n,
                )
            })
            .map(|(k, _)| k)
            .unwrap();
        let n = ready.swap_remove(pick);
        last_group = g.nodes[n as usize].group;
        out.push(n);
        for &o in &g.nodes[n as usize].outs {
            if let Some(s) = g.nets[o as usize].sink {
                let m = s.node as usize;
                if is_frag(m) {
                    indeg[m] -= 1;
                    if indeg[m] == 0 {
                        ready.push(m as NodeId);
                    }
                }
            }
        }
    }
    let mut sinks: Vec<NodeId> = (0..n_nodes as NodeId)
        .filter(|&n| matches!(g.nodes[n as usize].op, Op::Out { .. } | Op::RegD { .. }))
        .collect();
    sinks.sort_by_key(|&n| match g.nodes[n as usize].op {
        Op::RegD { reg, bit } => (0, reg, bit),
        Op::Out { port, bit } => (1, port, bit),
        _ => (2, 0, 0),
    });
    out.extend(sinks);
    out
}

/// Staircase layout.
///
/// Blocks (one per node) are laid along the diagonal in topological order, each strictly
/// above and to the right of the previous one. Every output leaves on its own row, which
/// runs east *below* all later blocks; every input owns a column that rises from below into
/// its block, where it zig-zags to absorb phase slack and turns into the input row. Rows
/// only ever cross columns at right angles (safe by the phase-class discipline), so every
/// connection is routable by construction with exactly the scheduled number of turns.
fn layout_diag(g: &Gnl) -> Result<LayoutResult, String> {
    let n_nodes = g.nodes.len();
    let order = g.topo();
    let node_p: Vec<i64> = (0..n_nodes)
        .map(|n| g.sched.get(n).copied().unwrap_or(0))
        .collect();
    let mut imps: Vec<NodeImpl> = vec![NodeImpl::Virtual; n_nodes];
    for n in 0..n_nodes {
        if matches!(g.nodes[n].op, Op::In { .. } | Op::One) {
            imps[n] = NodeImpl::Tape;
        }
    }
    let src_kind = |net: NetId| -> &Op { &g.nodes[g.nets[net as usize].driver.node as usize].op };
    let mut est_p: Vec<i64> = vec![0; g.nets.len()];
    // Launch of each driven net: driver port, prefix moves, state on the launch row.
    let mut launch: HashMap<NetId, (St, Vec<Move>, St)> = HashMap::new();
    let mut routes: Vec<Option<(St, Vec<Move>)>> = vec![None; g.nets.len()];
    let mut turns: Vec<i64> = vec![0; g.nets.len()];
    let mut live_rows: HashMap<NetId, i32> = HashMap::new();
    let mut last_reg_d_row = -2;
    let j_bot = -4;
    let (mut ci, mut cj) = (0i32, 0i32);
    let blocks = block_order(g, &node_p);
    let mut blocks_out: Vec<crate::phys::Block> = Vec::new();
    for &n in &blocks {
        let node = &g.nodes[n as usize];
        if let Op::RegQ { .. } = node.op {
            let port = St {
                i: ci,
                j: cj,
                t: Track::Row,
            };
            imps[n as usize] = NodeImpl::Source { port };
            launch.insert(node.outs[0], (port, Vec::new(), port));
            live_rows.insert(node.outs[0], port.j);
            blocks_out.push(crate::phys::Block {
                node: n,
                group: node.group,
                i0: ci,
                j0: cj,
                i1: ci + 1,
                j1: cj + 1,
            });
            ci += 2;
            cj += 2;
            continue;
        }
        let sink = matches!(node.op, Op::Out { .. } | Op::RegD { .. });
        // Zig-zag units (two turns each) per input.
        let mut zz: Vec<i64> = Vec::new();
        for &net in &node.ins {
            let fixed = !sink && matches!(src_kind(net), Op::Cross | Op::Split | Op::Delay { .. });
            let s = if fixed {
                let (port, pre, _) = launch
                    .get(&net)
                    .ok_or_else(|| format!("net {net} used before its driver is placed"))?;
                let _ = port;
                let tmin = pre.iter().filter(|&&m| m == Move::Turn).count() as i64 + 2;
                let t = node_p[n as usize] - est_p[net as usize];
                if t < tmin || (t - tmin) % 2 != 0 {
                    return Err(format!("node {n}: bad turn budget {t} (min {tmin})"));
                }
                (t - tmin) / 2
            } else {
                0
            };
            zz.push(s);
        }
        let max_s = zz.iter().copied().max().unwrap_or(0) as i32;
        // Reuse rows after their last consumer. Earlier components lie to the left;
        // only live output lanes must pass through this block's column range.
        let min_row = node
            .ins
            .iter()
            .filter_map(|net| launch.get(net))
            .map(|(_, _, st)| st.j + 2)
            .max()
            .unwrap_or(0);
        for net in &node.ins {
            live_rows.remove(net);
        }
        let height = ZZ_W * max_s
            + 4
            + if sink {
                1
            } else {
                let f = template(g, n);
                f.footprint
                    .iter()
                    .map(|&(_, j)| j)
                    .chain(f.outs.iter().map(|o| o.0.j + 2))
                    .max()
                    .unwrap_or(0)
            };
        cj = if matches!(node.op, Op::RegD { .. }) {
            min_row.max(last_reg_d_row + 2)
        } else {
            min_row
        };
        let mut occupied: Vec<i32> = live_rows.values().copied().collect();
        occupied.sort_unstable();
        for row in occupied {
            if row >= cj - 1 && row <= cj + height + 1 {
                cj = row + 2;
            }
        }

        if std::env::var("GOLDL_TRACE").is_ok() {
            eprintln!("block {n} {:?}: zz {zz:?} at ({ci},{cj})", node.op);
        }
        // Input zones side by side: [spacer][column + 3 per unit] ...
        let mut c0s: Vec<i32> = Vec::new();
        let mut x = ci + 1;
        for &s in &zz {
            c0s.push(x);
            x += ZZ_W * s as i32 + 2;
        }
        // Delay loops can extend far left of their input port. Keep the entire
        // fragment in this fresh column band, including those backward legs.
        if !sink {
            let left = template(g, n)
                .footprint
                .iter()
                .map(|&(i, _)| i)
                .min()
                .unwrap_or(0);
            x = x.max(ci + 1 - left);
        }
        let (ai, aj) = (x, cj + ZZ_W * max_s + 2);
        let (in_ports, out_ports, mut i_max, mut j_max): (Vec<St>, Vec<(NetId, St)>, i32, i32) =
            match node.op {
                Op::Out { .. } | Op::RegD { .. } => {
                    let port = St {
                        i: ai,
                        j: aj,
                        t: Track::Row,
                    };
                    imps[n as usize] = NodeImpl::Sink { port };
                    (vec![port], Vec::new(), ai + 2, aj + 1)
                }
                _ => {
                    let f = template(g, n).moved(ai, aj, 0);
                    let ins: Vec<St> = f.ins.iter().map(|x| x.0).collect();
                    let mut outs = Vec::new();
                    let mut oi = 0;
                    for (k, &o) in node.outs.iter().enumerate() {
                        let used = node.op != Op::Cross || out_use(g, o) == OutUse::Used;
                        if used {
                            outs.push((o, f.outs[oi].0));
                            oi += 1;
                        }
                        let off = match node.op {
                            Op::Cross => k as i64,
                            Op::Split => {
                                let sp = split_phases();
                                if k == 0 {
                                    sp.0
                                } else {
                                    sp.1
                                }
                            }
                            Op::Delay { m } => 2 * m as i64,
                            _ => 0,
                        };
                        est_p[o as usize] = node_p[n as usize] + off;
                    }
                    let mut im = ai;
                    let mut jm = aj;
                    for &(ii, jj) in &f.footprint {
                        im = im.max(ii);
                        jm = jm.max(jj);
                    }
                    for st in f.outs.iter().map(|x| x.0).chain(f.ins.iter().map(|x| x.0)) {
                        im = im.max(st.i);
                        jm = jm.max(st.j);
                    }
                    imps[n as usize] = NodeImpl::Frag(f);
                    (ins, outs, im, jm)
                }
            };
        // Outputs: launch rows.
        for &(o, st) in &out_ports {
            if !net_used(g, o) {
                continue;
            }
            let (pre, ls) = match st.t {
                Track::Row => (Vec::new(), st),
                Track::Col => {
                    // Turn east at once unless the gc's row track carries another output.
                    let shared = out_ports.iter().any(|&(o2, s2)| {
                        o2 != o && s2.t == Track::Row && (s2.i, s2.j) == (st.i, st.j)
                    });
                    let pre = if shared {
                        vec![Move::Straight(1), Move::Turn]
                    } else {
                        vec![Move::Turn]
                    };
                    let mut s = st;
                    for &m in &pre {
                        for (c, _) in move_cells(s, m) {
                            i_max = i_max.max(c.0);
                            j_max = j_max.max(c.1);
                        }
                        s = step(s, m);
                    }
                    (pre, s)
                }
            };
            i_max = i_max.max(ls.i);
            j_max = j_max.max(ls.j);
            launch.insert(o, (st, pre, ls));
            live_rows.insert(o, ls.j);
        }
        // Inputs: route from the driver's launch row (or a tape from below) into the block.
        for (k, &net) in node.ins.iter().enumerate() {
            let c0 = c0s[k];
            let port = in_ports[k];
            let (start, mut moves, mut s) = match launch.get(&net) {
                Some((st, pre, ls)) => {
                    let mut moves = pre.clone();
                    let mut s = *ls;
                    if s.j + 1 > cj || s.i > c0 - 1 {
                        return Err(format!(
                            "node {n}: driver of net {net} is not below-left of its block"
                        ));
                    }
                    if s.i < c0 - 1 {
                        let m = Move::Straight(c0 - 1 - s.i);
                        moves.push(m);
                        s = step(s, m);
                    }
                    moves.push(Move::Turn);
                    s = step(s, Move::Turn);
                    (*st, moves, s)
                }
                None => {
                    let st = St {
                        i: c0,
                        j: j_bot,
                        t: Track::Col,
                    };
                    (st, Vec::new(), st)
                }
            };
            if s.j < cj {
                let m = Move::Straight(cj - s.j);
                moves.push(m);
                s = step(s, m);
            }
            for _ in 0..zz[k] {
                for &m in ZZ_UNIT {
                    moves.push(m);
                    s = step(s, m);
                }
            }
            if s.j < port.j - 1 {
                let m = Move::Straight(port.j - 1 - s.j);
                moves.push(m);
                s = step(s, m);
            }
            moves.push(Move::Turn);
            s = step(s, Move::Turn);
            if s.i < port.i {
                let m = Move::Straight(port.i - s.i);
                moves.push(m);
                s = step(s, m);
            }
            if s != port {
                return Err(format!(
                    "node {n} input {k}: route ends at {s:?}, port {port:?}"
                ));
            }
            turns[net as usize] = moves.iter().filter(|&&m| m == Move::Turn).count() as i64;
            routes[net as usize] = Some((start, moves));
        }
        if matches!(node.op, Op::RegD { .. }) {
            live_rows.insert(node.ins[0], in_ports[0].j);
            last_reg_d_row = in_ports[0].j;
        }
        blocks_out.push(crate::phys::Block {
            node: n,
            group: node.group,
            i0: ci,
            j0: cj,
            i1: i_max,
            j1: j_max,
        });
        ci = i_max + 1;
        cj = j_max + 1;
    }
    // Phases along every connection.
    let mut p_start = est_p.clone();
    let mut p_sink = vec![0i64; g.nets.len()];
    for &n in &order {
        let node = &g.nodes[n as usize];
        if node.op == Op::Sink {
            continue;
        }
        for &net in &node.ins {
            let t = turns[net as usize];
            let free = matches!(src_kind(net), Op::In { .. } | Op::One | Op::RegQ { .. });
            if free {
                p_start[net as usize] = match node.op {
                    Op::Out { .. } | Op::RegD { .. } => {
                        if matches!(src_kind(net), Op::RegQ { .. }) {
                            0
                        } else {
                            1
                        }
                    }
                    _ => node_p[n as usize] - t,
                };
            }
            p_sink[net as usize] = p_start[net as usize] + t;
        }
    }
    let mut r = construct(g, &order, &imps, &routes, p_start, &p_sink)?;
    r.phys.blocks = blocks_out;
    Ok(r)
}

/// Build the physical design from placed nodes and routed connections.
fn construct(
    g: &Gnl,
    order: &[NodeId],
    imps: &[NodeImpl],
    routes: &[Option<(St, Vec<Move>)>],
    p_start: Vec<i64>,
    p_sink: &[i64],
) -> Result<LayoutResult, String> {
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

    let route_follow = |net: NetId,
                        start: (Traj, i64),
                        routes: &[Option<(St, Vec<Move>)>]|
     -> Option<PathBuilder> {
        let (from, moves) = routes[net as usize].as_ref()?;
        if std::env::var("GOLDL_NET").is_ok_and(|v| v == net.to_string()) {
            let compact: String = moves
                .iter()
                .map(|m| if *m == Move::Turn { 'T' } else { 's' })
                .collect();
            eprintln!("net {net}: from {from:?} start {:?} moves {compact}", start);
        }
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

    for &n in order {
        let node = &g.nodes[n as usize];
        let grp = node.group;
        // Collect input builders.
        let ins: Vec<PathBuilder> = node.ins.iter().filter_map(|i| at_sink.remove(i)).collect();
        match (&node.op, &imps[n as usize]) {
            (Op::In { .. } | Op::One, NodeImpl::Tape) => {
                let out = node.outs[0];
                if let Some((st, _)) = &routes[out as usize] {
                    let tr = port_traj(*st, p_start[out as usize]);
                    net_start.insert(out, (tr, t_at_entry(tr, *st)));
                }
            }
            (Op::In { .. } | Op::One | Op::RegQ { .. }, NodeImpl::Source { port }) => {
                let tr = port_traj(*port, p_start[node.outs[0] as usize]);
                net_start.insert(node.outs[0], (tr, t_at_entry(tr, *port)));
            }
            (Op::Cross | Op::Split | Op::Delay { .. }, NodeImpl::Frag(f0)) => {
                // Time-shift the fragment to the actual input phase.
                let (_, tin, ein) = f0.ins[0];
                let b0 = &ins[0];
                let dt = b0.cur.phi - tin.phi;
                let f = f0.moved(0, 0, dt);
                for (k, b) in ins.iter().enumerate() {
                    let (_, tk, ek) = f.ins[k];
                    if b.cur != tk {
                        return Err(format!(
                            "node {n} input {k}: glider {:?} != port {:?}",
                            b.cur, tk
                        ));
                    }
                    let mut b = b.clone();
                    b.finish(ek);
                    em.path(&b, node.ins[k], grp);
                    em.eat_sides(&b, node.ins[k], grp);
                }
                let _ = ein;
                // Fragment contents.
                let role_sig = |r: u8| -> u32 {
                    if r >= 100 {
                        node.outs[(r - 100) as usize]
                    } else {
                        node.ins[r as usize]
                    }
                };
                for &(p, r) in &f.placed {
                    em.inst(&p, Some(role_sig(r)), grp);
                }
                for &(l, r) in &f.legs {
                    em.phys.legs.push(Leg {
                        traj: l.traj,
                        t0: l.t0,
                        t1: l.t1,
                        sig: role_sig(r),
                    });
                }
                let (_, btc, bte) = *cross_book();
                let _ = (btc, bte);
                for &(tx, ty, t0, t1) in &f.crosses {
                    em.phys.crosses.push(CrossSite {
                        a: node.ins[0],
                        b: node.ins[1],
                        t0,
                        t1,
                        tx,
                        ty,
                        book: 0,
                        group: grp,
                        bbox: (tx - 10, ty - 10, tx + 10, ty + 10),
                    });
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
                            return Err(format!(
                                "node {n} out {k}: phase {} != expected {}",
                                tr.phi,
                                p_start[o as usize] * PH
                            ));
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
                em.d_tails.insert(
                    (*reg, *bit),
                    (ins.into_iter().next().unwrap(), node.ins[0], grp),
                );
            }
            _ => {}
        }
        // Follow routes of this node's output nets now that their start is known.
        for &o in &node.outs {
            if let Some(&start) = net_start.get(&o) {
                if routes[o as usize].is_some() {
                    let b = route_follow(o, start, &routes)
                        .ok_or_else(|| format!("cannot build route of net {o}"))?;
                    if b.cur.phi != p_sink[o as usize] * PH {
                        return Err(format!(
                            "net {o}: phase at sink {} != {}",
                            b.cur.phi,
                            p_sink[o as usize] * PH
                        ));
                    }
                    at_sink.insert(o, b);
                }
            }
        }
    }

    // ---- Tapes for In / One ----
    for node in g.nodes.iter() {
        match node.op {
            Op::In { port, bit, inv } => {
                let Some(&(tr, e)) = net_start.get(&node.outs[0]) else {
                    continue;
                };
                let sig = em.phys.sigs.len() as u32;
                em.phys.sigs.push(SigKind::Input { port, bit, inv });
                em.phys.tapes.push((tr, sig));
                em.phys.legs.push(Leg {
                    traj: tr,
                    t0: i64::MIN / 4,
                    t1: e,
                    sig,
                });
            }
            Op::One => {
                let Some(&(tr, e)) = net_start.get(&node.outs[0]) else {
                    continue;
                };
                let sig = em.phys.sigs.len() as u32;
                em.phys.sigs.push(SigKind::One);
                em.phys.tapes.push((tr, sig));
                em.phys.legs.push(Leg {
                    traj: tr,
                    t0: i64::MIN / 4,
                    t1: e,
                    sig,
                });
            }
            _ => {}
        }
    }

    // ---- Register loops ----
    let mut bb = em.bbox_insts().unwrap_or((0, 0, 0, 0));
    for l in &em.phys.legs {
        for t in [l.t0, l.t1] {
            if t > i64::MIN / 8 && t < i64::MAX / 8 {
                let (x, y) = l.traj.pos_at(t);
                let (x, y) = (x as i64, y as i64);
                bb = (bb.0.min(x), bb.1.min(y), bb.2.max(x), bb.3.max(y));
            }
        }
    }
    let corners = [(bb.0, bb.1), (bb.2, bb.1), (bb.0, bb.3), (bb.2, bb.3)];
    let u_max = corners.iter().map(|&(x, y)| x + y).max().unwrap() + G;
    let u_min = corners.iter().map(|&(x, y)| x + y).min().unwrap() - G;
    let v_max = corners.iter().map(|&(x, y)| x - y).max().unwrap() + 8 * G;
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
        let qtr = port_traj(qport, p_start[qnet as usize]);
        let off = 2 * G + G * k as i64;
        let b = return_loop(tail, qtr, u_max + off, v_max + off, u_min - off, None)?;
        need = need.max(b.cur.phi - qtr.phi);
        mins.push(off);
        let _ = qnet;
    }
    let period = ((need + 8 * 37 + 400 + 2 * PH - 1) / (2 * PH)) * 2 * PH;
    for (k, &(reg, bit, qnet, qport)) in regs.iter().enumerate() {
        let (tail, dnet, grp) = em
            .d_tails
            .get(&(reg, bit))
            .cloned()
            .ok_or("missing register D")?;
        let qtr = port_traj(qport, p_start[qnet as usize]);
        let off = mins[k];
        let b = return_loop(
            &tail,
            qtr,
            u_max + off,
            v_max + off,
            u_min - off,
            Some(qtr.phi + period),
        )?;
        let sig = em.phys.sigs.len() as u32;
        em.phys.sigs.push(SigKind::RegNext { reg, bit });
        let n_pre = tail.legs.len();
        let np_pre = tail.placed.len();
        let mut pre = b.clone();
        pre.legs.truncate(n_pre);
        pre.placed.truncate(np_pre);
        // The D net's last leg ends at the D port; split there.
        let t_port = t_at_entry(
            tail.cur,
            St {
                i: 0,
                j: 0,
                t: Track::Row,
            },
        )
        .max(tail.t_cur);
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
        em.phys.regions.push(Region {
            group: *gid,
            cells: v,
        });
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
    if let Some(l) = em.phys.legs.iter().find(|l| l.t1 < l.t0) {
        return Err(format!(
            "route travels backwards in time: {} -> {}",
            l.t0, l.t1
        ));
    }
    // Design-rule check: the quiescent circuitry must be a still life.
    {
        if let Some(diff) = crate::static_drc::check(&em.phys.insts) {
            let (px, py) = diff[0];
            let near: Vec<String> = em
                .phys
                .insts
                .iter()
                .filter(|i| {
                    let b = i.bbox();
                    px >= b.0 - 3 && px <= b.2 + 3 && py >= b.1 - 3 && py <= b.3 + 3
                })
                .map(|i| {
                    format!(
                        "{:?}@({},{}) sig {:?}",
                        i.oriented().kind,
                        i.tx,
                        i.ty,
                        i.trigger.map(|t| em.phys.sigs[t as usize])
                    )
                })
                .collect();
            return Err(format!(
                "DRC: components interfere near {:?}: {}",
                &diff[..diff.len().min(4)],
                near.join(", ")
            ));
        }
    }
    Ok(LayoutResult {
        phys: em.phys,
        phase: p_start,
        grid_cells: 0,
    })
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
        self.region_cells
            .entry(group)
            .or_default()
            .insert(xy_gc((a + c) / 2, (b + d) / 2));
        self.phys.insts.push(Inst {
            comp: comp_index(p.o),
            tx: p.tx,
            ty: p.ty,
            dt: p.dt,
            trigger,
            group,
        });
    }
    fn path(&mut self, b: &PathBuilder, sig: u32, group: u32) {
        for l in &b.legs {
            self.phys.legs.push(Leg {
                traj: l.traj,
                t0: l.t0,
                t1: l.t1,
                sig,
            });
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
        Some(it.fold(first, |a, b| {
            (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
        }))
    }
}

/// Build a register return loop from the D tail (row glider heading east) to the Q row.
/// Corners (in rotated coordinates): up at `u = ua`, left at `v = vb`, down at `u = uc`.
/// With `target_phi`, the loop is lengthened (steps of 8) to arrive with exactly that phase.
fn return_loop(
    tail: &PathBuilder,
    qtr: Traj,
    ua: i64,
    vb: i64,
    uc: i64,
    target_phi: Option<i64>,
) -> Result<PathBuilder, String> {
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
                    if b.legs.iter().any(|l| l.t1 < l.t0) {
                        return None;
                    }
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
                                eprintln!(
                                    "  lengthened: {:?}",
                                    r1.as_ref().map(|b| (b.cur.phi, b.cur.lane, qtr.lane))
                                );
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
        let t_end = f.outs.iter().map(|o| o.2).max().unwrap() + 1500;
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
        let t_end = f.outs.iter().map(|o| o.2).max().unwrap() + 1500;
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

#[cfg(test)]
mod delay_tests {
    use super::*;
    use crate::tech::glider::extract_gliders;
    use goldl_life::{Cell, Pattern};

    #[test]
    #[ignore]
    fn delay_min() {
        let ok: Vec<i64> = (4..40).filter(|&m| build_delay(m).is_some()).collect();
        eprintln!("delay ok: {ok:?}");
    }

    #[test]
    fn delay_fragment_exact() {
        for m in [DELAY_MIN_M, 17, 20, 27, 40] {
            let f = delay_fragment(m).unwrap_or_else(|| panic!("no delay for m={m}"));
            assert_eq!(f.outs[0].1.phi - f.ins[0].1.phi, 2 * PH * m);
            let t0 = f.ins[0].2 - 20;
            let mut cells: Vec<Cell> = Vec::new();
            for (p, _) in &f.placed {
                cells.extend(p.cells());
            }
            let stat = Pattern::from_cells(cells.clone());
            cells.extend(f.ins[0].1.cells_at(t0));
            let t_end = f.outs[0].2 + 1500;
            let mut u = Universe::from_pattern(&Pattern::from_cells(cells));
            u.step((t_end - t0) as u64);
            let (gl, rest) = extract_gliders(&u.to_pattern(), t_end);
            assert_eq!(rest, stat, "m={m}: restored");
            assert_eq!(gl, vec![f.outs[0].1], "m={m}");
        }
    }
}

#[cfg(test)]
mod frag_dump {
    use super::*;
    #[test]
    #[ignore]
    fn dump_fragments() {
        let show = |name: &str, f: &Fragment| {
            eprintln!(
                "{name}: ins {:?}",
                f.ins.iter().map(|x| (x.0, x.1.phi)).collect::<Vec<_>>()
            );
            eprintln!(
                "  outs {:?}",
                f.outs.iter().map(|x| (x.0, x.1.phi)).collect::<Vec<_>>()
            );
            eprintln!("  approach {:?}", f.approach);
            let fp: HashSet<(i32, i32)> = f.footprint.iter().copied().collect();
            let (i0, i1) = (
                fp.iter().map(|c| c.0).min().unwrap() - 3,
                fp.iter().map(|c| c.0).max().unwrap() + 3,
            );
            let (j0, j1) = (
                fp.iter().map(|c| c.1).min().unwrap() - 3,
                fp.iter().map(|c| c.1).max().unwrap() + 3,
            );
            for j in (j0..=j1).rev() {
                let mut line = format!("{j:4} ");
                for i in i0..=i1 {
                    let ch = if f.ins.iter().any(|x| (x.0.i, x.0.j) == (i, j)) {
                        'I'
                    } else if f.outs.iter().any(|x| (x.0.i, x.0.j) == (i, j)) {
                        'O'
                    } else if fp.contains(&(i, j)) {
                        '#'
                    } else if f.approach.iter().any(|a| (a.0, a.1) == (i, j)) {
                        'a'
                    } else {
                        '.'
                    };
                    line.push(ch);
                }
                eprintln!("{line}");
            }
            eprintln!("  (i from {i0})");
        };
        let clear = |name: &str, f: &Fragment| {
            for (k, (_, tr, e)) in f.outs.iter().enumerate() {
                for (p, _) in &f.placed {
                    let b = p.bbox();
                    for dt in (0..4000).step_by(4) {
                        let (x, y) = tr.pos_at(e + dt);
                        let (x, y) = (x as i64, y as i64);
                        if x >= b.0 - 4 && x <= b.2 + 4 && y >= b.1 - 4 && y <= b.3 + 4 {
                            eprintln!(
                                "{name}: output {k} passes {:?} at bbox {:?} (dt {dt})",
                                p.o.kind, b
                            );
                            break;
                        }
                    }
                }
            }
        };
        {
            let f = split_fragment();
            for (st, tr, e) in &f.outs {
                eprintln!(
                    "split out {st:?} {tr:?} e {e} pos {:?} pos+800 {:?}",
                    tr.pos_at(*e),
                    tr.pos_at(e + 800)
                );
            }
            for (p, r) in &f.placed {
                eprintln!(
                    "  placed {:?} bbox {:?} role {r} contact {}",
                    p.o.kind,
                    p.bbox(),
                    p.t_contact()
                );
            }
        }
        clear("cross UU", &cross_fragment(OutUse::Used, OutUse::Used));
        clear("cross UE", &cross_fragment(OutUse::Used, OutUse::Eat));
        clear("cross EU", &cross_fragment(OutUse::Eat, OutUse::Used));
        clear("split", split_fragment());
        clear("delay 14", &delay_fragment(14).unwrap());
        show("cross UU", &cross_fragment(OutUse::Used, OutUse::Used));
        show("cross UE", &cross_fragment(OutUse::Used, OutUse::Eat));
        show("split", split_fragment());
        show("delay 14", &delay_fragment(14).unwrap());
    }
}
