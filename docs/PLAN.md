# GoLDL: an HDL that compiles to Conway's Game of Life — project plan

Status: **planning** (no implementation yet). This document records the approach, the key
technical decisions (with the reasoning and the evidence gathered so far), the architecture,
and a milestone plan. Sections marked **Decision** are recommendations open for review;
section 13 lists the questions that need an answer from the project owner.

---

## 0. TL;DR

* **Signal encoding.** One glider per net per clock cycle, at a statically scheduled generation:
  glider present = `1`, absent = `0` (return-to-zero pulse logic, like RSFQ superconducting
  logic). No periodic glider streams, no glider guns per gate.
* **Logic primitive = a bare glider crossing.** Two gliders on perpendicular lanes that meet
  with one of a handful of exact timings annihilate cleanly. Each continuation lane therefore
  computes `A ∧ ¬B` / `B ∧ ¬A` with zero hardware. With a distributed constant-`1` (clock)
  net this is functionally complete. Stable ("P1") circuitry supplies everything else:
  reflectors (Snark), splitters, mergers, eaters, and an optional memory cell. The planning spike
  confirmed the core numbers (section 2.6).
* **The compiler is a real EDA flow.** Rust-like HDL → typed HIR → word-level RTL IR → bit-level
  sequential AIG → *glider netlist* (inhibit logic) → timing-driven place & route on the
  45°-rotated glider grid → Life pattern **plus a simulation database**. Every level has its own
  passes and its own simulator, and adjacent levels are checked for equivalence.
* **Simulation is hierarchical and exact.** The fast path is a compiled RTL simulator. Because
  every glider's spacetime trajectory is a pure function of the scheduled net values, the cell
  state at *any* generation `g` is reconstructed exactly from three things: the logic state of
  cycle `⌊g/T⌋`, the static layout, and precomputed per-component reaction "flipbooks". The
  cost is O(visible area), independent of `g`. True cell-level engines (sparse bit-parallel +
  HashLife) serve as ground truth, are differentially tested against the reconstruction, and
  are exposed as a "verify" button.
* **Playground.** Static web app: TypeScript + CodeMirror 6, with the Rust LSP compiled to WASM in
  a worker, WebGL2 for the Life view (a static layer plus an instanced glider overlay), a
  Sugiyama-style schematic with ANSI/IEEE distinctive-shape symbols, a waveform viewer and a
  single timeline that drives all views.
* **Plan shape.** De-risk the Life technology first (M0). Then build a thin vertical slice
  (tiny HDL subset → Life → playground), then deepen each layer.

---

## 1. Goals and non-goals

Goals

1. A small, Rust-flavoured, expression-oriented HDL (syntax per the `Alu` sample, section 3)
   for synchronous digital circuits: combinational logic, registers, memories, hierarchy and
   generics.
2. A general-purpose compiler: **every** synchronous design compiles to a working Life pattern.
   It optimizes for **generations per clock cycle** (signal delay) first and area second.
3. A high-performance simulator that can jump to any generation and recover the exact cell
   state there, without stepping every generation.
4. A web playground with a configurable editor (LSP-backed), examples (up to a small CPU), the
   Life view, the logic schematic view, and synchronized simulation.
5. Rust with **no external crates** in shipped code. Exceptions need a strong justification
   (section 9.3). The web front end may use a small number of npm packages.

Non-goals (at least initially)

* Asynchronous / multi-clock designs, latches, tri-states. There is one implicit global clock.
* Compiling to other cellular automata. The tech-library abstraction keeps this possible.
* Constructing patterns by glider synthesis (we emit the final pattern directly).

---

## 2. The core idea: how digital logic maps onto Life

### 2.1 Encoding — **Decision: single-glider pulse logic with a static schedule**

The compiler assigns each net `n` a scheduled time `τ(n)`. In clock cycle `c` the net's glider
(if the bit is 1) passes the net's reference point at generation `g0 + c·T + τ(n)`, where `T`
is the clock period in generations, chosen by the compiler.

Why not the classic encoding (periodic glider streams from Gosper guns, e.g. Rennard's gates,
Rendell's Turing machine, Loizeau's computer)?

* Streams need a gun per inverter. They force a global small period (30/60), so every lane is
  dense, which makes wire crossings and reflector recovery hard.
* Garbage values in unused stream slots would have to be simulated too.
* Single pulses with a large `T` give sparse lanes, and crossings become a pure timing
  question. Every glider event is then determined by the logic values of one cycle, and that
  is what makes the "jump to any generation" reconstruction exact and cheap.

Why not metacells (OTCA metapixel, VarLife as in Quest for Tetris)? They are orders of magnitude
larger and slower per logical operation.

### 2.2 Geometry: the rotated frame

Gliders only move diagonally (one cell per 4 generations). Use rotated coordinates
`u = x + y`, `v = x − y` (y grows downwards):

| direction | (Δx, Δy) per 4 gens | in (u, v)        | lane = constant |
|-----------|---------------------|------------------|-----------------|
| SE        | (+1, +1)            | u += 2           | v               |
| NW        | (−1, −1)            | u −= 2           | v               |
| NE        | (+1, −1)            | v += 2           | u               |
| SW        | (−1, +1)            | v −= 2           | u               |

In (u, v), glider wiring is **Manhattan routing**. Reflectors are the corners, and perpendicular
lanes always intersect.

A very useful invariant: **SE and NE gliders both move east at exactly c/4**. For any
eastbound glider the *wave phase* `φ = t − 4x` is constant along straight travel. The relative
timing of two eastbound gliders at their crossing point is therefore `φ_a − φ_b`, *wherever*
they cross. Combinational logic laid out flowing west→east thus has "x ≈ time". Placement can
reason about lags (`φ`) instead of absolute positions, and crossing safety becomes a
property of the two nets rather than of each intersection.

### 2.3 Primitive set (the "technology library")

All primitives are characterized by our own tool (section 5.1): ports (lane, direction),
input→output delays, recovery time, spacetime footprint, and reaction flipbooks.

| Primitive | Function | Implementation | Notes |
|---|---|---|---|
| `LANE` | wire | empty space | c/4; parallel lanes ≥ 7 units apart (spike) |
| `CROSS` | `a' = a∧¬b`, `b' = b∧¬a` | bare 90° collision with a vanish timing | the logic primitive; no cells at all |
| `PASS` | crossover | bare crossing with \|Δt\| ≥ 19 | timing constraint only |
| `TURN` | 90° reflection | Snark (stable) | left/right variants; also sets fine timing |
| `SPLIT` | fan-out 1→2 (or 1→3) | stable splitter (Herschel-based) | see section 2.7 for candidates |
| `MERGE` | `a ∨ b` for **mutually exclusive** a, b | stable fan-in | exclusivity is proven by the compiler |
| `ONE` | constant-1 source, period `T` | Snark-loop gun → `SPLIT` tree | only needed for inversion |
| `SINK` | absorb unused glider | eater 1 | |
| `REG` | `Q(c+1) = D(c)` | (a) delay line (Snark serpentine) or (b) stable memory cell (write + destructive read) | (a) needs Snarks only |
| `IN`/`OUT` | ports | lanes from/to the pattern boundary | inputs = glider "tapes" |

### 2.4 Logic in inhibit form

```
   a  (SE) ↘                 ↗  b' = b ∧ ¬a   (NE)
              ╲             ╱
                 ╳   CROSS: lanes intersect; a and b are timed to annihilate
              ╱             ╲
   b  (NE) ↗                 ↘  a' = a ∧ ¬b   (SE)
```

`CROSS` is the native gate. Costs per 2-input function, single-rail:

| function | construction | cost |
|---|---|---|
| `a ∧ ¬b` | `CROSS(a,b).a` | 1 cross (+ sink for `b∧¬a` if unused) |
| `¬a` | `CROSS(1,a).one` | 1 cross + 1 `ONE` tap (the `a` side never survives: no garbage) |
| `a ∧ b` | `CROSS(a, ¬b)` | 2 cross + 1 tap; the other output is **NOR(a,b) for free** |
| `a ⊕ b` | `MERGE(CROSS(a,b).a, CROSS(a,b).b)` | 1 cross + 1 merge; outputs are structurally exclusive |
| `a ∨ b` | `MERGE(a₁, CROSS(b, a₂).b)` | 1 split + 1 cross + 1 merge |
| `mux(s,x,y)` | `MERGE(CROSS(x,¬s₁).x, CROSS(y,s₂).y)` | 1 split + 3 cross + 1 tap + 1 merge |
| full adder carry | `MERGE(a∧b, c∧(a⊕b))` | exclusive by construction |

Consequences for synthesis:

* Every use of a signal needs its own glider. Fan-out costs splitters, so fan-out
  optimization is first-class (as in SFQ logic).
* Inversion is the expensive operation, so **polarity assignment** (choosing which nets carry
  the complement) is first-class, as in domino-logic synthesis.
* XOR/XNOR are cheap, which favours XOR-rich arithmetic decompositions.
* `MERGE` is only legal on provably exclusive inputs. Exclusivity is tracked structurally
  (crossing outputs, `s`/`¬s` gated pairs) and otherwise proven with a SAT/BDD check.

Alternative encoding kept in reserve: **dual-rail** (each bit on two lanes, exactly one glider
per cycle). Inversion becomes a free lane swap and no `ONE` tree is needed. The cost is twice
the lanes and splitters inside every gate. This is the analogue of clock-free xSFQ. The glider
netlist IR supports both, so the mapper can choose per design (section 4.5).

A spike finding that may become an optimization: at Δt = ±17 the two gliders produce **one**
glider kicked back along the reverse of one input lane (the known "kickback" reaction). That is
an `a ∧ b` output without a `ONE` tap, at the price of awkward output geometry.

### 2.5 Timing model

* Lane segment of `L` rotated units: +2·L generations. Every component has a fixed delay per
  port pair.
* `CROSS`: `τ_a(site) − τ_b(site) ∈ Δ_vanish(parity)` (exact equality, but there are 11
  distinct choices up to symmetry; section 2.6).
* `PASS`: `|τ_a(site) − τ_b(site) + kT| ≥ 19` for all integers `k`.
* `MERGE`: both input paths must deliver at the same `τ(out)` (exact path balancing).
* `REG`: `τ(D) + d_reg = τ(Q) + T`.
* `T ≥` max register-to-register path, and `T ≥` every component's recovery time (easily met:
  `T` is thousands of generations).
* Fine timing. A detour adds delay in steps of 4–8 generations (to be confirmed by
  characterization). The remaining residues come from three sources: the choice among `Δ_vanish`
  values, reflector variants, and lane parity. The vanish values alone already cover every
  residue mod 8 except 4 at lane parity 0, and residue 4 at parity 1. M0 must produce a
  **residue coverage table** proving every needed (lane offset, delay mod 8) adjustment is
  realizable.

This is the same problem class as path balancing in RSFQ logic synthesis (splitter insertion,
DFF-based path balancing, clock-tree synthesis). That literature is directly reusable.

**Generation 0** is a cycle boundary: the schedule guarantees that only register lines and the
`ONE` network hold in-flight gliders at that instant. Both have known contents (register
initial values, constant 1), so the emitted pattern is *by definition* the reconstruction at
`g = 0` (section 6.2). Initialization and simulation therefore share one code path.

**Back-of-envelope for the CPU example** (to be replaced by measurements):

* about 3k AIG nodes → about 10–15k GNL primitives;
* a horizontal pitch of about 60–100 cells per logic level and a critical loop of about 30
  levels give `T ≈ 10–20k` generations per cycle;
* the layout is on the order of 10⁴ × 10⁴ cells;
* about 10⁴ gliders are in flight at any time.

That is far too slow for per-cell stepping, which is the motivation for section 6. It is
trivial for the abstract simulator: microseconds per cycle, milliseconds per frame.

Note on the objective: the playground's simulation cost is **independent of `T`** (section 6).
Minimizing `T` still matters. It is the circuit's real speed in Life. It drives the cost of
cell-level verification and of running exported patterns in Golly. It also directly reflects
layout quality.

### 2.6 Evidence from the planning spike (`docs/spikes/glider_collisions.py`)

Brute-force Life simulation of SE-vs-SW glider pairs over every relative timing (`dt` = arrival
time difference at the lane intersection) and both lane parities:

* **Interaction window:** gliders interact iff `|dt| ≤ 18`. Every `|dt| ≥ 19` passes cleanly.
  So a non-interacting crossover needs only a 19-generation separation, which is tiny compared
  with `T`.
* **Clean annihilation ("vanish")** at 22 (dt, parity) classes, 11 up to mirror symmetry:
  dt ∈ {±4, ±11} (parity 1, finished 3 generations after contact), ±15 (parity 0, 3 gens),
  ±5, ±7, ±8, ±13, ±6, ±14 (parity 0, 5–12 gens), ±3 (parity 0, 31 gens), ±1 (parity 1, 93
  gens). The fast ones are preferred (smaller spacetime footprint).
* **Parallel lanes** in the same direction never interact iff their lane offset (x − y units)
  is ≥ 7.
* No interacting two-glider 90° collision yields exactly two gliders and nothing else, so
  fan-out *requires* a catalytic (stable) splitter or a gun. Δt = ±17 (parity 0) yields the
  single-glider kickback.

### 2.7 Component sourcing (M0)

Known stable components are taken from the Life engineering literature (LifeWiki,
conwaylife.com forums, Golly's pattern collection). Each entry records its provenance and
licence, and is **re-verified by our characterizer**: nothing enters the library on trust.

> Research brief on concrete candidates (Snark, splitters, fan-ins, memory cells, loop guns,
> prior art): see section 2.8. It is filled in from the M0 research task.

Fallbacks, in increasing cost, if a component class is missing:

* `MERGE` missing → implement OR as `¬(¬a ∧ ¬b)` (3 crosses + 2 `ONE` taps).
* Memory cell missing → delay-line registers (Snarks only).
* Stable splitter missing → gun-based duplicators. This is the expensive fallback and forces
  `T` to a multiple of the gun period.

### 2.8 Component research brief

_(to be completed from the research task; see the commit history)_

### 2.9 Alternatives considered

* **Isochronous tile fabric ("meta-CA").** Fixed-size tiles with a uniform traversal time, so
  the layout is a systolic array and reconstruction is a pure table lookup. It is simpler and
  correct by construction, but carries a large area and delay overhead. It is kept as the
  **fallback back end** if free-form timing closure proves too hard.
* **Timing-tolerant logic (regulators, set/read memory gates).** Removes exact-timing
  constraints at the cost of much bigger components. Humans prefer it because hand-timing is
  painful. A compiler doesn't mind exact integer timing, so we take the compact route.

---

## 3. The language

Working name GoLDL, file extension `.gdl`. Rust-like, expression-oriented, strictly
width-typed, with a single implicit clock.

### 3.1 Example (the target syntax)

```rust
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

Sequential logic and hierarchy (proposed):

```rust
module Counter<N: uint>(en: bit) -> (count: bits<N>, wrap: bit) {
  reg r: bits<N> = 0                 // value at generation 0 (no reset wire needed)
  r.next = if en { r + 1 } else { r }
  count = r
  wrap = en && r == ~0               // `~0` adapts to bits<N>
}

module Top(en: bit) -> (lo: bits<4>, hi: bits<4>) {
  let c0 = Counter<4>(en: en)
  let c1 = Counter<4>(en: c0.wrap)
  lo = c0.count
  hi = c1.count
}
```

### 3.2 Semantics and rules

* **Types:** `bit` (= `bits<1>`), `bits<N>` with `N` a const expression (`bits<N+1>`,
  `bits<clog2(N)>`), fixed arrays `T[N]`, `enum E: bits<k> { … }`, tuples for multiple results.
  Later: `struct` bundles with bit-cast.
* **Literals** are unsized and adapt to the expected width (`0x2A`, `0b1010`, `1_000`). It is an
  error if they don't fit. `~0` is all ones.
* **Strict widths:** binary arithmetic and bitwise operators require equal widths (results wrap).
  Conversions are explicit (`zext`, `sext`, `trunc`), as in the sample.
* **Operator precedence = Rust's** (`*` > `+ -` > `<< >> >>>` > `&` > `^` > `|` >
  comparisons > `&&` > `||`). So `op == 0 || op == 1` and `a & b == c` mean what Rust readers
  expect. Concatenation `a ++ b` (msb first) sits between shifts and `&`. Comparisons are
  unsigned; signed helpers are `slt`, `sle`, `>>>`.
* **Bit selection:** `x[i]` (const or dynamic; dynamic lowers to a mux), `x[hi:lo]` inclusive
  with a constant width.
* **`if`/`else`** is an expression (condition must be `bit`). `else if` chains are priority muxes.
* **`match`** (deliberate difference from Rust): literal/range/or-pattern arms must be **pairwise
  disjoint**, and `_` is the default **wherever it appears** (the sample puts it first). Order is
  irrelevant, which maps directly to a parallel case. A missing `_` on a non-exhaustive match is
  an error. A `_` that covers nothing is a warning.
* **Single assignment:** each output and each declared-but-unassigned `let x: T` is driven exactly
  once. Partial (`x[3:0] = …`) assignments are allowed if they cover every bit exactly once
  (checked).
* **Registers:** `reg r: T = init`, plus `r.next = expr`. Reading `r` gives the current value. A
  missing `.next` means hold, with a lint. Initial values *are* the Life pattern's state at
  generation 0 (gliders placed in register lines), so reset is just ordinary logic.
* **Combinational loops** (not through a `reg`) are errors.
* **Modules** have const generics (`<N: uint>`) and are instantiated with named or positional
  arguments, giving a value with output fields (`alu.y`) or a destructured tuple.
* **`fn`** declares pure combinational functions (inlined), with a block-expression body.
* **`for i in 0..N { … }`** is an elaboration-time generate loop.
* **`const`** items, including ROM tables (`const PROG: bits<8>[16] = [...]`). Dynamic indexing
  into them synthesizes ROM logic.
* **`mem m: bits<8>[16]`** declares RAM with `m.read(addr)` / `m.write(addr, data, en)`
  (read-old-value semantics). It is lowered to registers plus decoders (a delay-line memory back
  end is possible later).
* **Attributes:** `#[top]`, `#[probe]` (keep and show in views), `#[period(N)]` (force `T`),
  layout hints.
* **Tests:** `test "name" { let d = Counter<4>(); d.en = 1; step(3); assert d.count == 3 }`.
  These are imperative testbench blocks, run at RTL and optionally at the gate, glider or cell
  level. They also feed stimulus to the playground.

### 3.3 I/O in the Life world

* Top-level **inputs** enter through boundary lanes. A recorded or test-provided input sequence
  is emitted as an *input tape*: gliders pre-placed outside the circuit, one slot per cycle, so
  the exported pattern is self-contained. In the playground, interactive inputs simply append to
  the input log, which the abstract simulator consumes directly.
* **Outputs** leave through boundary lanes into eaters. Their values come from the logic model,
  and the Life view shows the gliders arriving.

---

## 4. Compiler architecture and IR stack

### 4.1 Pipeline

```
 source ─► lexer/parser ─► CST (lossless) ─► AST
        ─► HIR: name resolution, const eval, monomorphization, width inference/checking
        ─► RTL IR (word-level dataflow, hierarchical)            [RTL simulator]
        ─► flattened RTL ─► bit-blast ─► sequential AIG           [gate simulator]
        ─► glider netlist (GNL: CROSS/MERGE/SPLIT/ONE/REG/…)       [pulse simulator]
        ─► placement ─► routing ─► schedule/timing ─► DRC
        ─► Layout DB ─► (a) Life pattern (RLE / Macrocell)
                         (b) Simulation DB (routes, schedule, flipbook refs, provenance)
```

Every lowering step records **provenance**: GNL node → AIG node → RTL op → source span.
Cross-probing, hover values, schematic↔Life highlighting and error messages all depend on it.

### 4.2 Front end

* Hand-written lexer and recursive-descent + Pratt parser with error recovery. Produces a
  **lossless CST** (green/red tree, rowan-style, home-made). The formatter, LSP and
  incremental reparse all build on it.
* HIR: interned symbols, arena-allocated, stable IDs (so the LSP can map back).
* Sema: name resolution → const evaluation (generic params, `for` bounds, widths) →
  monomorphization (one module instance per parameter set) → type and width checking with
  literal adaptation → single-driver and coverage checking → combinational-loop detection
  → lints (unused, truncation, constant register, unreachable arm).
* Diagnostics carry spans, labels, notes and suggested fixes (also used as LSP code actions).
* Incrementality: a small memoized query layer keyed by file content hash (parse → item
  tree → per-module sema). It is not a full salsa, but keeps LSP latency in the milliseconds.

### 4.3 RTL IR (word level) and passes

A dataflow graph per module: `Const`, `Input`, `Not/And/Or/Xor`, `Add/Sub/Mul`, `Shl/Shr/Sar`
(constant or variable), `Eq/Ne/Ult/…`, `Mux`, `ParallelCase` (from `match`), `Concat`,
`Slice`, `Zext/Sext`, `Reg(init)`, `MemRead/MemWrite`, `Instance`.

Passes:

1. Constant folding and propagation, algebraic simplification, CSE / hash-consing.
2. Demanded-bits / width narrowing (for example, `wide[7:0]` only needs the low 8 sum bits).
3. Dead-code elimination; register clean-ups (constant, duplicate, unobservable registers).
4. `match` lowering: one-hot `ParallelCase` vs mux tree, by cost.
5. Memory lowering (`mem` → registers + decoders + write logic).
6. Flattening (provenance kept). The hierarchical form is kept for the schematic.
7. **Timing-driven datapath lowering:** adder, comparator and shifter architecture is chosen
   from the critical-path estimate (ripple vs. carry-select vs. prefix). Here delay is physical
   distance, so deep ripple chains are expensive *and* prefix trees cost wiring.

### 4.4 Gate IR (sequential AIG) and passes

Bit-blasting into an and-inverter graph with latches (registers keep a stable mapping to RTL
registers, so RTL state can drive lower levels; see section 6).

1. Structural hashing, constant propagation, balancing (depth reduction).
2. Cut-based rewriting with a precomputed table of optimal 4-input structures. The table is
   generated offline by exhaustive enumeration (by our own tool, checked in).
3. SAT sweeping / FRAIG (needs a small home-made CDCL solver; same solver used for equivalence
   and `MERGE` exclusivity checks). Later.
4. Optional retiming. Disabled by default because it breaks the RTL ↔ gate register mapping;
   when enabled, the mapping must be carried along.

### 4.5 Glider netlist (GNL) and technology mapping

GNL nodes: `CROSS`, `MERGE`, `SPLIT(k)`, `ONE`, `SINK`, `REG`, `IN`, `OUT`. Edges are glider
paths, each with an abstract (pre-layout) delay.

1. **Exact synthesis of the cell library.** Offline enumeration of minimal GNL networks for
   all 2- and 3-input functions (and the frequent 4-input ones) under a (crosses, splits, taps,
   merges, depth) cost. Negation is not free, so only permutation-equivalence classes apply.
2. **Polarity assignment.** Choose for each AIG node whether to materialize `f` or `¬f` so as
   to minimize `ONE` taps, using the free complementary outputs of `CROSS`.
3. **Cut-based technology mapping** onto the exact-synthesis table with delay-oriented,
   area-recovering cost (as in ABC's mappers).
4. **Fan-out trees.** Splitter trees are built timing-aware: critical sinks get the shallow
   branches.
5. **`ONE` (clock) tree synthesis.** A loop gun of period `T` feeds a splitter tree, and taps
   are placed late, near their consumers. The trade-off is one global (folded) loop plus a deep
   tree, versus several local loop guns (all period `T`, phases set by their seed gliders at
   generation 0) with shallow trees.
6. **Exclusivity verification** for every `MERGE` (structural first, SAT fallback).
7. Garbage outputs → `SINK`.

### 4.6 Physical design (place, route, time)

**Decision (v1 architecture): folded levelized "eastbound fabric".**

* Combinational logic flows west → east (SE/NE lanes), so x ≈ time and nets are reasoned about
  by wave phase `φ` (section 2.2). A `CROSS` always takes one SE and one NE input, and both
  outputs keep going east. `TURN` swaps SE ↔ NE without losing eastward progress.
* To avoid paying a pure return trip for feedback, the fabric is **folded** (U-turn, or a ring
  for deep designs). Logic flows east in one band, turns around, and continues west (SW/NW) in
  the next, so register outputs land next to the logic that consumes them. The loop delay is
  then almost all useful logic.
* Placement: levelize by ASAP time. Order nodes within each column with barycenter/median
  heuristics plus transpositions, to minimize wire length and unintended crossings. This is the
  same Sugiyama machinery as the schematic view (shared crate). Lane pitch follows the design
  rules (≥ 7 units parallel, ≥ 6 head-on, footprint clearances).
* Routing: channel routing between columns. v2 adds PathFinder-style negotiated congestion on
  the rotated grid, with spacetime conflicts as resources.
* **Schedule solving:** compute ASAP arrival times. For each `CROSS`, pick the `Δ_vanish` option
  and delay the earlier input. For each `MERGE`, balance its inputs. Insert detours or choose
  reflector variants to hit exact residues. Check every unintended crossing for `|Δt| ≥ 19`
  (mod `T`) and repair violations by re-phasing a net. Finally, set `T` and pad register paths.
  The equalities and inequalities form a difference-constraint system (Bellman-Ford / longest
  path) plus a small residue-assignment search.
* Registers: delay lines (folded Snark serpentines) when the padding is short; memory cells
  (if available) when the padding is long.
* **Spacetime DRC:** a sweep over all segment pairs and segment/footprint pairs, using the
  characterized interaction rules. The final gate is cell-level simulation of K random cycles
  (section 7).
* v2/v3 quality work: analytical timing-driven placement, compaction, ring/spiral folding for
  multi-stage designs, and register placement co-optimized with `T`.
* **Completeness guarantee:** the back end must never fail on a legal design. If the optimizing
  flow cannot close timing or DRC, it falls back progressively: wider lane pitch, larger phase
  separations, larger `T`, and finally the conservative tile fabric (section 2.9). The result
  may be slow, but it always compiles.

Output: a **Layout DB** (component instances with orientation, lane segments, the schedule,
`T`) from which both the pattern and the Simulation DB are emitted.

---

## 5. Technology library tooling (`goldl-tech`)

### 5.1 Characterizer

A Rust tool that, given a component RLE and its intended ports:

* Finds accepted input lanes and timings, outputs (lane, direction, delay), recovery time
  (minimum spacing between consecutive inputs), and failure modes.
* Records the **spacetime footprint** of each reaction (the bounding box per generation until
  the component is back at rest and the outputs are clear), plus **flipbooks**: the cell diffs
  vs. the quiescent state for each generation of each reaction.
* Enumerates bare glider–glider interactions (the Rust port of the spike), including
  head-on / parallel spacing rules and pass windows.
* Searches combinations (reflector pairs, Herschel conduits) for the **residue coverage
  table** (any lane offset × delay mod 8).
* Output: generated Rust data (checked in), plus tests that re-run the characterization and
  fail on drift.

### 5.2 Offline searches

* Exact synthesis tables for GNL (section 4.5).
* Optional: search for better small primitives (e.g. compact AND via kickback, three-glider
  reactions). This is research, not on the critical path.

---

## 6. Simulation architecture (the "holistic" simulator)

### 6.1 Tiers

| Tier | Model | Use | Speed target (native / wasm) |
|---|---|---|---|
| T0 | RTL compiled sim (word-level bytecode over `u64`s) | fast-forward, tests, waveforms | 1–10 M cycles/s for small designs; ~1 M for a small CPU |
| T1 | Gate (AIG) / GNL pulse sim, 64-way bit-parallel | equivalence checks; one-cycle evaluation for reconstruction | — |
| T2 | Glider event model: event = (net, cycle, τ) ⇒ exact trajectory | the "signals as gliders on lines" view | implied by T0/T1 |
| T3 | **Exact cell reconstruction** at generation `g` for a viewport | the Life view at any `g` | O(visible objects) per frame |
| T4 | True Life engines: sparse tiled bit-parallel stepping + HashLife | ground truth, "verify", raw sandbox | QuickLife/HashLife class |

### 6.2 Exact reconstruction (how "go to generation 700 000" works)

1. `c = ⌊(g − g0)/T⌋`. Register state for cycles `c` and `c−1` comes from T0 checkpoints (every
   K≈1024 cycles: register bits + an input-log cursor), so this is ≤ K RTL steps. That is
   microseconds to milliseconds even after a million cycles.
2. Evaluate the GNL for the needed cycles, lazily and only for the cones of nets that intersect
   the viewport (memoized). This gives presence bits for every glider event of cycles `c−1` and
   `c` (registers span the cycle boundary; only two cycles can be live at once by construction).
3. For each lane segment in the viewport (uniform-grid spatial index): if its net is 1 and the
   glider's scheduled position at `g` lies inside the segment, emit the glider at that position
   and phase `(g − t) mod 4`.
4. For each component in the viewport: if an input event lies within its reaction window
   `(g − R, g]`, emit flipbook frame `g − t_event` (selected by which inputs fired). Otherwise
   it is quiescent and comes from the static layer.
5. The `ONE` loop gun is just a net with period `T` and is handled identically.

Exactness rests on two invariants checked at compile time: reactions are confined to their
characterized spacetime footprints, and distinct live footprints and glider paths never
overlap (spacetime DRC). Cost is independent of `g` and of the total pattern size. Rendering
can therefore jump or scrub anywhere and play at any speed (1 gen/frame up to millions of
gens/frame) at the same frame cost.

Exporting the full pattern at generation `g` (to RLE/Macrocell, or to seed T4) is the same
procedure over the whole layout.

### 6.3 Cell-level engines (T4)

* **Tiled bit-parallel engine:** 64×64 tiles as `[u64; 64]`, a full-adder neighbour count
  (~1 ns per 64 cells natively, `simd128` in wasm), an active-tile set with change tracking so
  still lifes cost nothing. Used for viewport "free-run from here", tests and DRC checks.
* **HashLife:** arena nodes with `u32` ids, open-addressing hash, GC, arbitrary step sizes
  (power-of-two decomposition), 16×16 leaves stepped by the bit-parallel kernel. Used for
  "verify at generation g" and for the general Life sandbox.
* RLE and Golly Macrocell import/export.

### 6.4 GPU — **Decision: WebGL2 for rendering; CPU (wasm) for simulation**

These circuits are extremely sparse in *activity* (≪ 1 % of cells change per generation), and
the abstract model removes per-generation work altogether. Dense GPU stepping would therefore be
the wrong tool for the main path. A WebGPU compute stepper could be added later for the raw
sandbox mode on dense random patterns.

---

## 7. Verification strategy

* **Front end:** golden tests (CST, AST and diagnostics snapshots); fuzzing of the parser via a
  home-made generator/mutator; formatter idempotence and round-trip.
* **Between IR levels:** 64-way bit-parallel random co-simulation (RTL vs AIG vs GNL) for
  thousands of cycles. SAT-based combinational equivalence once the solver exists.
* **Tech library:** characterization re-run in tests (drift = failure).
* **Layout:** spacetime DRC, then **cell-level end-to-end**: run the compiled pattern with T4
  for K cycles under random input tapes, decode outputs at the output ports, and compare with
  T0.
* **Reconstruction:** at random generations, compare T3's reconstructed cells with T4's
  result, bit for bit. This is a property test over all examples, with a small home-made PRNG
  and shrinker.
* **Examples are tests:** every playground example must pass all levels in CI (small K for the
  CPU).
* **In the playground:** a "verify" button that runs HashLife from generation 0 to the current
  `g` in a worker and diffs against the reconstruction.

---

## 8. Web playground

### 8.1 Architecture

* Static site (Vite + TypeScript), deployable to GitHub Pages.
* **One wasm module** (compiler + simulators + LSP + schematic layout), instantiated in:
  * an **LSP worker** (parse/sema on every edit; milliseconds),
  * a **build/sim worker** (full compile, debounced; the RTL sim, checkpoints and
    reconstruction).
* **ABI:** plain `extern "C"` exports. Strings and blobs go through `alloc`/`free` and
  `(ptr, len)`. Bulk data (render buffers, waveforms) is read as typed-array views over wasm
  memory, with zero copies, and transferred to the main thread as `ArrayBuffer`s. There is no
  wasm-bindgen and no SharedArrayBuffer requirement (no COOP/COEP headers needed).
* Build: `cargo build --target wasm32-unknown-unknown --release` (`panic=abort`, LTO,
  `+simd128`); `wasm-opt` is optional if installed.
* Performance budgets in the browser:

  | Operation | Budget |
  |---|---|
  | LSP round-trip on edit | < 20 ms |
  | Full compile of the ALU | < 1 s |
  | Full compile of the CPU | < 5 s (optimizing "release" layout may be opt-in) |
  | Seek to any generation | < 50 ms |
  | Frame time at any zoom or speed | < 8 ms |
  | Wasm module size | < 3 MB (uncompressed) |

### 8.2 Editor and LSP

* **CodeMirror 6** (lighter and more modular than Monaco, which matters for a playground). It
  has a Lezer grammar for instant highlighting and keymaps (default / vim / emacs).
* LSP server in Rust (`goldl-lsp`), transport-agnostic: `handle(json) -> Vec<json>`. In the
  browser it sits behind a worker `postMessage`; natively it is `goldl lsp` over stdio, so a
  VS Code extension is nearly free. The client is either `@codemirror/lsp-client` (if it covers
  our needs) or a small home-made adapter.
* Features, in priority order: diagnostics, semantic tokens, hover (type, width, **current
  simulated value**), go-to-definition and references, document symbols and outline,
  completion (keywords, locals, ports, modules, builtins with snippets), signature help, inlay
  hints (inferred widths and **live values**), formatting, rename, code actions (insert `zext`,
  add missing match arm), folding.
* Configurable: theme, font and size, keymap, tab width, format-on-save, inlay hints on/off,
  diagnostics delay, pane layout. Settings persist in `localStorage` (wrapped in try/catch).
  Shareable links carry the compressed source in the URL hash.

### 8.3 Life view (WebGL2)

* **Static layer:** all stable components rendered once into a tiled texture pyramid (LOD
  mipmaps using "any live cell" reduction), so the whole circuit is visible zoomed out.
* **Dynamic overlay:** gliders in flight and active reaction flipbook frames, produced by T3 for
  the visible region and drawn with instanced quads. Even a CPU has only ~10⁴ gliders in
  flight, which is trivial per frame.
* Zoom from whole-pattern overview to individual cells; a grid at high zoom; hover a glider or
  component to see its net or source signal (provenance); click to cross-probe.
* Raw mode: hand the reconstructed state to T4 and step genuinely (for skeptics and for
  sandboxing).

### 8.4 Schematic view (ANSI/IEEE distinctive-shape symbols)

* Two levels:
  * **RTL view:** hierarchical module blocks, adders, muxes (trapezoids), registers (box with a
    clock wedge), buses with width labels.
  * **Gate view:** AND (D), OR (shield), XOR (double curve), NOT/NAND/NOR/XNOR with bubbles,
    buffers, DFFs.
  * A **GNL view** shows the glider primitives themselves.
* Layout in Rust (`goldl-schematic`):
  * layering by longest path, with register edges cut as feedback edges routed as clean
    return wires;
  * crossing minimization (barycenter + transposition);
  * Brandes–Köpf coordinate assignment;
  * orthogonal routing with channel track assignment (left-edge) and bus bundling;
  * collapsible hierarchy.
  The output is a display list; the front end renders it to Canvas2D with cached `Path2D`
  symbols (virtualized for large netlists) and exports SVG.
* Wires coloured by value at the current cycle. Within a cycle, the timeline position `g mod T`
  can animate the signal front as gliders arrive.

### 8.5 Timeline, I/O and waveforms

* A single source of truth: the current generation `g` (also shown as cycle `c` + phase).
  Controls: play/pause, log-scale speed (1 gen/s … 10⁹ gens/s), step by generation or cycle,
  a scrubbable slider.
* Inputs panel: per-input toggles or a per-cycle stimulus table, or "run test X".
* A waveform viewer for probed signals over cycles, with VCD export.
* A stats panel: cells, bounding box, `T` (generations per cycle), component counts, critical
  path, compile time.

### 8.6 Examples (each one is also a CI test)

1. NOT, buffer (shows a `ONE` tap and a `CROSS`)
2. AND / OR / XOR / XNOR
3. Half adder, full adder
4. 4-bit ripple-carry vs. 8-bit prefix adder (a visible `T` difference)
5. 2:1 and 4:1 mux, 3→8 decoder, priority encoder
6. Comparator, popcount, 7-segment decoder
7. D flip-flop toggle, T flip-flop, 4-bit counter, Gray counter, Johnson counter, LFSR
8. Shift register, serial→parallel
9. FSMs: traffic light, sequence detector, vending machine
10. The 8-bit `Alu` from the spec
11. Barrel shifter, 4×4 array multiplier, Euclid GCD (multi-cycle)
12. 4×8 register file
13. **CPU:** an 8-bit accumulator machine (about 16 instructions, ROM program in a `const`,
    small RAM via `mem`, output port), running Fibonacci/primes. The stretch goal is a
    4-register RISC.
14. Meta: a small toroidal Life board (e.g. 6×6) implemented in GoLDL and compiled into Life.

---

## 9. Repository layout

### 9.1 Layout

```
Cargo.toml                 (workspace)
crates/
  goldl-util/              arenas, interner, bitvec, small JSON, PRNG, test helpers
  goldl-syntax/            lexer, parser, CST/AST, formatter
  goldl-sema/              resolution, consts, monomorphization, types, elaboration → RTL
  goldl-rtl/               RTL IR, passes, RTL simulator
  goldl-gate/              AIG, passes, gate sim, SAT solver
  goldl-tech/              component library, characterizer, flipbooks, generated tables
  goldl-map/               GNL, tech mapping, polarity, fan-out and ONE trees
  goldl-pnr/               placement, routing, schedule/timing, DRC, pattern emission
  goldl-life/              Life engines (tiled bit-parallel, HashLife), RLE/Macrocell I/O
  goldl-sim/               Simulation DB, event model, reconstruction, checkpoints
  goldl-schematic/         layered schematic layout (also reused by goldl-pnr placement)
  goldl-lsp/               LSP core (transport-agnostic)
  goldl-driver/            pipeline orchestration, memoized queries
  goldl-cli/               `goldl check|build|sim|test|fmt|lsp|verify|bench`
  goldl-wasm/              C-ABI exports for the playground
tools/                     offline searches (collision tables, exact synthesis, residues)
examples/                  *.gdl (shared by tests and the playground)
web/                       playground (Vite + TS)
docs/                      PLAN.md, language spec, design notes, spikes/
```

### 9.2 Engineering conventions

* Workspace-wide `cargo fmt`, `clippy -D warnings`, `cargo test`. CI on GitHub Actions also
  builds the wasm and the site and runs the example suite at all levels.
* Benchmarks use a small home-made harness, so the bench path needs no criterion.

### 9.3 Dependency policy

* Shipped crates have zero external dependencies.
* Candidates for exceptions, decided case by case:
  * none needed for wasm, because the raw ABI avoids `wasm-bindgen`;
  * dev-only crates (e.g. a property-testing or benchmark crate) are acceptable if the
    home-made versions become a time sink.
* Front end: Vite, TypeScript, CodeMirror 6 (+ lang/lint/autocomplete/vim packages), possibly
  `@codemirror/lsp-client`. No UI framework is needed (vanilla TS components); Preact is the
  fallback if UI state gets complex.

---

## 10. Milestones

Each milestone ends with something demonstrable and tested.

| # | Milestone | Content | Exit criteria |
|---|---|---|---|
| **M0** | Life tech de-risking | workspace + CI; `goldl-life` (naive + tiled engines, RLE); Rust characterizer reproducing the spike; import and characterize Snark, eater 1, splitter, merger, loop gun (+ memory cell candidates); residue coverage table; **hand-assembled** NOT / AND / XOR / half adder with a `ONE` loop, verified at cell level | Verified primitive set (or documented fallbacks) and timing rules |
| **M1** | Thin vertical slice | Minimal language (bits, `let`, `~ & ^ \|`, `reg`) → RTL → naive mapping → naive (non-folded) layout → RLE + Simulation DB → reconstruction; minimal web page showing the Life view of a compiled counter with a timeline | Counter runs in the browser; reconstruction equals cell sim at random `g` |
| **M2** | Full front end | Full grammar, CST, sema, diagnostics, formatter, generics, `for`, `fn`, `match`, `mem`, `const` ROMs, tests; RTL passes + compiled RTL sim; CLI | All examples parse, check and simulate at RTL; spec document written |
| **M3** | Synthesis | Bit-blasting, AIG passes, exact-synthesis tables, polarity assignment, cut mapping, fan-out and `ONE` trees, exclusivity checks; co-simulation equivalence | Examples through GNL with equivalence checks green |
| **M4** | Physical design | Folded levelized placement, channel routing, schedule solver, register implementation, spacetime DRC, emission | All examples (except perhaps the CPU) compile and pass cell-level end-to-end tests |
| **M5** | Simulation engine | Checkpoints, lazy cone evaluation, T3 at scale, HashLife, verify-at-g, Macrocell export | Seek to any `g` < 50 ms for the ALU; differential tests green |
| **M6** | Playground | Editor + LSP worker, build worker, WebGL2 Life view with LOD, timeline, I/O panel, examples, settings, share links | Usable playground deployed |
| **M7** | Schematic + waveforms | RTL / gate / GNL schematic views, cross-probing, value colouring, waveform + VCD | Schematic of the CPU is readable |
| **M8** | Quality of results | Timing-driven datapath choices, better placement (ring folding, analytical), compaction, memory cells, SAT sweeping, kickback-AND; CPU `T` and area targets; RTL→wasm JIT if T0 is a bottleneck | CPU example meets agreed `T`/area targets |
| **M9** | Polish | Docs/tutorial, VS Code extension (optional), dual-rail mode (optional) | — |

Parallelizable streams after M1:

* front end (M2), synthesis (M3) and the Life engines / simulation (M5) are largely independent;
* the web shell (M6) can start against M1's wasm API.

---

## 11. Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| A needed stable component (splitter, fan-in, memory cell) is unavailable, too big or too slow | area/T blow-up | M0 first; characterizer; fallbacks in section 2.7; the tech library is an interface, so components can be swapped |
| Fine timing residues not all reachable | timing closure fails | multiple `Δ_vanish` choices + reflector variants + Herschel conduits; residue table proven in M0; tile-fabric fallback (section 2.9) |
| P&R too slow or poor in wasm | bad UX or large `T` | levelized v1 is near-linear; heavy optimization runs as an opt-in "release" compile; cache layouts by netlist hash |
| Reconstruction diverges from true Life | wrong visuals (credibility!) | spacetime DRC; differential property tests; "verify" button |
| Pattern size for the CPU (10⁸ cells) | memory / render cost | static LOD pyramid + sparse dynamic overlay; HashLife for raw mode |
| Scope (this is several projects) | never ships | vertical slice first; milestones with demos; defer optimizations |
| Zero-dependency policy (JSON, LSP, SAT, layout, ABI all home-made) | schedule | keep each minimal and purpose-built; allow dev-only exceptions |

---

## 12. Prior art and references

* Rennard, *Implementation of logical functions in the Game of Life* (2002): stream-based gates
  with Gosper guns.
* Rendell, Turing machine (2000) and universal TM (2010) in Life.
* Loizeau, programmable computer in Life (p60 stream logic).
* Quest for Tetris (2017): VarLife → OTCA metapixels.
* Goucher et al.: stable "Spartan" Herschel circuitry, APGsembly / pi calculator, 0E0P
  metacell. Playle's Snark (2013).
* Collision-based computing (Adamatzky; Fredkin–Toffoli billiard-ball logic): the
  interaction-gate view of `CROSS`.
* RSFQ/xSFQ EDA: pulse logic, splitter trees, path balancing, clockless dual-rail.
* Gosper's HashLife (1984); Golly's QuickLife and HashLife implementations.
* ABC (Berkeley): AIGs, rewriting, cut-based mapping, FRAIG.
* McMurchie & Ebeling, PathFinder (1995); Sugiyama et al. (1981); Brandes & Köpf (2001).
* Spade (Rust-inspired HDL): syntax inspiration for registers and pipelines.

---

## 13. Open questions for the project owner

1. **Encoding/technology:** single-glider pulse logic with exact timing (compact, fastest),
   versus timing-tolerant or tile-based designs (simpler, larger). The plan assumes the former,
   with the tile fabric as a fallback.
2. **Register syntax:** `reg r: T = init` + `r.next = …` (proposed), vs. a one-statement form
   like Spade's, vs. `r <= …`.
3. **`match` semantics:** disjoint arms with `_` as a position-independent default (as the
   sample implies), vs. Rust's first-match.
4. **Reset:** initial values only (proposed), or also an explicit `reset` input convention?
5. **File extension** (`.gdl`?) and **project name**.
6. **Front end:** vanilla TS + CodeMirror 6 (proposed) vs. React/Svelte and/or Monaco.
7. **Hosting** of the playground (GitHub Pages?).
8. **CPU example scope:** 8-bit accumulator (proposed) vs. a small RISC.
9. **Bundled patterns licence:** confirm LifeWiki/Golly-sourced components are acceptable
   (with attribution).
