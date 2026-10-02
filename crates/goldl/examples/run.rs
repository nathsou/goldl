//! Run a design in the RTL simulator and print its outputs every cycle.
//!
//! Usage: cargo run --release -p goldl --example run -- FILE [CYCLES] [INPUT...]

use goldl::rtl::RtlSim;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let cycles: usize = args.get(2).map_or(20, |s| s.parse().unwrap());
    let ins: Vec<u64> = args.iter().skip(3).map(|s| s.parse().unwrap()).collect();
    let (rtl, _) = goldl::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
    let mut s = RtlSim::new(&rtl);
    let names: Vec<&str> = rtl.outputs.iter().map(|(p, _)| p.name.as_str()).collect();
    for c in 0..cycles {
        let o = s.step(&rtl, &ins);
        let cols: Vec<String> = names
            .iter()
            .zip(&o)
            .map(|(n, v)| format!("{n}={v}"))
            .collect();
        println!("{c:4}: {}", cols.join(" "));
    }
}
