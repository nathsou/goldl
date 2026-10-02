//! Exported patterns are self-contained: constant streams come from a gun inside the pattern,
//! so the pattern keeps computing in plain Life after its input tape has run out (inputs then
//! read as 0). Checked cell by cell with HashLife against the reconstruction.

use goldl::session::Session;
use goldl::sim::Rect;
use goldl_life::hashlife::HashLife;
use goldl_life::Pattern;

/// Export at generation 0 with `tape` cycles of inputs, run `cycles` clock cycles of real Life,
/// and compare with the logic model at the middle of every cycle.
fn standalone(src: &str, inputs: &[(usize, u64, usize)], tape: usize, cycles: i64) {
    let mut s = Session::new(src, None).unwrap_or_else(|d| panic!("{d:?}"));
    for &(port, value, from) in inputs {
        s.set_input(port, value, from);
    }
    // Inputs read 0 once the tape is over.
    for p in 0..s.c.rtl.inputs.len() {
        s.set_input(p, 0, tape);
    }
    s.ensure(tape);
    let p0 = Pattern::from_cells(s.cells(0, Rect::ALL));
    let t = s.period();
    s.ensure(cycles as usize + 4);
    let mut h = HashLife::from_pattern(&p0);
    let mut done = 0;
    for c in 0..cycles {
        let g = c * t + t / 2;
        h.step((g - done) as u64);
        done = g;
        let real = h.to_pattern().to_set();
        let model = Pattern::from_cells(s.cells(g, Rect::ALL)).to_set();
        let diff = real.symmetric_difference(&model).count();
        assert_eq!(
            diff,
            0,
            "cycle {c}: {diff} cells differ ({} real, {} model)",
            real.len(),
            model.len()
        );
    }
}

#[test]
fn counter_runs_past_its_tape() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/counter.goldl"
    ))
    .unwrap();
    // en = 1 for 3 cycles, then the tape ends: the counter must hold its value.
    standalone(&src, &[(0, 1, 0)], 3, 8);
}

#[test]
fn inverted_inputs_and_constants() {
    // `~a` and constants exercise the inverted-input and constant-one paths.
    let src = "module M(a: bit, b: bit) -> (x: bit, y: bit) {\n  reg r: bit = 0\n  r.next = ~r\n  x = ~a & r\n  y = a | ~b\n}\n";
    standalone(src, &[(0, 1, 1), (1, 1, 2)], 4, 7);
}

#[test]
fn a_seeded_register_can_be_cleared_to_constant_zero() {
    standalone(
        "module Clear() -> (q: bit) {\n reg r: bit = 1\n r.next = 0\n q = r\n}",
        &[],
        0,
        4,
    );
}

#[test]
fn register_bank_with_different_launch_phases() {
    // Different read cones give the feedback lanes different required phases.
    // Every bit starts live, then repeatedly changes while other lanes hold.
    let src = "module Bank(en: bit) -> (x: bits<16>, y: bits<16>, z: bit) {
        reg a: bits<16> = 65535
        reg b: bits<16> = 43690
        a.next = if en { a + 7 } else { b ^ a }
        b.next = if en { b ^ a } else { b + 1 }
        x = a + b
        y = (a ^ b) + (a & b)
        z = parity(a)
    }";
    standalone(src, &[(0, 1, 0), (0, 0, 2), (0, 1, 4)], 6, 8);
}
