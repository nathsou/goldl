//! Return-path timing census, including the self-sustaining constant supplies.
//! Usage: cargo run --release -p goldl --example loop_stats -- FILE [TOP]
use goldl::{
    driver::{compile, Options},
    phys::SigKind,
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let src = std::fs::read_to_string(args.get(1).expect("usage: loop_stats FILE [TOP]")).unwrap();
    let c = compile(
        &src,
        &Options {
            top: args.get(2).cloned(),
            ..Options::default()
        },
    )
    .unwrap_or_else(|(d, _)| panic!("{d:?}"));
    let p = &c.layout.as_ref().expect("layout").phys;
    let mut ranges = vec![None::<(i64, i64)>; p.sigs.len()];
    for leg in &p.legs {
        if matches!(p.sigs[leg.sig as usize], SigKind::RegNext { .. }) {
            let r = ranges[leg.sig as usize].get_or_insert((leg.t0, leg.t1));
            r.0 = r.0.min(leg.t0);
            r.1 = r.1.max(leg.t1);
        }
    }
    let mut spans: Vec<_> = ranges.into_iter().flatten().map(|(a, b)| b - a).collect();
    spans.sort_unstable();
    println!("period={}, loops={}", p.period, spans.len());
    if !spans.is_empty() {
        println!(
            "return elapsed min/median/max={}/{}/{} generations; median {:.1}% of period",
            spans[0],
            spans[spans.len() / 2],
            spans[spans.len() - 1],
            100.0 * spans[spans.len() / 2] as f64 / p.period as f64
        );
    }
}
