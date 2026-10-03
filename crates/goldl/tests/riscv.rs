//! Architectural RV32I checks plus RTL/AIG/glider-netlist equivalence on every cycle.
//! Memory responses are driven explicitly, independently of the built-in demo.
use goldl::{aig, elab, gnl, map, rtl};
use std::collections::HashMap;

const SRC: &str = include_str!("../../../examples/riscv.goldl");
type Outputs = HashMap<String, u64>;

struct Core {
    rtl: rtl::Rtl,
    aig: aig::Aig,
    gnl: gnl::Gnl,
    sim: rtl::RtlSim,
    gliders: gnl::GnlSim,
    inputs: Vec<u64>,
}
impl Core {
    fn new() -> Self {
        Self::with_top("RV32I")
    }
    fn with_top(top: &str) -> Self {
        let (rtl, _) = elab::elaborate(SRC, Some(top)).unwrap_or_else(|e| panic!("{:?}", e.diags));
        let aig = aig::bitblast(&rtl);
        let gnl = map::map(&aig, &rtl);
        let mut c = Self {
            sim: rtl::RtlSim::new(&rtl),
            gliders: gnl::GnlSim::new(&gnl),
            inputs: vec![0; rtl.inputs.len()],
            rtl,
            aig,
            gnl,
        };
        for name in ["enable", "instr_ready", "mem_ready"] {
            if c.rtl.inputs.iter().any(|p| p.name == name) {
                c.set(name, 1);
            }
        }
        c
    }
    fn set(&mut self, name: &str, value: u64) {
        let p = self.rtl.inputs.iter().position(|p| p.name == name).unwrap();
        self.inputs[p] = value;
    }
    fn peek(&mut self) -> Outputs {
        self.sim.eval(&self.rtl, &self.inputs);
        self.rtl
            .outputs
            .iter()
            .map(|(p, id)| (p.name.clone(), self.sim.vals[*id as usize]))
            .collect()
    }
    fn clock(&mut self) -> Outputs {
        let before = self.peek();
        let bits: Vec<Vec<u64>> = self
            .rtl
            .inputs
            .iter()
            .zip(&self.inputs)
            .map(|(p, v)| (0..p.width).map(|b| (v >> b) & 1).collect())
            .collect();
        let latch: Vec<u64> = self
            .aig
            .latches
            .iter()
            .map(|l| (self.sim.regs[l.reg as usize] >> l.bit) & 1)
            .collect();
        let av = self.aig.eval(&bits, &latch);
        for &(p, b, l) in &self.aig.outputs {
            assert_eq!(
                aig::Aig::lit_val(&av, l) & 1,
                (before[&self.rtl.outputs[p as usize].0.name] >> b) & 1
            );
        }
        let go = self.gliders.step(&self.gnl, &bits);
        let ro = self.sim.step(&self.rtl, &self.inputs);
        for (p, &value) in ro.iter().enumerate() {
            let gv = go[p]
                .iter()
                .enumerate()
                .map(|(b, v)| (v & 1) << b)
                .sum::<u64>();
            assert_eq!(
                gv, value,
                "{} at cycle {}",
                self.rtl.outputs[p].0.name, self.sim.cycle
            );
        }
        before
    }
    fn exec(&mut self, ins: u32) -> Outputs {
        self.set("instr", ins as u64);
        self.clock()
    }
    fn init(&mut self, reg: u32, value: u32) {
        let upper = value.wrapping_add(0x800) & 0xfffff000;
        assert_eq!(self.exec(upper | reg << 7 | 0x37)["retired"], 1);
        assert_eq!(self.exec(i(0x13, 0, reg, reg, value as i32))["retired"], 1);
    }
    fn check(&mut self, ins: u32, result: u32) {
        let o = self.exec(ins);
        assert_eq!(o["retired"], 1, "instruction {ins:08x}");
        assert_eq!(o["wb_data"], result as u64, "instruction {ins:08x}");
        assert_eq!(o["wb_valid"], ((ins >> 7) & 31 != 0) as u64);
        assert_eq!(o["wb_rd"], ((ins >> 7) & 31) as u64);
        assert_eq!(self.peek()["trap"], 0);
    }
    fn fault(&mut self, ins: u32, cause: u64, tval: u32) {
        let pc = self.peek()["pc"];
        let o = self.exec(ins);
        assert_eq!(o["retired"], 0);
        assert_eq!(o["wb_valid"], 0);
        let o = self.peek();
        assert_eq!(
            (o["trap"], o["cause"], o["tval"], o["epc"], o["pc"]),
            (1, cause, tval as u64, pc, pc),
            "instruction {ins:08x}"
        );
        assert_eq!(o["mem_valid"], 0);
        assert_eq!(o["fetch_valid"], 0);
        self.clock();
        assert_eq!(self.peek()["epc"], pc);
        self.set("reset", 1);
        self.clock();
        self.set("reset", 0);
        assert_eq!((self.peek()["trap"], self.peek()["pc"]), (0, 0));
    }
}
fn r(f3: u32, f7: u32, rd: u32, a: u32, b: u32) -> u32 {
    f7 << 25 | b << 20 | a << 15 | f3 << 12 | rd << 7 | 0x33
}
fn i(op: u32, f3: u32, rd: u32, a: u32, imm: i32) -> u32 {
    (imm as u32 & 0xfff) << 20 | a << 15 | f3 << 12 | rd << 7 | op
}
fn s(f3: u32, a: u32, b: u32, imm: i32) -> u32 {
    let n = imm as u32 & 0xfff;
    (n >> 5) << 25 | b << 20 | a << 15 | f3 << 12 | (n & 31) << 7 | 0x23
}
fn branch(f3: u32, a: u32, b: u32, imm: i32) -> u32 {
    let n = imm as u32;
    ((n >> 12) & 1) << 31
        | ((n >> 5) & 63) << 25
        | b << 20
        | a << 15
        | f3 << 12
        | ((n >> 1) & 15) << 8
        | ((n >> 11) & 1) << 7
        | 0x63
}
fn jal(rd: u32, imm: i32) -> u32 {
    let n = imm as u32;
    ((n >> 20) & 1) << 31
        | ((n >> 1) & 1023) << 21
        | ((n >> 11) & 1) << 20
        | ((n >> 12) & 255) << 12
        | rd << 7
        | 0x6f
}

#[test]
fn all_register_and_immediate_alu_instructions() {
    let mut c = Core::new();
    let pairs = [
        (0u32, 0u32),
        (u32::MAX, 1),
        (0x80000000, 31),
        (0x7fffffff, 0x80000000),
        (0x87654321, 0x12345678),
        (1, 32),
    ];
    for (a, b) in pairs {
        c.init(1, a);
        c.init(2, b);
        for (f3, f7, value) in [
            (0, 0, a.wrapping_add(b)),
            (0, 32, a.wrapping_sub(b)),
            (1, 0, a.wrapping_shl(b & 31)),
            (2, 0, ((a as i32) < (b as i32)) as u32),
            (3, 0, (a < b) as u32),
            (4, 0, a ^ b),
            (5, 0, a >> (b & 31)),
            (5, 32, ((a as i32) >> (b & 31)) as u32),
            (6, 0, a | b),
            (7, 0, a & b),
        ] {
            c.check(r(f3, f7, 3, 1, 2), value);
        }
        for imm in [-2048i32, -1, 0, 1, 2047] {
            let b = imm as u32;
            for (f3, value) in [
                (0, a.wrapping_add(b)),
                (2, ((a as i32) < imm) as u32),
                (3, (a < b) as u32),
                (4, a ^ b),
                (6, a | b),
                (7, a & b),
            ] {
                c.check(i(0x13, f3, 3, 1, imm), value);
            }
        }
        for sh in [0, 1, 7, 16, 31] {
            c.check(i(0x13, 1, 3, 1, sh), a << sh);
            c.check(i(0x13, 5, 3, 1, sh), a >> sh);
            c.check(i(0x13, 5, 3, 1, 0x400 | sh), ((a as i32) >> sh) as u32);
        }
    }
}

#[test]
fn all_registers_and_zero_register() {
    let mut c = Core::new();
    for reg in 1..32 {
        c.init(reg, 0x9e3779b9u32.wrapping_mul(reg));
    }
    c.exec(i(0x13, 0, 0, 0, -1));
    c.check(i(0x13, 0, 0, 0, 0), 0);
    for reg in 1..32 {
        c.check(i(0x13, 0, 0, reg, 0), 0x9e3779b9u32.wrapping_mul(reg));
    }
}

#[test]
fn upper_immediates_jumps_and_all_branches() {
    let mut c = Core::new();
    c.check(0x89abc1b7, 0x89abc000); // LUI x3
    let pc = c.peek()["pc"] as u32;
    c.check(0xfffff197, pc.wrapping_add(0xfffff000)); // AUIPC x3
    for off in [-1048576, -4, 4, 1048572] {
        let pc = c.peek()["pc"] as u32;
        c.check(jal(3, off), pc.wrapping_add(4));
        assert_eq!(c.peek()["pc"], pc.wrapping_add(off as u32) as u64);
    }
    c.init(1, 0x1001);
    let pc = c.peek()["pc"] as u32;
    c.check(i(0x67, 0, 1, 1, 4), pc.wrapping_add(4)); // rd==rs1; clear bit zero
    assert_eq!(c.peek()["pc"], 0x1004);
    for (a, b) in [
        (0u32, 0u32),
        (1, 2),
        (u32::MAX, 0),
        (0x80000000, 0x7fffffff),
    ] {
        c.init(1, a);
        c.init(2, b);
        for (f3, taken) in [
            (0, a == b),
            (1, a != b),
            (4, (a as i32) < (b as i32)),
            (5, (a as i32) >= (b as i32)),
            (6, a < b),
            (7, a >= b),
        ] {
            for off in [-4096, -4, 12, 4092] {
                let pc = c.peek()["pc"] as u32;
                assert_eq!(c.exec(branch(f3, 1, 2, off))["retired"], 1);
                assert_eq!(
                    c.peek()["pc"],
                    pc.wrapping_add(if taken { off as u32 } else { 4 }) as u64
                );
            }
        }
    }
    c.init(1, 1);
    c.init(2, 2);
    assert_eq!(c.exec(branch(0, 1, 2, 2))["retired"], 1); // untaken misaligned target is legal
    assert_eq!(c.peek()["trap"], 0);
}

#[test]
fn all_loads_stores_and_byte_lanes() {
    let mut c = Core::new();
    let word = 0x80ff7f01u32;
    c.set("rdata", word as u64);
    for offset in 0..4 {
        c.init(1, 0x1000 + offset);
        for f3 in [0, 1, 2, 4, 5] {
            let size = match f3 {
                0 | 4 => 1,
                1 | 5 => 2,
                _ => 4,
            };
            if offset % size != 0 {
                continue;
            }
            let shifted = word >> (8 * offset);
            let want = match f3 {
                0 => (shifted as i8 as i32) as u32,
                1 => (shifted as i16 as i32) as u32,
                4 => shifted & 255,
                5 => shifted & 65535,
                _ => shifted,
            };
            let o = c.exec(i(3, f3, 3, 1, 0));
            assert_eq!(
                (
                    o["mem_valid"],
                    o["mem_write"],
                    o["mem_addr"],
                    o["wb_data"],
                    o["retired"]
                ),
                (1, 0, (0x1000 + offset) as u64, want as u64, 1)
            );
        }
        c.init(2, 0x12345678);
        for f3 in 0..3 {
            let size = 1 << f3;
            if offset % size != 0 {
                continue;
            }
            let o = c.exec(s(f3, 1, 2, 0));
            assert_eq!(
                (
                    o["mem_valid"],
                    o["mem_write"],
                    o["mem_addr"],
                    o["wdata"],
                    o["wstrb"],
                    o["retired"],
                    o["wb_valid"]
                ),
                (
                    1,
                    1,
                    (0x1000 + offset) as u64,
                    0x12345678u32.wrapping_shl(8 * offset) as u64,
                    (((1 << size) - 1) << offset) as u64,
                    1,
                    0
                )
            );
        }
    }
    c.init(1, 0x1004);
    let o = c.exec(s(2, 1, 2, -4));
    assert_eq!(o["mem_addr"], 0x1000);
    c.check(i(3, 2, 3, 1, -4), word);
}

#[test]
fn stalls_pause_and_fence() {
    let mut c = Core::new();
    c.init(1, 0x1000);
    c.init(2, 0x42);
    c.set("instr", s(2, 1, 2, 0) as u64);
    c.set("mem_ready", 0);
    let before = c.peek();
    for _ in 0..4 {
        let o = c.clock();
        assert_eq!(
            (o["retired"], o["mem_valid"], o["mem_addr"], o["wdata"]),
            (0, 1, 0x1000, 0x42)
        );
        assert_eq!(o["pc"], before["pc"]);
    }
    c.set("enable", 0);
    assert_eq!((c.peek()["mem_valid"], c.peek()["wstrb"]), (0, 0));
    c.clock();
    c.set("enable", 1);
    c.set("mem_ready", 1);
    assert_eq!(c.clock()["retired"], 1);
    c.set("instr_ready", 0);
    c.set("instr_fault", 1);
    let pc = c.peek()["pc"];
    for _ in 0..3 {
        assert_eq!(c.clock()["retired"], 0);
    }
    assert_eq!((c.peek()["pc"], c.peek()["trap"]), (pc, 0));
    c.set("instr_ready", 1);
    c.set("instr_fault", 0);
    let o = c.exec(0x0ff0000f); // FENCE iorw,iorw
    assert_eq!((o["retired"], o["wb_valid"], o["mem_valid"]), (1, 0, 0));
    // Load stall must not write rd until the acknowledged data is available.
    c.set("instr", i(3, 2, 3, 1, 0) as u64);
    c.set("mem_ready", 0);
    c.set("rdata", 123);
    assert_eq!(c.clock()["wb_valid"], 0);
    c.set("rdata", 456);
    c.set("mem_ready", 1);
    assert_eq!(c.clock()["wb_data"], 456);
    c.check(i(0x13, 0, 0, 3, 0), 456);
}

#[test]
fn precise_traps_reserved_encodings_and_fault_handshakes() {
    let mut c = Core::new();
    for ins in [
        0,
        0xffffffff,
        0x0000100f,
        0x30001073,
        r(0, 1, 3, 1, 2),
        r(1, 32, 3, 1, 2),
        i(0x13, 1, 3, 1, 0x400),
        i(0x13, 5, 3, 1, 0x200),
        i(3, 3, 3, 1, 0),
        s(3, 1, 2, 0),
        branch(2, 1, 2, 4),
        i(0x67, 1, 3, 1, 0),
    ] {
        c.fault(ins, 2, ins);
    }
    c.fault(jal(1, 2), 0, 2);
    c.init(1, 3);
    c.fault(i(0x67, 0, 1, 1, 0), 0, 2);
    c.init(1, 0x1001);
    c.fault(i(3, 1, 3, 1, 0), 4, 0x1001);
    c.fault(i(3, 2, 3, 1, 0), 4, 0x1001);
    c.fault(s(1, 1, 2, 0), 6, 0x1001);
    c.fault(s(2, 1, 2, 0), 6, 0x1001);
    c.fault(0x00000073, 11, 0);
    let pc = c.peek()["pc"] as u32;
    c.fault(0x00100073, 3, pc);
    c.set("instr_fault", 1);
    c.fault(i(0x13, 0, 1, 0, 1), 1, 0);
    c.set("instr_fault", 0);
    c.init(1, 0x1000);
    c.set("mem_fault", 1);
    c.set("mem_ready", 0);
    c.set("instr", i(3, 2, 3, 1, 0) as u64);
    assert_eq!(c.clock()["retired"], 0);
    assert_eq!(c.peek()["trap"], 0);
    c.set("mem_ready", 1);
    c.fault(i(3, 2, 3, 1, 0), 5, 0x1000);
    c.fault(s(2, 1, 2, 0), 7, 0x1000);
}

#[test]
fn bundled_demo_and_testbenches() {
    let results = goldl::testbench::run_tests(SRC);
    assert_eq!(results.len(), 2);
    for r in results {
        assert!(r.passed, "{}: {:?}", r.name, r.failure);
    }
    // Run the ROM through ECALL in all three logical representations, and check
    // every externally visible store rather than only the final Fibonacci value.
    let mut c = Core::with_top("RiscVDemo");
    let mut output = Vec::new();
    for _ in 0..80 {
        let o = c.clock();
        if o["valid"] != 0 {
            output.push(o["out"]);
        }
    }
    assert_eq!(output, [0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89]);
    let o = c.peek();
    assert_eq!((o["halted"], o["cause"], o["pc"]), (1, 11, 36));
}
