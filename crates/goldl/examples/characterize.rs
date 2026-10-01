use goldl::tech::glider::*;
use goldl_life::{Pattern, Universe};

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    for name in names {
        let src = std::fs::read_to_string(format!("crates/goldl/tech/{name}.rle")).unwrap();
        let p = Pattern::parse_rle(&src).unwrap();
        let (ins, stat) = extract_gliders(&p, 0);
        let still = stat.run(1) == stat;
        println!("== {name}: {} cells, bbox {:?}, still={still}", stat.len(), stat.bbox());
        for g in &ins { println!("   input {} lane={} phi={}", g.dir.name(), g.lane, g.phi); }
        let mut u = Universe::from_pattern(&p);
        let n = 1500;
        u.step(n);
        let (outs, rest) = extract_gliders(&u.to_pattern(), n as i64);
        println!("   restored={}", rest == stat);
        for g in &outs { println!("   output {} lane={} phi={}", g.dir.name(), g.lane, g.phi); }
    }
}
