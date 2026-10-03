//! Reproducible physical quality-of-results census (core only, excluding input tapes).
//! cargo run --release -p goldl --example qor -- examples/*.goldl

use goldl::driver::{compile, Options};
use goldl::tech::component::Kind;

fn main() {
    println!("design,width,height,area,period,static_cells,components,crossings,splitters,delays,snarks,cc_reflectors,duplicators,eaters");
    for path in std::env::args().skip(1) {
        let src = std::fs::read_to_string(&path).expect("read design");
        let c = compile(&src, &Options::default()).unwrap_or_else(|(d, _)| panic!("{path}: {d:?}"));
        let lay = c
            .layout
            .as_ref()
            .unwrap_or_else(|| panic!("{path}: {:?}", c.stats.layout_error));
        let s = &c.stats;
        let count = |kind| {
            lay.phys
                .insts
                .iter()
                .filter(|i| i.oriented().kind == kind)
                .count()
        };
        println!(
            "{path},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            s.width,
            s.height,
            s.width as i128 * s.height as i128,
            s.period,
            s.cells,
            s.components,
            s.gnl.cross,
            s.gnl.split,
            s.gnl.delay,
            count(Kind::Snark),
            count(Kind::Cc),
            count(Kind::Dup),
            count(Kind::Eater)
        );
    }
}
