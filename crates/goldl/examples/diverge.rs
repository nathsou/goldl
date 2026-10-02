//! Locate the first divergence between the real Life evolution (HashLife) and the exact
//! reconstruction: find a mismatching checkpoint, then bisect on the generation, comparing
//! a window around the first bad cell, and print the circuitry nearby.
//!
//! Usage: cargo run --release -p goldl --example diverge -- FILE [CYCLES] [SEED]

use goldl::driver::{compile, Options};
use goldl::sim::{reconstruct, reconstruct_all, Rect, TraceSignals};
use goldl_life::hashlife::HashLife;
use goldl_life::Pattern;
use std::collections::HashSet;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = &args[1];
    let cycles: usize = args.get(2).map_or(3, |s| s.parse().unwrap());
    let mut seed: u64 = args.get(3).map_or(1, |s| s.parse().unwrap());
    let src = std::fs::read_to_string(path).unwrap();
    let c = compile(&src, &Options::default()).unwrap_or_else(|(d, _)| panic!("{d:?}"));
    let lay = c.layout.as_ref().expect("layout");
    let ph = &lay.phys;
    let mut rng = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut bits: Vec<Vec<Vec<bool>>> = (0..cycles)
        .map(|_| {
            c.rtl
                .inputs
                .iter()
                .map(|p| (0..p.width).map(|_| rng() & 1 == 1).collect())
                .collect()
        })
        .collect();
    for _ in 0..3 {
        bits.push(
            c.rtl
                .inputs
                .iter()
                .map(|p| vec![false; p.width as usize])
                .collect(),
        );
    }
    let trace = TraceSignals::record(&c.gnl, bits);
    let p0 = reconstruct_all(ph, 0, &trace);
    let t = ph.period;
    // 1. First mismatching checkpoint.
    let mut focus = None;
    let mut hi = 0;
    {
        let mut u = HashLife::from_pattern(&p0);
        let mut now = 0;
        for k in 1..=(2 * cycles as i64) {
            let cp = k * t / 2 + 13;
            u.step((cp - now) as u64);
            now = cp;
            let real = u.to_pattern().to_set();
            let model = reconstruct_all(ph, cp, &trace).to_set();
            if let Some(&p) = real.symmetric_difference(&model).min() {
                focus = Some(p);
                hi = cp;
                break;
            }
        }
    }
    let Some((fx, fy)) = focus else {
        println!("no divergence");
        return;
    };
    // 2. Bisect on generation within a window around the focus.
    let r = 1500;
    let rect = Rect {
        x0: fx - r,
        y0: fy - r,
        x1: fx + r,
        y1: fy + r,
    };
    let window = |g: i64| -> (HashSet<(i64, i64)>, HashSet<(i64, i64)>) {
        let mut u = HashLife::from_pattern(&p0);
        u.step(g as u64);
        let real: HashSet<(i64, i64)> = u
            .to_pattern()
            .cells
            .into_iter()
            .filter(|&(x, y)| x >= rect.x0 && x <= rect.x1 && y >= rect.y0 && y <= rect.y1)
            .collect();
        let mut v = Vec::new();
        reconstruct(ph, g, &trace, rect, &mut v);
        let model: HashSet<(i64, i64)> = v
            .into_iter()
            .filter(|&(x, y)| x >= rect.x0 && x <= rect.x1 && y >= rect.y0 && y <= rect.y1)
            .collect();
        (real, model)
    };
    let mut lo = 0;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        let (a, b) = window(mid);
        if a == b {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (real, model) = window(hi);
    let a: Vec<_> = real.difference(&model).copied().collect();
    let b: Vec<_> = model.difference(&real).copied().collect();
    let all: Vec<(i64, i64)> = a.iter().chain(b.iter()).copied().collect();
    let bb = Pattern::from_cells(all.clone()).bbox().unwrap();
    println!("first divergence at generation {hi} (cycle {}, phase {}): {} only-real, {} only-model, bbox {:?}", hi / t, hi % t, a.len(), b.len(), bb);
    for (k, i) in ph.insts.iter().enumerate() {
        let ib = i.bbox();
        if ib.0 - 150 <= bb.2 && bb.0 <= ib.2 + 150 && ib.1 - 150 <= bb.3 && bb.1 <= ib.3 + 150 {
            println!("  inst {k}: {:?} {:?} bbox {:?} at ({},{}) contact {} settled {} trigger {:?} group {}", i.oriented().kind, i.oriented().iso, i.bbox(), i.tx, i.ty, i.t_contact(), i.t_settled(), i.trigger.map(|s| ph.sigs[s as usize]), i.group);
        }
    }
    let g = hi % t;
    for (k, l) in ph.legs.iter().enumerate() {
        for gg in [g, g + t, g - t] {
            if l.t0 > gg + 3000 || l.t1 < gg - 3000 {
                continue;
            }
            let (x, y) = l.traj.pos_at(gg);
            if (x as i64) >= bb.0 - 400
                && (x as i64) <= bb.2 + 400
                && (y as i64) >= bb.1 - 400
                && (y as i64) <= bb.3 + 400
            {
                println!(
                    "  leg {k}: {:?} [{}, {}) sig {:?} (rel gen {gg})",
                    l.traj, l.t0, l.t1, ph.sigs[l.sig as usize]
                );
            }
        }
    }
    for (k, cs) in ph.crosses.iter().enumerate() {
        if (cs.tx - bb.0).abs() < 80 && (cs.ty - bb.1).abs() < 80 {
            println!(
                "  cross {k} at ({},{}) window [{}, {})",
                cs.tx, cs.ty, cs.t0, cs.t1
            );
        }
    }
    println!("  only-real: {:?}", &a[..a.len().min(12)]);
    println!("  only-model: {:?}", &b[..b.len().min(12)]);
}
