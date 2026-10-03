# GoLDL

**A hardware description language that compiles to Conway's Game of Life.**

Write a circuit in a small Rust-like HDL. GoLDL compiles it to a Game of Life pattern:
- every wire carries one glider (1) or none (0) per clock cycle;
- logic happens where gliders collide;
- wires are glider lanes bent by reflectors;
- registers are glider loops that come back exactly one clock period later.

A hierarchical simulator reconstructs the **exact** state of every cell at **any**
generation without stepping through the generations in between. HashLife confirms that the
real Life rules agree.

The playground runs the whole Rust toolchain in the browser as WebAssembly:
- a code editor with a language server;
- a WebGL view of the running universe. An abstraction overlay names every region after the
  source that produced it (`Cpu › acc.next › if writes_acc`), down to single gates
  (`AND (a ∧ ¬b) · 4-bit adder`) when you zoom in. The input, output and register pins are
  drawn where their glider streams enter and leave the pattern, with their current values;
- a hierarchical logic schematic with ANSI symbols: each scope shows its sub-groups as boxes.
  Double-click a box to enter it, or an operator (adder, multiplexer, comparator…) to see its
  gates with live values;
- waveforms;
- dockable panes (code, problems and tests, inspector) with layout presets, and light, dark
  and system themes.

```goldl
/// 8-bit ALU: add, subtract, and, or, xor, shift right, not.
module Alu(a: bits<8>, b: bits<8>, op: bits<3>) -> (y: bits<8>, c: bit, v: bit) {
  let subtract: bit = op == 1
  let addend: bits<8> = if subtract { ~b } else { b }
  let carry_in: bits<9> = if subtract { 1 } else { 0 }
  let wide: bits<9> = zext(a, 9) + zext(addend, 9) + carry_in
  let arithmetic: bit = op == 0 || op == 1
  let result: bits<8> = match op {
    _ => wide[7:0],
    2 => a & b,
    3 => a | b,
    4 => a ^ b,
    6 => a >> 1,
    7 => ~a,
  }
  y = result
  c = if arithmetic { wide[8] != subtract } else if op == 6 { a[0] } else { 0 }
  v = arithmetic && a[7] == addend[7] && result[7] != a[7]
}
```

This ALU becomes 14,284 Life components spread over 694,196 × 669,556 cells, with a
clock period of 5.56 million generations. The included Glider-8 CPU uses about
3 million live cells. A complete [RV32I example](docs/RISCV.md) adds a 32-bit RISC-V
core with a self-running Fibonacci program. [Measured layout experiments](docs/experiments/README.md)
reduce the ALU's area by 35% and clock period by 27%, with 6% fewer static live cells.

Exported patterns are self-contained: they run in any Life program (Golly, for instance)
with no harness. Nothing is generated from outside the pattern; the only external data is
the input tape described below.

## Quick start

```sh
cargo build --release
./target/release/goldl test examples/cpu.goldl        # run the test benches
./target/release/goldl run examples/cpu.goldl 120     # RTL simulation
./target/release/goldl build examples/alu.goldl -o alu.rle   # pattern for Golly
./target/release/goldl build examples/counter.goldl --set en=1 --cycles 100 -o counter.rle
./target/release/goldl verify examples/counter.goldl 4      # Life vs. logic, with HashLife
./target/release/goldl lsp                            # language server on stdio
```

Playground (needs the `wasm32-unknown-unknown` target):

```sh
cd web
npm install
npm run wasm     # builds the compiler to web/public/goldl.wasm
npm run dev
```

The playground deploys to GitHub Pages from `main` (`.github/workflows/pages.yml`).

## The language

| Construct | Example |
|---|---|
| module with generics | `module Ripple<N: uint = 4>(a: bits<N>, b: bits<N>) -> (s: bits<N>, cout: bit) { … }` |
| wires | `let x: bits<8> = a + b` (or declared, then assigned bit by bit: `c[i+1] = …`) |
| registers | `reg r: bits<4> = 0` and `r.next = if en { r + 1 } else { r }` |
| memories | `mem regs: bits<8>[4]`, `regs.write(addr, data, enable)`, `regs[addr]` |
| ROMs / constants | `const PROGRAM: bits<8>[16] = [0x11, 0x71, …]`, `PROGRAM[pc]` |
| inferred array lengths | `const VALUES: bits<8>[_] = [1, 2, 3]` (also works in initialized `let` bindings) |
| inline assembly | `const PROGRAM: bits<8>[_] = asm for Glider8 { LDI 1; OUT; HLT }` |
| match | `match op { _ => default, 2 => a & b, 3 | 4 => … }`: disjoint arms, `_` anywhere |
| enums | `enum Light: bits<2> { Red, Green }`, `Light::Red` |
| functions | `fn maj(a: bit, b: bit, c: bit) -> bit { (a & b) | (a & c) | (b & c) }` (inlined) |
| loops | `for i in 0..N { … }` (unrolled) |
| instances | `let c0 = Counter<4>(en: en)`, then `c0.count` |
| builtins | `zext sext trunc cat rep clog2 width any all parity mux slt sle sgt sge` |
| tests | `test "name" for Cpu { step(8) assert valid == 1 && out == 1 }` |

Widths are checked: there are no implicit truncations. Combinational loops are reported
with their path. The examples in [`examples/`](examples) range from a flip-flop to the
Glider-8 CPU and a complete RV32I core.

### Inline assembly

Programs live in the same source file as the hardware. `asm for Glider8 { … }`
produces 8-bit instruction words; `asm for RiscV { … }` (also spelled `RV32I`)
produces 32-bit RV32I instruction words:

```goldl
const PROGRAM: bits<8>[_] = asm for Glider8 {
  LDI 3
  OUT
  HLT
}

const RV_PROGRAM: bits<32>[_] = asm for RiscV {
  li a0, 7
  li a1, 5
  add a0, a0, a1
  sw a0, 0(zero)
  ecall
}
```

`[_]` infers the emitted word count (3 and 5 above), including pseudoinstruction
expansion. It works for array literals and repeats too. Inference requires an
initializer, and lengths must be 1–4096. Labels and comments do not occupy ROM
slots. An explicit assembly length, such as `[16]`, sets capacity: shorter programs
are padded with the ISA's NOP (`0x00` for Glider-8, `0x00000013` for RV32I), while
longer programs are errors. Ordinary array literals still require an exact length.
Dynamic array reads outside the declared/inferred length return zero; constant
out-of-bounds indices are errors. For a CPU with a 16-slot ROM, use `[16]` when you
want NOPs in every unused slot. The declared element width must match the ISA.

Instructions are case-insensitive, separated by newlines or semicolons. Operands
use decimal, hexadecimal or binary integers, with optional signs. Labels are
case-sensitive identifiers ending in `:`, local to their assembly block. GoLDL
`//` and `/* … */` comments and assembly `#` comments are supported.

- **Glider-8:** all 16 instructions (`NOP LDI ADD SUB AND OR XOR ST LD JMP JZ JC
  OUT SHL SHR HLT`), registers `r0`–`r3`, and at most 16 instructions. `LDI`
  immediates and jump addresses are 0–15. Jump labels resolve to ROM slot numbers.
- **RV32I:** all base integer instructions, `x0`–`x31`, standard ABI aliases
  (`zero`, `ra`, `sp`, `a0`, `t0`, etc.) and `fp`. Loads/stores use `offset(rs1)`;
  `jalr` accepts that form or `rd, rs1, imm`. `jal label` defaults to `ra`.
  `fence` defaults to `iorw, iorw`, or accepts explicit sets such as `fence rw, rw`.
  Labels are byte addresses from the start of the ROM. Branch/jump labels are
  PC-relative; numeric operands are byte offsets, aligned to 4 bytes. Signed
  immediates must fit their instruction's field; `lui`/`auipc` take an unsigned
  20-bit immediate.
- **RV32I pseudoinstructions:** `nop`, `li`, `mv`, `j`, `jr`, `ret`, `beqz`,
  `bnez`, `not`, `neg`. `li` supports signed or unsigned 32-bit literals and emits
  one or two instructions. Label offsets account for that expansion. Extensions
  such as M, C and Zicsr, directives, symbolic relocations and external symbols
  are not supported.

The playground provides inline errors, mnemonic/register/label completion, label
navigation and instruction hover showing ROM addresses and encodings. Its design
picker includes **Inline assembly**, plus Glider-8 and RV32I Fibonacci demos with
editable assembly. Use **New design** to start a minimal module; **Restore previous
design** recovers the draft it replaced. Changed source is marked until compiled,
and compiling a changed design resets simulation to cycle zero. Assembly also
works in the CLI: it is compiled into ordinary ROM logic, so program changes
rebuild the circuit and its Life pattern.

## How it compiles

```
source ─▶ elaboration ─▶ RTL ─▶ AIG ─▶ glider netlist ─▶ phase schedule ─▶ compact layout ─▶ Life pattern
           (lazy, typed)  word    AND/     crossings,       slack-minimising   blocks + exact      + exact
                          level   inverter splitters,       LP, timing-driven  glider routes       reconstruction
                                  graph    constant streams fan-out trees
```

- **Logic: inhibition.** Two gliders on perpendicular lanes annihilate. A crossing therefore
  computes `a ∧ ¬b` on one output and `b ∧ ¬a` on the other. With a constant stream of
  gliders (a "One"), this gives NOT and AND, so every circuit can be built. All constant
  streams come from banks of one-bit registers that hold 1. Each glider circles the circuit
  once per clock period and feeds up to 32 consumers through a duplicator tree. Fan-out uses
  Syringe duplicators. Turns use Snarks and Bandersnatch-based colour-changing reflectors.
  All components are characterised by simulation in all eight orientations, with flipbooks
  of their reactions.
- **Phase classes.** Gliders on rows travel at a phase ≡ 0 (mod 86 generations) and gliders
  on columns at ≡ 43. Turns add one class step (+43) or three (+129), with the cheaper three-step
  turns paired to restore lane parity. Any row glider can then
  cross any column glider safely, so wiring never needs to check for collisions.
- **Timing.** A crossing needs both inputs at exactly the same phase. A schedule assigns a
  phase to every node: input streams, constants and register outputs have free phase, and
  the schedule minimises the total slack. Fan-out trees are rebuilt so that late consumers
  sit deep in the tree. Remaining slack is absorbed by the shortest mixture of
  ordinary/slow turn pairs or compact delay loops.
- **Layout with row reuse.** Each node occupies fresh columns in topological order.
  Its block sits above its drivers and clear of all live output lanes. Rows become
  available after their last consumer; register input rows stay reserved to the
  perimeter. Inputs rise on dedicated columns and absorb exactly the scheduled
  number of turns. Complete fragment footprints keep delay loops clear too.
- **Registers** are return loops around the whole circuit. The clock period is chosen so
  that every loop closes exactly. There is no clock signal: the timing is entirely in the
  geometry.
- **Inputs** are glider tapes: gliders placed outside the circuit, one slot per clock cycle,
  flying in on each input lane. `goldl build --cycles N --set PORT=VALUE[@CYCLE]` (or
  *Share → Download RLE* in the playground) bakes N cycles of input values into the pattern.
  When the tape runs out, the inputs read 0 and the circuit keeps running.

## How it simulates

The logic is simulated cycle by cycle, either word-level (RTL) or on the glider netlist.
The physical database records:
- every glider leg: an exact trajectory over a time window, gated by a signal;
- every component: its still-life cells plus a flipbook of its reaction;
- every crossing site.

The cells in any rectangle at any generation `g` follow directly from which signals are 1
in the relevant cycles. The cost depends on the number of objects in view, not on `g`. That
is how the playground can jump to generation 700,000 or 7,000,000,000 instantly. `Verify`
(playground) and `goldl verify` (CLI) run the actual Life rules with HashLife and compare
every cell.

## Repository

| Path | |
|---|---|
| `crates/goldl-life` | Life engines: sparse bit-parallel tiles with change tracking, and HashLife |
| `crates/goldl` | compiler, simulator, language server, test benches, schematic layout, JSON |
| `crates/goldl/tech` | component RLEs (Snark, Bandersnatch, Syringe, …) with attribution |
| `crates/goldl-wasm` | raw C-ABI WebAssembly interface (no bindings generator) |
| `crates/goldl-cli` | the `goldl` command |
| `web` | Svelte + TypeScript + Vite playground (custom editor, WebGL2 renderer) |
| `docs/PLAN.md` | design notes |

The Rust code has no external dependencies. The playground depends only on Svelte,
TypeScript and Vite.

## Numbers

| example | AND gates | crossings | components | clock period (generations) | pattern size (cells) |
|---|---:|---:|---:|---:|---|
| blinker | 0 | 1 | 70 | 41,968 | 5,258 × 4,504 |
| half_adder | 4 | 6 | 142 | 86,688 | 10,690 × 8,039 |
| full_adder | 11 | 17 | 391 | 180,944 | 22,364 × 18,304 |
| counter | 25 | 36 | 1,079 | 439,976 | 55,352 × 51,474 |
| traffic_light | 40 | 56 | 2,012 | 802,896 | 100,761 × 88,255 |
| lfsr | 33 | 60 | 1,677 | 659,448 | 83,395 × 78,602 |
| ripple_adder | 44 | 59 | 1,328 | 571,728 | 70,870 × 59,497 |
| popcount | 82 | 109 | 3,050 | 1,148,272 | 143,032 × 128,433 |
| register_file | 128 | 214 | 6,158 | 2,261,800 | 284,829 × 258,829 |
| alu | 309 | 394 | 10,844 | 3,824,936 | 478,216 × 455,649 |
| cpu | 718 | 1,003 | 34,453 | 11,748,632 | 1,479,191 × 1,433,410 |
| riscv | 11,253 | 11,850 | 413,182 | 153,306,696 | 19,703,806 × 18,935,080 |

Measured layout improvements and alternatives are documented in
[the experiment report](docs/experiments/README.md).
