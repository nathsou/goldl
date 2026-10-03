//! Inline assemblers. ROM entries are instructions (8-bit Glider-8 or 32-bit RV32I),
//! not bytes. RISC-V labels are byte addresses; numeric branch operands are offsets.
use crate::syntax::ast::{AsmBlock, Ident};
use crate::syntax::lexer::{lex, parse_int, Tok, Token, P};
use crate::syntax::{Diag, Span};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isa {
    Glider8,
    RiscV,
}
impl Isa {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "Glider8" => Some(Self::Glider8),
            "RiscV" | "RV32I" => Some(Self::RiscV),
            _ => None,
        }
    }
    pub fn width(self) -> u32 {
        match self {
            Self::Glider8 => 8,
            Self::RiscV => 32,
        }
    }
    pub fn nop(self) -> u32 {
        match self {
            Self::Glider8 => 0,
            Self::RiscV => 0x13,
        }
    }
    fn stride(self) -> u32 {
        self.width() / 8
    }
    pub fn instructions(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Glider8 => GLIDER8,
            Self::RiscV => RISCV,
        }
    }
}

pub const GLIDER8: &[(&str, &str)] = &[
    ("NOP", "No operation"),
    ("LDI", "LDI imm: load a 4-bit immediate (0..15)"),
    ("ADD", "ADD r: add register to accumulator"),
    ("SUB", "SUB r: subtract register from accumulator"),
    ("AND", "AND r: bitwise AND"),
    ("OR", "OR r: bitwise OR"),
    ("XOR", "XOR r: bitwise XOR"),
    ("ST", "ST r: store accumulator in r0..r3"),
    ("LD", "LD r: load r0..r3"),
    ("JMP", "JMP label: unconditional jump"),
    ("JZ", "JZ label: jump if accumulator is zero"),
    ("JC", "JC label: jump if carry is set"),
    ("OUT", "Output the accumulator"),
    ("SHL", "Shift accumulator left"),
    ("SHR", "Shift accumulator right"),
    ("HLT", "Halt"),
];
pub const RISCV: &[(&str, &str)] = &[
    ("add", "add rd, rs1, rs2"),
    ("sub", "sub rd, rs1, rs2"),
    ("sll", "sll rd, rs1, rs2"),
    ("slt", "slt rd, rs1, rs2"),
    ("sltu", "sltu rd, rs1, rs2"),
    ("xor", "xor rd, rs1, rs2"),
    ("srl", "srl rd, rs1, rs2"),
    ("sra", "sra rd, rs1, rs2"),
    ("or", "or rd, rs1, rs2"),
    ("and", "and rd, rs1, rs2"),
    ("addi", "addi rd, rs1, signed_12_bit_imm"),
    ("slti", "slti rd, rs1, imm"),
    ("sltiu", "sltiu rd, rs1, imm"),
    ("xori", "xori rd, rs1, imm"),
    ("ori", "ori rd, rs1, imm"),
    ("andi", "andi rd, rs1, imm"),
    ("slli", "slli rd, rs1, shamt (0..31)"),
    ("srli", "srli rd, rs1, shamt (0..31)"),
    ("srai", "srai rd, rs1, shamt (0..31)"),
    ("lb", "lb rd, offset(rs1)"),
    ("lh", "lh rd, offset(rs1)"),
    ("lw", "lw rd, offset(rs1)"),
    ("lbu", "lbu rd, offset(rs1)"),
    ("lhu", "lhu rd, offset(rs1)"),
    ("sb", "sb rs2, offset(rs1)"),
    ("sh", "sh rs2, offset(rs1)"),
    ("sw", "sw rs2, offset(rs1)"),
    ("beq", "beq rs1, rs2, label_or_byte_offset"),
    ("bne", "bne rs1, rs2, label_or_byte_offset"),
    ("blt", "blt rs1, rs2, label_or_byte_offset"),
    ("bge", "bge rs1, rs2, label_or_byte_offset"),
    ("bltu", "bltu rs1, rs2, label_or_byte_offset"),
    ("bgeu", "bgeu rs1, rs2, label_or_byte_offset"),
    ("lui", "lui rd, unsigned_20_bit_imm"),
    ("auipc", "auipc rd, unsigned_20_bit_imm"),
    ("jal", "jal [rd,] label_or_byte_offset (default rd=ra)"),
    ("jalr", "jalr rd, offset(rs1); or jalr rd, rs1, imm"),
    ("fence", "fence [pred, succ]: sets of i, o, r, w"),
    ("ecall", "Environment call"),
    ("ebreak", "Breakpoint"),
    ("nop", "No operation (addi zero, zero, 0)"),
    (
        "li",
        "li rd, imm: load a 32-bit constant (one or two instructions)",
    ),
    ("mv", "mv rd, rs: copy a register"),
    ("j", "j label: jump without a link"),
    ("jr", "jr rs: jump to register"),
    ("ret", "Return to ra"),
    ("beqz", "beqz rs, label"),
    ("bnez", "bnez rs, label"),
    ("not", "not rd, rs"),
    ("neg", "neg rd, rs"),
];
pub const ABI_REGS: [&str; 32] = [
    "zero", "ra", "sp", "gp", "tp", "t0", "t1", "t2", "s0", "s1", "a0", "a1", "a2", "a3", "a4",
    "a5", "a6", "a7", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "t3", "t4",
    "t5", "t6",
];
pub fn register(isa: Isa, name: &str) -> Option<u32> {
    let name = name.to_ascii_lowercase();
    match isa {
        Isa::Glider8 => name
            .strip_prefix('r')?
            .parse::<u32>()
            .ok()
            .filter(|r| *r < 4),
        Isa::RiscV => {
            if name == "fp" {
                Some(8)
            } else {
                ABI_REGS
                    .iter()
                    .position(|r| *r == name)
                    .map(|r| r as u32)
                    .or_else(|| {
                        name.strip_prefix('x')?
                            .parse::<u32>()
                            .ok()
                            .filter(|r| *r < 32)
                    })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Word {
    pub value: u32,
    pub address: u32,
    pub span: Span,
}
pub struct Assembled {
    pub isa: Isa,
    pub words: Vec<Word>,
    pub labels: Vec<(Ident, u32)>,
    pub references: Vec<(Span, Span)>,
}
#[derive(Clone)]
struct Operand {
    tokens: Vec<Token>,
    span: Span,
}
struct Instruction {
    op: Ident,
    args: Vec<Operand>,
    span: Span,
    address: u32,
}
struct Assembly<'a> {
    block: &'a AsmBlock,
    isa: Isa,
    labels: HashMap<String, (Ident, u32)>,
    references: Vec<(Span, Span)>,
}
type R<T> = Result<T, Diag>;

fn text<'a>(block: &'a AsmBlock, t: Token) -> &'a str {
    &block.body[(t.span.start - block.body_span.start) as usize
        ..(t.span.end - block.body_span.start) as usize]
}
fn tokens(block: &AsmBlock) -> Vec<Token> {
    let mut result = Vec::new();
    let mut comment = false;
    for mut t in lex(&block.body) {
        t.span.start += block.body_span.start;
        t.span.end += block.body_span.start;
        if t.tok == Tok::Punct(P::Hash) {
            comment = true;
        }
        if t.tok == Tok::Newline {
            comment = false;
        }
        if !comment
            && !matches!(
                t.tok,
                Tok::LineComment | Tok::DocComment | Tok::BlockComment | Tok::Eof
            )
        {
            result.push(t);
        }
    }
    result
}
fn ident(block: &AsmBlock, t: Token) -> R<Ident> {
    if !matches!(t.tok, Tok::Ident | Tok::Kw(_)) {
        return Err(Diag::error(t.span, "expected an assembly identifier"));
    }
    Ok(Ident {
        name: text(block, t).into(),
        span: t.span,
    })
}
fn operands(ts: &[Token]) -> R<Vec<Operand>> {
    let mut args = Vec::new();
    let mut from = 0;
    let mut depth = 0;
    for (i, t) in ts.iter().enumerate() {
        match t.tok {
            Tok::Punct(P::LParen) => depth += 1,
            Tok::Punct(P::RParen) => depth -= 1,
            Tok::Punct(P::Comma) if depth == 0 => {
                if i == from {
                    return Err(Diag::error(t.span, "expected an operand"));
                }
                args.push(Operand {
                    tokens: ts[from..i].to_vec(),
                    span: ts[from].span.join(ts[i - 1].span),
                });
                from = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err(Diag::error(
            ts.last().unwrap().span,
            "unbalanced memory operand parentheses",
        ));
    }
    if from < ts.len() {
        args.push(Operand {
            tokens: ts[from..].to_vec(),
            span: ts[from].span.join(ts.last().unwrap().span),
        });
    } else if from > 0 {
        return Err(Diag::error(
            ts.last().unwrap().span,
            "expected an operand after comma",
        ));
    }
    Ok(args)
}
fn number(block: &AsmBlock, arg: &Operand) -> R<i64> {
    let ts = &arg.tokens;
    let (sign, lit) = match ts.as_slice() {
        [t] => (1i128, *t),
        [s, t] if matches!(s.tok, Tok::Punct(P::Minus | P::Plus)) => {
            (if s.tok == Tok::Punct(P::Minus) { -1 } else { 1 }, *t)
        }
        _ => return Err(Diag::error(arg.span, "expected an integer literal")),
    };
    if lit.tok != Tok::Int {
        return Err(Diag::error(arg.span, "expected an integer literal"));
    }
    let n = parse_int(text(block, lit))
        .filter(|n| *n <= i64::MAX as u128)
        .ok_or_else(|| Diag::error(arg.span, "invalid or oversized integer literal"))?;
    Ok((n as i128 * sign) as i64)
}

pub fn assemble(block: &AsmBlock) -> Result<Assembled, Vec<Diag>> {
    let Some(isa) = Isa::named(&block.isa.name) else {
        return Err(vec![Diag::error(
            block.isa.span,
            "unknown instruction set; expected Glider8, RiscV or RV32I",
        )]);
    };
    let mut a = Assembly {
        block,
        isa,
        labels: HashMap::new(),
        references: Vec::new(),
    };
    let mut instructions = Vec::new();
    let mut diags = Vec::new();
    let mut address = 0;
    let ts = tokens(block);
    for line in ts.split(|t| matches!(t.tok, Tok::Newline | Tok::Punct(P::Semi))) {
        let mut line = line;
        while line.len() >= 2 && line[1].tok == Tok::Punct(P::Colon) {
            match ident(block, line[0]) {
                Ok(id) => {
                    if let Some((old, _)) = a.labels.get(&id.name) {
                        diags.push(
                            Diag::error(id.span, format!("duplicate label `{}`", id.name))
                                .with_note(old.span, "first defined here"),
                        );
                    } else {
                        a.labels.insert(id.name.clone(), (id, address));
                    }
                }
                Err(d) => diags.push(d),
            }
            line = &line[2..];
        }
        if line.is_empty() {
            continue;
        }
        let parsed = (|| -> R<Instruction> {
            let op = ident(block, line[0])?;
            let args = operands(&line[1..])?;
            Ok(Instruction {
                op,
                args,
                span: line[0].span.join(line.last().unwrap().span),
                address,
            })
        })();
        match parsed {
            Ok(ins) => {
                let count = if isa == Isa::RiscV
                    && ins.op.name.eq_ignore_ascii_case("li")
                    && ins.args.len() == 2
                {
                    number(block, &ins.args[1])
                        .map(|n| if (-2048..=2047).contains(&n) { 1 } else { 2 })
                        .unwrap_or(1)
                } else {
                    1
                };
                address += count * isa.stride();
                instructions.push(ins);
            }
            Err(d) => diags.push(d),
        }
    }
    let limit = if isa == Isa::Glider8 { 16 } else { 4096 };
    if address / isa.stride() > limit {
        diags.push(Diag::error(
            block.body_span,
            format!("program exceeds {limit} instruction words"),
        ));
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let mut words = Vec::new();
    for ins in &instructions {
        let encoded = match isa {
            Isa::Glider8 => a.glider(ins),
            Isa::RiscV => a.riscv(ins),
        };
        match encoded {
            Ok(values) => {
                for (k, value) in values.into_iter().enumerate() {
                    words.push(Word {
                        value,
                        address: ins.address + k as u32 * isa.stride(),
                        span: ins.span,
                    });
                }
            }
            Err(d) => diags.push(d),
        }
    }
    if diags.is_empty() {
        let mut labels: Vec<_> = a.labels.into_values().collect();
        labels.sort_by_key(|(id, _)| id.span.start);
        Ok(Assembled {
            isa,
            words,
            labels,
            references: a.references,
        })
    } else {
        Err(diags)
    }
}

impl Assembly<'_> {
    fn arity(&self, ins: &Instruction, n: usize) -> R<()> {
        if ins.args.len() == n {
            Ok(())
        } else {
            Err(Diag::error(
                ins.span,
                format!(
                    "{} expects {n} operand(s), found {}",
                    ins.op.name,
                    ins.args.len()
                ),
            ))
        }
    }
    fn reg(&self, arg: &Operand) -> R<u32> {
        let r = if arg.tokens.len() == 1 {
            register(self.isa, text(self.block, arg.tokens[0]))
        } else {
            None
        };
        r.ok_or_else(|| {
            Diag::error(
                arg.span,
                match self.isa {
                    Isa::Glider8 => "expected register r0..r3",
                    Isa::RiscV => "expected register x0..x31 or an ABI register name",
                },
            )
        })
    }
    fn imm(&self, arg: &Operand, lo: i64, hi: i64) -> R<i64> {
        let n = number(self.block, arg)?;
        if (lo..=hi).contains(&n) {
            Ok(n)
        } else {
            Err(Diag::error(
                arg.span,
                format!("immediate must be between {lo} and {hi} (got {n})"),
            ))
        }
    }
    fn target(&mut self, arg: &Operand, pc: u32, lo: i64, hi: i64) -> R<i64> {
        let n = if arg.tokens.len() == 1 && matches!(arg.tokens[0].tok, Tok::Ident | Tok::Kw(_)) {
            let name = text(self.block, arg.tokens[0]);
            let (id, address) = self
                .labels
                .get(name)
                .ok_or_else(|| Diag::error(arg.span, format!("undefined label `{name}`")))?;
            self.references.push((arg.span, id.span));
            *address as i64 - if self.isa == Isa::RiscV { pc as i64 } else { 0 }
        } else {
            number(self.block, arg)?
        };
        if !(lo..=hi).contains(&n) {
            return Err(Diag::error(
                arg.span,
                format!("jump target/offset must be between {lo} and {hi} (got {n})"),
            ));
        }
        if self.isa == Isa::RiscV && n % 4 != 0 {
            return Err(Diag::error(
                arg.span,
                "RV32I branch/jump offsets must be aligned to 4 bytes",
            ));
        }
        Ok(n)
    }
    fn memory(&self, arg: &Operand) -> R<(u32, u32)> {
        let ts = &arg.tokens;
        let Some(k) = ts.iter().position(|t| t.tok == Tok::Punct(P::LParen)) else {
            return Err(Diag::error(arg.span, "expected offset(register)"));
        };
        if ts.len() != k + 3 || ts[k + 2].tok != Tok::Punct(P::RParen) {
            return Err(Diag::error(arg.span, "expected offset(register)"));
        }
        let base = Operand {
            tokens: vec![ts[k + 1]],
            span: ts[k + 1].span,
        };
        let offset = if k == 0 {
            0
        } else {
            self.imm(
                &Operand {
                    tokens: ts[..k].to_vec(),
                    span: ts[0].span.join(ts[k - 1].span),
                },
                -2048,
                2047,
            )? as u32
        };
        Ok((self.reg(&base)?, offset & 0xfff))
    }
    fn glider(&mut self, ins: &Instruction) -> R<Vec<u32>> {
        let op = GLIDER8
            .iter()
            .position(|(name, _)| name.eq_ignore_ascii_case(&ins.op.name))
            .ok_or_else(|| {
                Diag::error(
                    ins.op.span,
                    format!("unknown Glider8 instruction `{}`", ins.op.name),
                )
            })? as u32;
        let arg = match op {
            1 => {
                self.arity(ins, 1)?;
                self.imm(&ins.args[0], 0, 15)? as u32
            }
            2..=8 => {
                self.arity(ins, 1)?;
                self.reg(&ins.args[0])?
            }
            9..=11 => {
                self.arity(ins, 1)?;
                self.target(&ins.args[0], ins.address, 0, 15)? as u32
            }
            _ => {
                self.arity(ins, 0)?;
                0
            }
        };
        Ok(vec![op << 4 | arg])
    }
    fn riscv(&mut self, ins: &Instruction) -> R<Vec<u32>> {
        let op = ins.op.name.to_ascii_lowercase();
        let args = &ins.args;
        let r = |k: usize| self.reg(&args[k]);
        let i = |k: usize| self.imm(&args[k], -2048, 2047).map(|n| n as u32 & 0xfff);
        let word = match op.as_str() {
            "add" | "sub" | "sll" | "slt" | "sltu" | "xor" | "srl" | "sra" | "or" | "and" => {
                self.arity(ins, 3)?;
                let (f3, f7) = match op.as_str() {
                    "add" => (0, 0),
                    "sub" => (0, 32),
                    "sll" => (1, 0),
                    "slt" => (2, 0),
                    "sltu" => (3, 0),
                    "xor" => (4, 0),
                    "srl" => (5, 0),
                    "sra" => (5, 32),
                    "or" => (6, 0),
                    _ => (7, 0),
                };
                f7 << 25 | r(2)? << 20 | r(1)? << 15 | f3 << 12 | r(0)? << 7 | 0x33
            }
            "addi" | "slti" | "sltiu" | "xori" | "ori" | "andi" => {
                self.arity(ins, 3)?;
                let f3 = match op.as_str() {
                    "addi" => 0,
                    "slti" => 2,
                    "sltiu" => 3,
                    "xori" => 4,
                    "ori" => 6,
                    _ => 7,
                };
                itype(0x13, f3, r(0)?, r(1)?, i(2)?)
            }
            "slli" | "srli" | "srai" => {
                self.arity(ins, 3)?;
                let imm = self.imm(&args[2], 0, 31)? as u32 | if op == "srai" { 0x400 } else { 0 };
                itype(0x13, if op == "slli" { 1 } else { 5 }, r(0)?, r(1)?, imm)
            }
            "lb" | "lh" | "lw" | "lbu" | "lhu" => {
                self.arity(ins, 2)?;
                let (base, imm) = self.memory(&args[1])?;
                let f3 = match op.as_str() {
                    "lb" => 0,
                    "lh" => 1,
                    "lw" => 2,
                    "lbu" => 4,
                    _ => 5,
                };
                itype(3, f3, r(0)?, base, imm)
            }
            "sb" | "sh" | "sw" => {
                self.arity(ins, 2)?;
                let (base, imm) = self.memory(&args[1])?;
                let f3 = match op.as_str() {
                    "sb" => 0,
                    "sh" => 1,
                    _ => 2,
                };
                (imm >> 5) << 25 | r(0)? << 20 | base << 15 | f3 << 12 | (imm & 31) << 7 | 0x23
            }
            "beq" | "bne" | "blt" | "bge" | "bltu" | "bgeu" | "beqz" | "bnez" => {
                let pseudo = op.ends_with('z');
                self.arity(ins, if pseudo { 2 } else { 3 })?;
                let rs1 = r(0)?;
                let rs2 = if pseudo { 0 } else { r(1)? };
                let imm =
                    self.target(&args[if pseudo { 1 } else { 2 }], ins.address, -4096, 4094)?
                        as u32
                        & 0x1fff;
                let f3 = match op.as_str() {
                    "beq" | "beqz" => 0,
                    "bne" | "bnez" => 1,
                    "blt" => 4,
                    "bge" => 5,
                    "bltu" => 6,
                    _ => 7,
                };
                (imm >> 12) << 31
                    | ((imm >> 5) & 63) << 25
                    | rs2 << 20
                    | rs1 << 15
                    | f3 << 12
                    | ((imm >> 1) & 15) << 8
                    | ((imm >> 11) & 1) << 7
                    | 0x63
            }
            "lui" | "auipc" => {
                self.arity(ins, 2)?;
                (self.imm(&args[1], 0, 0xfffff)? as u32) << 12
                    | r(0)? << 7
                    | if op == "lui" { 0x37 } else { 0x17 }
            }
            "jal" | "j" => {
                let rd = if op == "j" {
                    self.arity(ins, 1)?;
                    0
                } else if args.len() == 1 {
                    1
                } else {
                    self.arity(ins, 2)?;
                    r(0)?
                };
                let imm = self.target(args.last().unwrap(), ins.address, -1048576, 1048574)? as u32
                    & 0x1fffff;
                (imm >> 20) << 31
                    | ((imm >> 1) & 0x3ff) << 21
                    | ((imm >> 11) & 1) << 20
                    | ((imm >> 12) & 255) << 12
                    | rd << 7
                    | 0x6f
            }
            "jalr" => {
                if args.len() == 2 {
                    let (base, imm) = self.memory(&args[1])?;
                    itype(0x67, 0, r(0)?, base, imm)
                } else {
                    self.arity(ins, 3)?;
                    itype(0x67, 0, r(0)?, r(1)?, i(2)?)
                }
            }
            "fence" => {
                let mask = |arg: &Operand| -> R<u32> {
                    if arg.tokens.len() != 1 {
                        return Err(Diag::error(
                            arg.span,
                            "expected a fence set containing i, o, r, w",
                        ));
                    }
                    let mut mask = 0;
                    for ch in text(self.block, arg.tokens[0]).chars() {
                        mask |= match ch {
                            'i' => 8,
                            'o' => 4,
                            'r' => 2,
                            'w' => 1,
                            _ => {
                                return Err(Diag::error(
                                    arg.span,
                                    "fence sets may only contain i, o, r, w",
                                ))
                            }
                        };
                    }
                    Ok(mask)
                };
                let (pred, succ) = if args.is_empty() {
                    (15, 15)
                } else {
                    self.arity(ins, 2)?;
                    (mask(&args[0])?, mask(&args[1])?)
                };
                pred << 24 | succ << 20 | 0x0f
            }
            "ecall" | "ebreak" | "nop" | "ret" => {
                self.arity(ins, 0)?;
                match op.as_str() {
                    "ecall" => 0x73,
                    "ebreak" => 0x100073,
                    "nop" => 0x13,
                    _ => 0x8067,
                }
            }
            "mv" | "not" | "neg" => {
                self.arity(ins, 2)?;
                match op.as_str() {
                    "mv" => itype(0x13, 0, r(0)?, r(1)?, 0),
                    "not" => itype(0x13, 4, r(0)?, r(1)?, 0xfff),
                    _ => 0x40000033 | r(1)? << 20 | r(0)? << 7,
                }
            }
            "jr" => {
                self.arity(ins, 1)?;
                itype(0x67, 0, 0, r(0)?, 0)
            }
            "li" => {
                self.arity(ins, 2)?;
                let rd = r(0)?;
                let value = self.imm(&args[1], i32::MIN as i64, u32::MAX as i64)? as u32;
                let signed = value as i32 as i64;
                // Use the same one/two-word decision as the address pass, including
                // unsigned spelling of negative 32-bit values.
                let spelling = number(self.block, &args[1])?;
                if (-2048..=2047).contains(&spelling) {
                    itype(0x13, 0, rd, 0, value & 0xfff)
                } else {
                    let upper = ((signed + 0x800) >> 12) as u32 & 0xfffff;
                    return Ok(vec![
                        upper << 12 | rd << 7 | 0x37,
                        itype(0x13, 0, rd, rd, value & 0xfff),
                    ]);
                }
            }
            _ => {
                return Err(Diag::error(
                    ins.op.span,
                    format!(
                        "unknown RV32I instruction `{}` (extensions are not supported)",
                        ins.op.name
                    ),
                ))
            }
        };
        Ok(vec![word])
    }
}
fn itype(op: u32, f3: u32, rd: u32, rs1: u32, imm: u32) -> u32 {
    imm << 20 | rs1 << 15 | f3 << 12 | rd << 7 | op
}

/// Find assembly interiors even while a block is incomplete or has invalid operands.
/// Used for context-aware editor features, independently of successful elaboration.
pub fn blocks(src: &str) -> Vec<AsmBlock> {
    use crate::syntax::lexer::Kw;
    let ts: Vec<_> = lex(src)
        .into_iter()
        .filter(|t| {
            !matches!(
                t.tok,
                Tok::Newline | Tok::LineComment | Tok::DocComment | Tok::BlockComment
            )
        })
        .collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k + 3 < ts.len() {
        if ts[k].tok == Tok::Kw(Kw::Asm)
            && ts[k + 1].tok == Tok::Kw(Kw::For)
            && ts[k + 2].tok == Tok::Ident
            && ts[k + 3].tok == Tok::Punct(P::LBrace)
        {
            let id = ts[k + 2];
            let start = ts[k + 3].span.end;
            k += 4;
            while k < ts.len() && !matches!(ts[k].tok, Tok::Punct(P::RBrace) | Tok::Eof) {
                if ts[k].tok == Tok::Punct(P::Hash) {
                    let tail = &src[ts[k].span.end as usize..];
                    let end = ts[k].span.end + tail.find('\n').unwrap_or(tail.len()) as u32;
                    while k < ts.len() && ts[k].span.start < end {
                        k += 1;
                    }
                } else {
                    k += 1;
                }
            }
            let end = ts.get(k).map(|t| t.span.start).unwrap_or(src.len() as u32);
            out.push(AsmBlock {
                isa: Ident {
                    name: src[id.span.start as usize..id.span.end as usize].into(),
                    span: id.span,
                },
                body: src[start as usize..end as usize].into(),
                body_span: Span::new(start, end),
            });
        } else {
            k += 1;
        }
    }
    out
}

/// Lexical semantic classifications for assembly identifiers and hash comments.
pub fn classifications(block: &AsmBlock) -> Vec<(Span, &'static str)> {
    let mut out = vec![(block.isa.span, "class")];
    let isa = Isa::named(&block.isa.name);
    let ts = tokens(block);
    for line in ts.split(|t| matches!(t.tok, Tok::Newline | Tok::Punct(P::Semi))) {
        let mut k = 0;
        while k + 1 < line.len() && line[k + 1].tok == Tok::Punct(P::Colon) {
            out.push((line[k].span, "variable"));
            k += 2;
        }
        if let Some(t) = line.get(k) {
            out.push((t.span, "function"));
        }
        if let Some(isa) = isa {
            for t in line.iter().skip(k + 1) {
                if register(isa, text(block, *t)).is_some() {
                    out.push((t.span, "property"));
                }
            }
        }
    }
    let mut hash = None;
    for mut t in lex(&block.body) {
        t.span.start += block.body_span.start;
        t.span.end += block.body_span.start;
        if t.tok == Tok::Punct(P::Hash) && hash.is_none() {
            hash = Some(t.span.start);
        }
        if matches!(t.tok, Tok::Newline | Tok::Eof) {
            if let Some(start) = hash.take() {
                out.push((Span::new(start, t.span.start), "comment"));
            }
        }
    }
    out
}
