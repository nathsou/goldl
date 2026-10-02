//! Diagnostic companion to cell-level `verify`: test every free flight against
//! nearby quiescent components and all return-loop flight pairs (including
//! neighboring clock cycles). This is not a replacement for full Life verification:
//! it does not check two simultaneous component reactions or all fabric crossings.
//! Usage: cargo run --release -p goldl --example audit_routes -- FILE [TOP]

use goldl::{
    driver::{compile, Options},
    tech::glider::Dir,
};
use goldl_life::Pattern;
use std::collections::HashMap;
fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: audit_routes FILE [TOP]");
    let opts = Options {
        top: std::env::args().nth(2),
        ..Options::default()
    };
    let design = compile(&std::fs::read_to_string(path).unwrap(), &opts)
        .unwrap_or_else(|(diags, _)| panic!("{diags:?}"));
    let ph = &design.layout.as_ref().unwrap().phys;
    let mut rows: HashMap<i64, Vec<usize>> = HashMap::new();
    let mut cols: HashMap<i64, Vec<usize>> = HashMap::new();
    for (i, inst) in ph.insts.iter().enumerate() {
        let (a, b, c, d) = inst.bbox();
        for j in (a - d - 4).div_euclid(128)..=(c - b + 4).div_euclid(128) {
            rows.entry(j).or_default().push(i);
        }
        for j in (a + b - 4).div_euclid(128)..=(c + d + 4).div_euclid(128) {
            cols.entry(j).or_default().push(i);
        }
    }
    let mut checked = 0;
    for (k, l) in ph.legs.iter().enumerate() {
        let dir = l.traj.dir;
        let (index, lane) = if dir == Dir::SE || dir == Dir::NW {
            (&rows, l.traj.lane * dir.dx as i64)
        } else {
            (&cols, -l.traj.lane * dir.dx as i64)
        };
        let Some(insts) = index.get(&lane.div_euclid(128)) else {
            continue;
        };
        for &i in insts {
            let inst = &ph.insts[i];
            let (a, b, c, d) = inst.bbox();
            let (low, high) = if dir.dx == dir.dy {
                (a - d, c - b)
            } else {
                (a + b, c + d)
            };
            if lane < low - 4 || lane > high + 4 {
                continue;
            }
            let (ta, tb) = (l.traj.time_at_x(a - 8), l.traj.time_at_x(c + 8));
            let start = l.t0.max(ta.min(tb));
            let end = (l.t1 - 1).min(ta.max(tb));
            if end <= start {
                continue;
            }
            let stat = Pattern::from_cells(
                inst.oriented()
                    .cells
                    .iter()
                    .map(|&(x, y)| (x + inst.tx, y + inst.ty))
                    .collect(),
            );
            let real = stat
                .union(&l.traj.pattern_at(start))
                .run((end - start) as u64);
            let want = stat.union(&l.traj.pattern_at(end));
            checked += 1;
            if real != want {
                println!("BAD leg {k} {:?} [{}..{}) sig {:?}; inst {i} {:?} bbox {:?} trigger {:?} [{}..{}); sampled {start}..{end}; checked {checked}",l.traj,l.t0,l.t1,ph.sigs[l.sig as usize],inst.oriented().kind,inst.bbox(),inst.trigger.map(|n|ph.sigs[n as usize]),inst.t_contact(),inst.t_settled());
                panic!("free flight is obstructed");
            }
        }
    }
    println!("checked {checked}: static lanes clear");
    let loops: Vec<_> = ph
        .legs
        .iter()
        .filter(|l| {
            matches!(
                ph.sigs[l.sig as usize],
                goldl::phys::SigKind::RegNext { .. }
            )
        })
        .collect();
    for (i, a) in loops.iter().enumerate() {
        for b in &loops[..i] {
            for shift in [-ph.period, 0, ph.period] {
                let bt = b.traj.delayed(shift);
                let start = a.t0.max(b.t0 + shift);
                let end = a.t1.min(b.t1 + shift) - 1;
                if start >= end {
                    continue;
                }
                let (ax, ay) = a.traj.pos_at(start);
                let (bx, by) = bt.pos_at(start);
                let mut lo = 0.0_f64;
                let mut hi = (end - start) as f64;
                for (r, v) in [
                    (ax - bx, (a.traj.dir.dx - bt.dir.dx) as f64 / 4.0),
                    (ay - by, (a.traj.dir.dy - bt.dir.dy) as f64 / 4.0),
                ] {
                    if v == 0.0 {
                        if r.abs() > 6.0 {
                            hi = -1.0;
                            break;
                        }
                    } else {
                        let x = (-6.0 - r) / v;
                        let y = (6.0 - r) / v;
                        lo = lo.max(x.min(y));
                        hi = hi.min(x.max(y));
                    }
                }
                if lo > hi {
                    continue;
                }
                let ts = (start + lo.floor() as i64 - 24).max(start);
                let te = (start + hi.ceil() as i64 + 24).min(end);
                let real = a
                    .traj
                    .pattern_at(ts)
                    .union(&bt.pattern_at(ts))
                    .run((te - ts) as u64);
                let want = a.traj.pattern_at(te).union(&bt.pattern_at(te));
                if real != want {
                    panic!(
                        "return collision {:?} {:?} and {:?} {:?}, shift {shift}, {ts}..{te}",
                        ph.sigs[a.sig as usize], a.traj, ph.sigs[b.sig as usize], b.traj
                    );
                }
            }
        }
    }
    println!("{} return legs: dynamic crossings clear", loops.len());
}
