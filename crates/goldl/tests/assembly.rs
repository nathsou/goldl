use goldl::{asm, elab, rtl, syntax::{ast::{ExprKind, Item}, parser, Severity}};

fn assembled(isa: &str, body: &str) -> asm::Assembled {
    let src = format!("const P = asm for {isa} {{\n{body}\n}}");
    let (file, ds) = parser::parse(&src);
    assert!(ds.is_empty(), "{ds:?}");
    let Item::Const(c) = &file.items[0] else { panic!() };
    let ExprKind::Asm(b) = &c.value.kind else { panic!() };
    asm::assemble(b).unwrap_or_else(|ds| panic!("{ds:?}"))
}
fn words(isa: &str, body: &str) -> Vec<u32> {
    assembled(isa, body).words.iter().map(|w| w.value).collect()
}
fn errors(src: &str) -> Vec<String> {
    elab::analyze(src).diags.iter().filter(|d| d.severity == Severity::Error).map(|d| d.message.clone()).collect()
}

#[test]
fn glider_fibonacci_encoding_is_unchanged() {
    assert_eq!(words("Glider8", "LDI 1; ST r1; LDI 0; ST r0\nloop: LD r0\nADD r1\nJC done\nOUT\nST r2\nLD r1\nST r0\nLD r2\nST r1\nJMP loop\ndone: HLT"),
        [0x11,0x71,0x10,0x70,0x80,0x21,0xbe,0xc0,0x72,0x81,0x70,0x82,0x71,0x94,0xf0]);
    assert_eq!(words("Glider8", "NOP; LDI 15; ADD R3; SUB r2; AND r1; OR r0; XOR r1; ST r2; LD r3; JMP 0; JZ 1; JC 2; OUT; SHL; SHR; HLT"),
        [0,0x1f,0x23,0x32,0x41,0x50,0x61,0x72,0x83,0x90,0xa1,0xb2,0xc0,0xd0,0xe0,0xf0]);
}

#[test]
fn riscv_fibonacci_encoding_is_unchanged() {
    assert_eq!(words("RiscV", "addi x1, zero, 0\naddi x2, zero, 1\naddi x3, zero, 12\nloop: sw ra, 0(zero)\nadd x4, x1, x2\nmv x1, x2\nmv x2, x4\naddi x3, x3, -1\nbne x3, zero, loop\necall"),
        [0x00000093,0x00100113,0x00c00193,0x00102023,0x00208233,0x00010093,0x00020113,0xfff18193,0xfe0196e3,0x73]);
}

#[test]
fn rv32i_base_encodings() {
    // Fixed architectural encodings, independent of the assembler's encoding helpers.
    let cases = [
        ("add x1,x2,x3",0x003100b3), ("sub x1,x2,x3",0x403100b3),
        ("sll x1,x2,x3",0x003110b3), ("slt x1,x2,x3",0x003120b3), ("sltu x1,x2,x3",0x003130b3),
        ("xor x1,x2,x3",0x003140b3), ("srl x1,x2,x3",0x003150b3), ("sra x1,x2,x3",0x403150b3),
        ("or x1,x2,x3",0x003160b3), ("and x1,x2,x3",0x003170b3),
        ("addi x1,x2,-1",0xfff10093), ("slti x1,x2,-1",0xfff12093), ("sltiu x1,x2,-1",0xfff13093),
        ("xori x1,x2,-1",0xfff14093), ("ori x1,x2,-1",0xfff16093), ("andi x1,x2,-1",0xfff17093),
        ("slli x1,x2,31",0x01f11093), ("srli x1,x2,31",0x01f15093), ("srai x1,x2,31",0x41f15093),
        ("lb x1,-1(x2)",0xfff10083), ("lh x1,-1(x2)",0xfff11083), ("lw x1,-1(x2)",0xfff12083),
        ("lbu x1,-1(x2)",0xfff14083), ("lhu x1,-1(x2)",0xfff15083),
        ("sb x3,-1(x2)",0xfe310fa3), ("sh x3,-1(x2)",0xfe311fa3), ("sw x3,-1(x2)",0xfe312fa3),
        ("beq x1,x2,8",0x00208463), ("bne x1,x2,8",0x00209463), ("blt x1,x2,8",0x0020c463),
        ("bge x1,x2,8",0x0020d463), ("bltu x1,x2,8",0x0020e463), ("bgeu x1,x2,8",0x0020f463),
        ("lui x1,0xabcde",0xabcde0b7), ("auipc x1,0xabcde",0xabcde097),
        ("jal x1,8",0x008000ef), ("jalr x1,-1(x2)",0xfff100e7), ("jalr x1,x2,-1",0xfff100e7),
        ("fence",0x0ff0000f), ("fence rw,rw",0x0330000f), ("ecall",0x73), ("ebreak",0x100073),
    ];
    for (src, want) in cases { assert_eq!(words("RV32I", src), [want], "{src}"); }
}

#[test]
fn pseudoinstructions_and_relocation_after_expansion() {
    assert_eq!(words("RiscV", "li a0,0x12345800\nj done\nli a1,-2048\ndone: ret"),
        [0x12346537,0x80050513,0x0080006f,0x80000593,0x8067]);
    let a = assembled("RiscV", "start: nop\nli t0,0xffffffff\nbnez t0,start\nbeqz t0,end\nj start\nend: jr ra");
    assert_eq!(a.words.iter().map(|w| w.address).collect::<Vec<_>>(), [0,4,8,12,16,20,24]);
    assert_eq!(a.labels.iter().map(|(_, a)| *a).collect::<Vec<_>>(), [0,24]);
    assert_eq!(words("RiscV", "mv a0,a1; not a0,a1; neg a0,a1; jal 4"), [0x58513,0xfff5c513,0x40b00533,0x4000ef]);
}

#[test]
fn inferred_arrays_and_nop_padding_compile_to_rom_values() {
    for (isa, width, nop) in [("Glider8",8,0), ("RiscV",32,0x13)] {
        let instruction = if width == 8 { "LDI 3" } else { "addi x1,zero,3" };
        let src = format!("const P: bits<{width}>[4] = asm for {isa} {{ {instruction} }}\nmodule M(a: bits<2>) -> (y: bits<{width}>) {{ y = P[a] }}");
        let (r, _) = elab::elaborate(&src, None).unwrap_or_else(|e| panic!("{:?}", e.diags));
        let mut sim = rtl::RtlSim::new(&r);
        for addr in 1..4 { sim.eval(&r, &[addr]); assert_eq!(sim.vals[r.outputs[0].1 as usize], nop); }
        assert!(errors(&src.replace("[4]", "[_]")).is_empty());
    }
    for src in [
        "const P: bits<8>[_] = [1,2,3]\nmodule M() -> (y: bits<8>) { y = P[2] }",
        "module M() -> (y: bits<8>) { let p: bits<8>[_] = [1,2,3]; y = p[2] }",
        "const P: bits<8>[_] = [7;3]\nmodule M() -> (y: bits<8>) { y = P[2] }",
    ] {
        let (r, _) = elab::elaborate(src, None).unwrap_or_else(|e| panic!("{:?}", e.diags));
        let mut sim = rtl::RtlSim::new(&r); sim.eval(&r, &[]);
        assert!(matches!(sim.vals[r.outputs[0].1 as usize], 3 | 7));
    }
}

#[test]
fn diagnostics_are_precise_and_invalid_programs_do_not_panic() {
    for (isa, body, expected) in [
        ("Glider8","LDI 16","immediate"), ("Glider8","LD r4","register"),
        ("Glider8","JMP missing","undefined label"), ("Glider8","x: NOP\nx: HLT","duplicate label"),
        ("RiscV","add x32,x0,x0","register"), ("RiscV","addi x1,x0,2048","immediate"),
        ("RiscV","slli x1,x0,32","immediate"), ("RiscV","beq x0,x0,2","aligned"),
        ("RiscV","j 1048576","offset"), ("RiscV","mul x1,x2,x3","unknown RV32I"),
        ("RiscV","sw x1,0(x2","unbalanced"), ("RiscV","sw x1,0(x2))(","offset(register)"),
        ("RiscV","jal","expects"), ("RiscV","add x1,,x2","operand"),
        ("RiscV","add x1,x2,x3,","operand"), ("Wrong","NOP","instruction set"),
    ] {
        let src = format!("const P: bits<32>[_] = asm for {isa} {{\n{body}\n}}");
        let ds = elab::analyze(&src).diags;
        assert!(ds.iter().any(|d| d.message.contains(expected)), "{body}: {ds:?}");
        assert!(ds.iter().all(|d| d.span.start <= d.span.end && d.span.end as usize <= src.len()));
    }
    for (src, expected) in [
        ("const P: bits<8>[_] = []", "inferred array size"),
        ("const P: bits<8>[1] = asm for Glider8 { NOP; HLT }", "expected 1 elements"),
        ("const P: bits<8>[_] = asm for RiscV { nop }", "width mismatch"),
        ("module M(a: bits<8>[_]) -> (y: bit) { y = 0 }", "requires an initializer"),
    ] { assert!(errors(src).iter().any(|d| d.contains(expected)), "{src}"); }
    let body = "NOP;".repeat(17);
    assert!(errors(&format!("const P = asm for Glider8 {{ {body} }}")).iter().any(|d| d.contains("exceeds 16")));
}

#[test]
fn comments_and_formatting_preserve_assembly() {
    let src = "const P: bits<32>[_] = asm for RiscV {\n# a comment with } and café\nli a0, 3 // comment\n/* another comment */\nloop: addi a0,a0,-1\nbnez a0,loop\n}\nmodule M() -> (y: bits<32>) { y=P[0] }\n";
    assert!(errors(src).is_empty());
    let formatted = goldl::syntax::format::format(src, 2);
    assert!(errors(&formatted).is_empty(), "{formatted}");
    assert_eq!(formatted, goldl::syntax::format::format(&formatted, 2));
}
