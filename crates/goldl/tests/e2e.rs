//! Full pipeline: GoLDL source → Life pattern, verified cell-by-cell with the Life engine
//! against the exact reconstruction, and output values checked against the RTL simulator.

use goldl::driver::{compile, Options};
use goldl::rtl::RtlSim;
use goldl::sim::{reconstruct_all, TraceSignals};
use goldl_life::Universe;

fn run(src: &str, inputs: Vec<Vec<u64>>, label: &str) {
    let c = compile(src, &Options::default()).unwrap_or_else(|(d, _)| panic!("{label}: {d:?}"));
    let lay = c
        .layout
        .as_ref()
        .unwrap_or_else(|| panic!("{label}: layout: {:?}", c.stats.layout_error));
    let ph = &lay.phys;
    // Input bits per cycle.
    let n = inputs.len();
    let mut bits: Vec<Vec<Vec<bool>>> = inputs
        .iter()
        .map(|v| {
            c.rtl
                .inputs
                .iter()
                .enumerate()
                .map(|(p, port)| (0..port.width).map(|b| (v[p] >> b) & 1 == 1).collect())
                .collect()
        })
        .collect();
    let idle: Vec<Vec<bool>> = c
        .rtl
        .inputs
        .iter()
        .map(|p| vec![false; p.width as usize])
        .collect();
    bits.push(idle.clone());
    bits.push(idle);
    let trace = TraceSignals::record(&c.gnl, bits);
    // GNL outputs must match RTL outputs.
    let mut rs = RtlSim::new(&c.rtl);
    let mut gs = goldl::gnl::GnlSim::new(&c.gnl);
    for v in &inputs {
        let ro = rs.step(&c.rtl, v);
        let gi: Vec<Vec<u64>> = c
            .rtl
            .inputs
            .iter()
            .enumerate()
            .map(|(p, port)| (0..port.width).map(|b| (v[p] >> b) & 1).collect())
            .collect();
        let go = gs.step(&c.gnl, &gi);
        for (p, &x) in ro.iter().enumerate() {
            let g: u64 = go[p].iter().enumerate().map(|(b, w)| (w & 1) << b).sum();
            assert_eq!(g, x, "{label}: GNL vs RTL output {p}");
        }
    }
    let p0 = reconstruct_all(ph, 0, &trace);
    let mut u = Universe::from_pattern(&p0);
    let t = ph.period;
    let mut now = 0;
    for k in 1..=(2 * n as i64) {
        let cp = k * t / 2 + 13;
        u.step((cp - now) as u64);
        now = cp;
        let real = u.to_pattern();
        let model = reconstruct_all(ph, cp, &trace);
        assert!(
            real == model,
            "{label}: cell mismatch at generation {cp} (period {t}): real {} cells, model {} cells",
            real.len(),
            model.len()
        );
    }
}

#[test]
fn half_adder() {
    let src = "module HalfAdder(a: bit, b: bit) -> (sum: bit, carry: bit) {\n  sum = a ^ b\n  carry = a & b\n}\n";
    run(
        src,
        vec![vec![0, 0], vec![1, 0], vec![0, 1], vec![1, 1]],
        "half_adder",
    );
}

#[test]
fn counter2() {
    let src = "module C(en: bit) -> (q: bits<2>) {\n  reg r: bits<2> = 0\n  r.next = if en { r + 1 } else { r }\n  q = r\n}\n";
    run(
        src,
        vec![vec![1], vec![1], vec![0], vec![1], vec![1]],
        "counter2",
    );
}
