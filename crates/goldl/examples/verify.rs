//! Cell-level verification of a compiled design: run the emitted Life pattern with HashLife
//! for a few clock cycles of random inputs and compare it, at checkpoints, with the exact
//! reconstruction from the logic simulation.
//!
//! Usage: cargo run --release -p goldl --example verify -- FILE [CYCLES] [SEED]

use goldl::driver::{compile, Options};
use goldl::rtl::RtlSim;
use goldl::sim::{reconstruct_all, TraceSignals};
use goldl_life::hashlife::HashLife;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("usage: verify FILE [CYCLES] [SEED]");
    let cycles: usize = args.get(2).map_or(3, |s| s.parse().unwrap());
    let mut seed: u64 = args.get(3).map_or(1, |s| s.parse().unwrap());
    let src = std::fs::read_to_string(path).unwrap();
    let t0 = Instant::now();
    let c = compile(&src, &Options::default()).unwrap_or_else(|(d, _)| panic!("{d:?}"));
    let lay = c.layout.as_ref().unwrap_or_else(|| panic!("layout: {:?}", c.stats.layout_error));
    let ph = &lay.phys;
    eprintln!("compiled in {:?}: {} components, period {}", t0.elapsed(), ph.insts.len(), ph.period);
    let mut rng = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let inputs: Vec<Vec<u64>> = (0..cycles).map(|_| c.rtl.inputs.iter().map(|p| rng() & mask(p.width)).collect()).collect();
    let mut bits: Vec<Vec<Vec<bool>>> = inputs
        .iter()
        .map(|v| c.rtl.inputs.iter().enumerate().map(|(p, port)| (0..port.width).map(|b| (v[p] >> b) & 1 == 1).collect()).collect())
        .collect();
    let idle: Vec<Vec<bool>> = c.rtl.inputs.iter().map(|p| vec![false; p.width as usize]).collect();
    bits.push(idle.clone());
    bits.push(idle);
    let trace = TraceSignals::record(&c.gnl, bits);
    let mut rs = RtlSim::new(&c.rtl);
    let mut gs = goldl::gnl::GnlSim::new(&c.gnl);
    for v in &inputs {
        let ro = rs.step(&c.rtl, v);
        let gi: Vec<Vec<u64>> = c.rtl.inputs.iter().enumerate().map(|(p, port)| (0..port.width).map(|b| (v[p] >> b) & 1).collect()).collect();
        let go = gs.step(&c.gnl, &gi);
        for (p, &x) in ro.iter().enumerate() {
            let g: u64 = go[p].iter().enumerate().map(|(b, w)| (w & 1) << b).sum();
            assert_eq!(g, x, "GNL vs RTL output {p}");
        }
    }
    let t1 = Instant::now();
    let p0 = reconstruct_all(ph, 0, &trace);
    eprintln!("pattern: {} cells ({:?})", p0.len(), t1.elapsed());
    let mut u = HashLife::from_pattern(&p0);
    let t = ph.period;
    let mut now = 0i64;
    for k in 1..=(2 * cycles as i64) {
        let cp = k * t / 2 + 13;
        let t2 = Instant::now();
        u.step((cp - now) as u64);
        now = cp;
        let real = u.to_pattern();
        let model = reconstruct_all(ph, cp, &trace);
        if real != model {
            let rs = real.to_set();
            let ms = model.to_set();
            let only_real: Vec<_> = rs.difference(&ms).copied().take(8).collect();
            let only_model: Vec<_> = ms.difference(&rs).copied().take(8).collect();
            panic!("mismatch at generation {cp}: real {} cells, model {}; only real {only_real:?}, only model {only_model:?}", real.len(), model.len());
        }
        eprintln!("generation {cp}: {} cells match ({:?})", real.len(), t2.elapsed());
    }
    eprintln!("verified {cycles} cycles");
}

fn mask(w: u32) -> u64 {
    if w >= 64 {
        !0
    } else {
        (1u64 << w) - 1
    }
}
