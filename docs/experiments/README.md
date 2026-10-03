# Life circuit layout experiments

This report records the first two rounds, through `47d47e5`. For the subsequent
register-return correction, see [shorter return paths](return-loops.md).

The second round retains a smaller grid, paired slow turns, bounded constant-supply
banks, live-lane scheduling, and shared primary-input inversions. These are the
current defaults; compilation does not run a portfolio search or depend on
experimental environment variables.

`baseline.csv` records `d49dfae` on `main`; `round-one.csv` records `9533d7e`, before
this round; `optimized.csv` records the current implementation. The
[first-round report](round-one.md) preserves its experiments and validation.

## Measurements

Area is the axis-aligned core bounding-box width × height. Clock period is Life
generations per simulated clock. Population is **quiescent live cells** in the
components, excluding input tapes and data-dependent moving gliders. Component
count is reported separately and is not a substitute for population.

Changes relative to the **first round**, before this additional work:

| Example | Area | Clock period | Static live cells |
|---|---:|---:|---:|
| alu | -53.1% | -31.2% | -29.3% |
| blinker | -16.9% | -8.3% | -15.0% |
| counter | -39.3% | -25.0% | -26.6% |
| cpu | -49.0% | -30.2% | -28.2% |
| full_adder | -29.7% | -19.4% | -16.6% |
| half_adder | -17.0% | -2.7% | -7.8% |
| lfsr | -43.8% | -28.0% | -21.6% |
| popcount | -38.1% | -21.2% | -22.8% |
| register_file | -37.6% | -22.7% | -20.7% |
| ripple_adder | -34.9% | -20.0% | -18.2% |
| riscv | -47.4% | -29.5% | -30.0% |
| traffic_light | -38.0% | -22.6% | -25.4% |

Across the 11 original examples, geometric-mean reductions are **37.0% area,
21.4% clock period, and 21.3% static cells** relative to the first round.
Relative to the original main branch, the reductions are **51.4%, 35.1%, and
24.2%**, respectively.

The RV32I demo now measures **19,703,806 × 18,935,080** cells,
**153,306,696 generations/clock**, **25,277,608 static live cells**, and
**413,182 components**.

These are measured heuristics, not a proof of optimality. All examples improve
all three metrics relative to the first round. The RISC-V example is new in this
PR, so it has no counterpart on the original main branch.

## Why the changes work

- **Paired slow turns.** The original class turn uses a duplicator (+37 wave
  generations), colour-changing reflector (+6), and spare-output eater: +43
  generations and 192 static live cells. The opposite duplicator orientation
  (+125) followed by a Snark (+4) gives +129 generations for 140 cells. A pair can
  replace six ordinary turns when six phase quanta are needed. The router chooses
  the shortest sequence meeting the exact phase budget. Slow turns occur in
  pairs, shifting the intermediate lane by one cell and restoring its parity at
  the second turn. Free-phase register connections can also use the cheaper pair.
- **Pitch 116 instead of 128.** Fragment outputs are handed over only after their
  reaction has released a free glider and after the complete output track clears
  the footprint. This fixes the earlier exit constraint and permits tighter
  spacing. It also selects a cheaper splitter template. Smaller pitches tested
  still produce backward-time component connections.
- **32-consumer constant banks.** Shorter duplicator trees reduce phase imbalance
  and its delay hardware. Each bank has a self-sustaining feedback register; the
  pattern still runs in ordinary Life without an external source of constants.
  The first round's return-lane corrections make multiple banks usable.
- **Schedule blocks by live-lane pressure.** Among ready blocks, prefer the fewest
  used outputs minus inputs, then the earliest wave phase. This frees rows sooner.
  A priority queue implements the deterministic order without repeatedly scanning
  the entire ready set.
- **Share inverted inputs.** An inversion needs a crossing and a constant tap,
  so reuse it through a splitter tree. Plain primary inputs retain independent
  tape lanes. Reverse the arbitrary equal-cost inhibit-polarity tie, which gave a
  better overall result in the combined experiments.
- **Keep short odd delays feasible.** Their duplicators need an exit on row 2 at
  the tighter pitch. Trying that row first avoids many obstructed candidates.
  HashLife accelerates candidate characterization; independent tile-engine tests
  check all delay lengths 14–40 and representative long delays through 500.

## Alternatives tested

The search included input-inversion costs 0–4, both polarity tie directions,
independent/shared input polarities, supply banks from 1 to 256 consumers,
multiple ready-block priorities, pitches from 100 to 128, and compact-delay
thresholds from 8 through effectively disabled. Combined sweeps checked their
interactions rather than selecting each knob in isolation.

Selected ablations appear below, with their per-design measurements in
[`round-two-variants.csv`](round-two-variants.csv). Percentages are geometric-mean
changes across the 11 original examples, relative to `round-one.csv`. These
measurements precede the final short-delay repair. Unless stated otherwise, the
last eight rows use the combined configuration and vary one setting.

| Experiment | Area | Clock period | Static live cells |
|---|---:|---:|---:|
| Pitch 116 only | -17.4% | -9.1% | -3.3% |
| Pitch, banks, placement and mapping | -24.8% | -14.1% | -5.7% |
| Pitch 116 and paired slow turns | -32.7% | -17.6% | -20.0% |
| Combined, before delay repair | -37.2% | -21.7% | -21.3% |
| Combined at pitch 128 | -24.3% | -14.1% | -18.0% |
| Delay threshold 32 | -37.1% | -21.5% | -21.1% |
| Compact delays disabled | -36.5% | -21.2% | -20.6% |
| Supply banks of 8 | -34.4% | -20.6% | -20.7% |
| Supply banks of 16 | -36.2% | -21.3% | -21.2% |
| Supply banks of 64 | -37.2% | -21.4% | -21.2% |
| Group-based placement | -32.5% | -17.4% | -21.3% |
| Pressure / critical-path placement | -35.7% | -20.7% | -21.3% |
| Share both input polarities | -28.5% | -16.7% | -13.2% |

- Removing compact delays still loses: slow turns reduce bend cost but do not
  replace the benefit of a long folded delay.
- Sharing plain inputs worsens all three metrics by adding splitter hardware.
- Critical-path, phase-only, group-based, and several pressure tie priorities
  sometimes help one circuit but lose to pressure/early-phase ordering overall.
- Smaller banks spend too much on feedback; larger banks increase balancing cost.
  Bank 32 is a practical default, not a per-design optimum.
- A Snark cannot simply replace the original colour-changing logic reflector:
  its wave-phase shift does not implement the required annihilation timing.

The next substantial gains likely need local register feedback, a different
placement/routing architecture, or new logic macros. Those require broader
physical characterization. The retained implementation is a good stopping point
for this PR: it gives sizeable improvements without adding a costly search to
normal compilation.

## Reproduction and validation

```sh
cargo run --release -p goldl --example qor -- examples/*.goldl
cargo test --release --workspace
cargo build --release -p goldl-cli
for file in examples/*.goldl; do
  target/release/goldl test "$file"
  cargo run --release -p goldl --example verify -- "$file" 4 23
done
cargo run --release -p goldl --example audit_routes -- examples/riscv.goldl
cargo run --release -p goldl --example audit_routes -- examples/riscv.goldl RV32I
cd web
npm run wasm
npm run check
npm run build
```

The turn regression exercises both directions and every even phase budget 2–32
in real Life, checking the output trajectory and restoration of all components.
The delay regression uses the tile engine independently of HashLife construction.
Full-design verification compares emitted cells with exact reconstruction at
half-cycle checkpoints. Route audits additionally check free-flight interference
with static components and return-loop crossings, including neighboring cycles;
they are not a proof of every possible simultaneous reaction.

All **60 active Rust tests** pass (two pre-existing diagnostic tests remain ignored),
as do all **14 source testbenches** across 12 examples. Every example passes four
full clocks of Life verification. The final RV32I demo reaches generation
**613,226,797**, with eight exact half-cycle snapshots of about 25.28 million cells.
The full 76-cycle Fibonacci program and ISA/trap tests are checked logically; the
full program was not evolved in Life.

The final demo route audit passes 822,939 candidate static interactions and 7,411
return legs. The external-memory core passes 1,298,529 candidate static interactions
and 9,550 return legs. That core compiles without a layout error to **648,032
components**, **39,990,460 static live cells**, and **251,953,856 generations/clock**.

WASM compilation, TypeScript checking, the production web build, and Chromium
checks pass. The picker has no title/count overlap or horizontal overflow at 1280,
768, and 390 px; search, dismissal, idle rendering, zoom, and stepping work. The
RV32I demo compiles and renders in the browser with the correct component count.
Observed compilation times were 2.35 seconds natively and 3.6 seconds in Chromium
on this environment; timings vary with machine load. The web build retains its
pre-existing transport/waveform reactivity warnings.
