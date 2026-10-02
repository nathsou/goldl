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
- a WebGL view of the running universe, with an abstraction overlay that labels the cells
  (adder, multiplexer, ROM, register file…);
- a logic schematic with ANSI symbols;
- waveforms.

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

This ALU becomes 8,612 Life components spread over 450,000 × 197,000 cells, with a clock
period of 1.8 million generations. The included 8-bit CPU runs a Fibonacci program as
2.1 million live cells. It is verified cell by cell for 12 clock cycles: 200 million
generations of real Life, compared with the reconstruction.

## Quick start

```sh
cargo build --release
./target/release/goldl test examples/cpu.goldl        # run the test benches
./target/release/goldl run examples/cpu.goldl 120     # RTL simulation
./target/release/goldl build examples/alu.goldl -o alu.rle   # pattern for Golly
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
| match | `match op { _ => default, 2 => a & b, 3 | 4 => … }`: disjoint arms, `_` anywhere |
| enums | `enum Light: bits<2> { Red, Green }`, `Light::Red` |
| functions | `fn maj(a: bit, b: bit, c: bit) -> bit { (a & b) | (a & c) | (b & c) }` (inlined) |
| loops | `for i in 0..N { … }` (unrolled) |
| instances | `let c0 = Counter<4>(en: en)`, then `c0.count` |
| builtins | `zext sext trunc cat rep clog2 width any all parity mux slt sle sgt sge` |
| tests | `test "name" for Cpu { step(8) assert valid == 1 && out == 1 }` |

Widths are checked: there are no implicit truncations. Combinational loops are reported
with their path. The examples in [`examples/`](examples) range from a flip-flop to the
Glider-8 CPU.

## How it compiles

```
source ─▶ elaboration ─▶ RTL ─▶ AIG ─▶ glider netlist ─▶ phase schedule ─▶ staircase layout ─▶ Life pattern
           (lazy, typed)  word    AND/     crossings,       slack-minimising   blocks + exact      + exact
                          level   inverter splitters,       LP, timing-driven  glider routes       reconstruction
                                  graph    constant streams fan-out trees
```

- **Logic: inhibition.** Two gliders on perpendicular lanes annihilate. A crossing therefore
  computes `a ∧ ¬b` on one output and `b ∧ ¬a` on the other. With a constant stream of
  gliders (a "One"), this gives NOT and AND, so every circuit can be built. Fan-out uses
  Syringe duplicators. Turns use Snarks and Bandersnatch-based colour-changing reflectors.
  All components are characterised by simulation in all eight orientations, with flipbooks
  of their reactions.
- **Phase classes.** Gliders on rows travel at a phase ≡ 0 (mod 86 generations) and gliders
  on columns at ≡ 43. A turn costs exactly one class step (+43). Any row glider can then
  cross any column glider safely, so wiring never needs to check for collisions.
- **Timing.** A crossing needs both inputs at exactly the same phase. A schedule assigns a
  phase to every node: input streams, constants and register outputs have free phase, and
  the schedule minimises the total slack. Fan-out trees are rebuilt so that late consumers
  sit deep in the tree. Remaining slack is absorbed by zig-zags (+2 phases each) or by
  compact delay loops.
- **Staircase layout.** Each node becomes a block on a diagonal staircase, in topological
  order. Every output leaves on its own row, which runs below all later blocks. Every input
  owns a column that rises into its block. Rows only cross columns at right angles, so
  every connection can be routed by construction, with exactly the scheduled number of
  turns and no search.
- **Registers** are return loops around the whole circuit. The clock period is chosen so
  that every loop closes exactly.

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
| blinker | 0 | 1 | 42 | 31,820 | 3,939 × 3,060 |
| half_adder | 4 | 5 | 68 | 18,318 | 4,181 × 2,225 |
| full_adder | 11 | 14 | 171 | 42,484 | 10,133 × 4,145 |
| counter | 25 | 35 | 801 | 462,336 | 58,009 × 46,686 |
| traffic_light | 40 | 58 | 1,726 | 940,668 | 117,864 × 91,359 |
| ripple_adder | 44 | 53 | 768 | 179,998 | 43,925 × 23,281 |
| popcount | 82 | 102 | 1,992 | 427,334 | 105,877 × 56,945 |
| register_file | 128 | 190 | 4,110 | 2,343,672 | 294,669 × 212,950 |
| alu | 309 | 403 | 8,612 | 1,809,440 | 450,261 × 196,646 |
| cpu | 718 | 1,020 | 31,757 | 16,747,038 | 2,100,222 × 1,749,184 |
