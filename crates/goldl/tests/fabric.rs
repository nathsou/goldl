//! End-to-end physical verification: lay out small glider netlists, emit the pattern at
//! generation 0, run the real Life engine, and compare with the exact reconstruction at
//! several later generations.

use goldl::gnl::{Gnl, Op, PortInfo, RegInfo};
use goldl::layout::layout;
use goldl::sim::{reconstruct_all, TraceSignals};
use goldl_life::{Pattern, Universe};

fn check(g: &Gnl, inputs: Vec<Vec<Vec<bool>>>, label: &str) {
    let (g, lr) = layout(g).unwrap_or_else(|e| panic!("{label}: {e}"));
    let g = &g;
    let ph = &lr.phys;
    let n = inputs.len() as i64;
    // Record two extra idle cycles so the model knows what follows the last checkpoint.
    let mut ext = inputs.clone();
    let idle: Vec<Vec<bool>> = g
        .inputs
        .iter()
        .map(|p| vec![false; p.width as usize])
        .collect();
    ext.push(idle.clone());
    ext.push(idle);
    let trace = TraceSignals::record(g, ext);
    let p0 = reconstruct_all(ph, 0, &trace);
    let mut u = Universe::from_pattern(&p0);
    let t = ph.period;
    let checkpoints: Vec<i64> = (1..=2 * n).map(|k| k * t / 2 + 17).collect();
    let mut now = 0;
    for &cp in &checkpoints {
        u.step((cp - now) as u64);
        now = cp;
        let real = u.to_pattern();
        let model = reconstruct_all(ph, cp, &trace);
        if real != model {
            let rs = real.to_set();
            let ms = model.to_set();
            let only_real: Vec<_> = rs.difference(&ms).copied().collect();
            let only_model: Vec<_> = ms.difference(&rs).copied().collect();
            let bb = |v: &Vec<(i64, i64)>| Pattern::from_cells(v.clone()).bbox();
            panic!(
                "{label}: mismatch at generation {cp} (period {t}): {} cells only in real ({:?}), {} only in model ({:?})",
                only_real.len(),
                bb(&only_real),
                only_model.len(),
                bb(&only_model)
            );
        }
    }
}

fn io(g: &mut Gnl, ins: &[(&str, u32)], outs: &[(&str, u32)]) {
    for (n, w) in ins {
        g.inputs.push(PortInfo {
            name: n.to_string(),
            width: *w,
        });
    }
    for (n, w) in outs {
        g.outputs.push(PortInfo {
            name: n.to_string(),
            width: *w,
        });
    }
}

fn bits(v: &[u8]) -> Vec<Vec<Vec<bool>>> {
    v.iter()
        .map(|&x| vec![vec![x & 1 == 1], vec![x & 2 == 2]])
        .collect()
}

#[test]
fn wire() {
    let mut g = Gnl::new();
    io(&mut g, &[("a", 1)], &[("y", 1)]);
    let a = g.add(
        Op::In {
            port: 0,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    g.add(Op::Out { port: 0, bit: 0 }, &[g.out(a, 0)], 0);
    g.sink_dangling();
    g.mark_zero_nets();
    check(
        &g,
        vec![vec![vec![true]], vec![vec![false]], vec![vec![true]]],
        "wire",
    );
}

#[test]
fn not_gate() {
    let mut g = Gnl::new();
    io(&mut g, &[("a", 1)], &[("y", 1)]);
    let a = g.add(
        Op::In {
            port: 0,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    let one = g.add(Op::One, &[], 0);
    let x = g.add(Op::Cross, &[g.out(one, 0), g.out(a, 0)], 0);
    g.add(Op::Out { port: 0, bit: 0 }, &[g.out(x, 0)], 0);
    g.sink_dangling();
    g.mark_zero_nets();
    check(
        &g,
        vec![
            vec![vec![true]],
            vec![vec![false]],
            vec![vec![true]],
            vec![vec![false]],
        ],
        "not",
    );
}

#[test]
fn inhibit_both_outputs() {
    // y0 = a & !b, y1 = b & !a
    let mut g = Gnl::new();
    io(&mut g, &[("a", 1), ("b", 1)], &[("y", 2)]);
    let a = g.add(
        Op::In {
            port: 0,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    let b = g.add(
        Op::In {
            port: 1,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    let x = g.add(Op::Cross, &[g.out(a, 0), g.out(b, 0)], 0);
    g.add(Op::Out { port: 0, bit: 0 }, &[g.out(x, 0)], 0);
    g.add(Op::Out { port: 0, bit: 1 }, &[g.out(x, 1)], 0);
    g.sink_dangling();
    g.mark_zero_nets();
    check(&g, bits(&[0, 1, 2, 3]), "inhibit");
}

#[test]
fn split_and() {
    // y = a & b = a & !(!b), with fan-out of `a` to two outputs.
    let mut g = Gnl::new();
    io(&mut g, &[("a", 1), ("b", 1)], &[("y", 2)]);
    let a = g.add(
        Op::In {
            port: 0,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    let b = g.add(
        Op::In {
            port: 1,
            bit: 0,
            inv: false,
        },
        &[],
        0,
    );
    let one = g.add(Op::One, &[], 0);
    let sa = g.add(Op::Split, &[g.out(a, 0)], 0);
    let nb = g.add(Op::Cross, &[g.out(one, 0), g.out(b, 0)], 0);
    let y = g.add(Op::Cross, &[g.out(sa, 0), g.out(nb, 0)], 0);
    g.add(Op::Out { port: 0, bit: 0 }, &[g.out(y, 0)], 0);
    g.add(Op::Out { port: 0, bit: 1 }, &[g.out(sa, 1)], 0);
    g.sink_dangling();
    g.mark_zero_nets();
    check(&g, bits(&[3, 1, 2, 3, 0]), "split_and");
}

#[test]
fn toggle_register() {
    // q' = !q, output q.
    let mut g = Gnl::new();
    io(&mut g, &[], &[("q", 1)]);
    g.regs.push(RegInfo {
        name: "t".into(),
        width: 1,
        init: vec![true],
    });
    let q = g.add(Op::RegQ { reg: 0, bit: 0 }, &[], 0);
    let one = g.add(Op::One, &[], 0);
    let sq = g.add(Op::Split, &[g.out(q, 0)], 0);
    let nq = g.add(Op::Cross, &[g.out(one, 0), g.out(sq, 0)], 0);
    g.add(Op::RegD { reg: 0, bit: 0 }, &[g.out(nq, 0)], 0);
    g.add(Op::Out { port: 0, bit: 0 }, &[g.out(sq, 1)], 0);
    g.sink_dangling();
    g.mark_zero_nets();
    check(&g, vec![vec![]; 4], "toggle");
}
