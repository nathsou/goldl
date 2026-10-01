use goldl::driver::{compile, Options};
use goldl::sim::{reconstruct_all, TraceSignals};
use goldl_life::Universe;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let ins: Vec<u64> = std::env::args().skip(2).map(|s| s.parse().unwrap()).collect();
    let src = std::fs::read_to_string(&path).unwrap();
    let c = compile(&src, &Options::default()).unwrap_or_else(|(d, _)| panic!("{d:?}"));
    let lay = c.layout.as_ref().expect("layout");
    let ph = &lay.phys;
    let nin = c.rtl.inputs.len();
    let cycles: Vec<Vec<u64>> = ins.chunks(nin.max(1)).map(|c| c.to_vec()).collect();
    let mut bits: Vec<Vec<Vec<bool>>> = cycles.iter().map(|v| c.rtl.inputs.iter().enumerate().map(|(p, port)| (0..port.width).map(|b| (v.get(p).copied().unwrap_or(0) >> b) & 1 == 1).collect()).collect()).collect();
    for _ in 0..3 { bits.push(c.rtl.inputs.iter().map(|p| vec![false; p.width as usize]).collect()); }
    let trace = TraceSignals::record(&c.gnl, bits);
    println!("period {} insts {} legs {} crosses {}", ph.period, ph.insts.len(), ph.legs.len(), ph.crosses.len());
    let mut u = Universe::from_pattern(&reconstruct_all(ph, 0, &trace));
    let limit = ph.period * (cycles.len() as i64 + 1);
    let mut g = 0i64;
    while g < limit {
        u.step(1);
        g += 1;
        let real = u.to_pattern();
        let model = reconstruct_all(ph, g, &trace);
        if real != model {
            let rs = real.to_set(); let ms = model.to_set();
            let a: Vec<_> = rs.difference(&ms).copied().collect();
            let b: Vec<_> = ms.difference(&rs).copied().collect();
            let all: Vec<(i64,i64)> = a.iter().chain(b.iter()).copied().collect();
            let bb = goldl_life::Pattern::from_cells(all.clone()).bbox().unwrap();
            println!("first divergence at generation {g}: {} only-real, {} only-model, bbox {:?}", a.len(), b.len(), bb);
            for (k, i) in ph.insts.iter().enumerate() {
                let ib = i.bbox();
                if ib.0 - 20 <= bb.2 && bb.0 <= ib.2 + 20 && ib.1 - 20 <= bb.3 && bb.1 <= ib.3 + 20 {
                    println!("  inst {k}: {:?} iso {:?} at ({},{}) dt {} contact {} settled {} trigger {:?}", i.oriented().kind, i.oriented().iso, i.tx, i.ty, i.dt, i.t_contact(), i.t_settled(), i.trigger);
                }
            }
            for (k, l) in ph.legs.iter().enumerate() {
                let (x, y) = l.traj.pos_at(g);
                if (x as i64) >= bb.0 - 10 && (x as i64) <= bb.2 + 10 && (y as i64) >= bb.1 - 10 && (y as i64) <= bb.3 + 10 && l.t0 <= g + 100 && l.t1 >= g - 100 {
                    println!("  leg {k}: {:?} [{}, {}) sig {}", l.traj, l.t0, l.t1, l.sig);
                }
            }
            for (k, cs) in ph.crosses.iter().enumerate() {
                if (cs.tx - bb.0).abs() < 60 && (cs.ty - bb.1).abs() < 60 { println!("  cross {k} at ({},{}) window [{}, {})", cs.tx, cs.ty, cs.t0, cs.t1); }
            }
            for &(px, py) in all.iter().take(4) {
                for (k, i) in ph.insts.iter().enumerate() {
                    let ib = i.bbox();
                    if px >= ib.0 - 3 && px <= ib.2 + 3 && py >= ib.1 - 3 && py <= ib.3 + 3 {
                        println!("  point ({px},{py}) near inst {k} {:?} bbox {:?}", i.oriented().kind, ib);
                    }
                }
            }
            println!("  only-real: {:?}", &a[..a.len().min(12)]);
            println!("  only-model: {:?}", &b[..b.len().min(12)]);
            return;
        }
    }
    println!("no divergence up to {limit}");
}
