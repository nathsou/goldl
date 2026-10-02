# First round of Life circuit layout experiments

This report records commit `9533d7e`. See [the current results](README.md) for the second round.

Baseline: `d49dfae` on `main`. `baseline.csv` and `round-one.csv` were generated with
`cargo run --release -p goldl --example qor -- examples/*.goldl`. The RISC-V example
is new and has no baseline counterpart. `row-reuse.csv` records an intermediate
experiment, before the final mapper and large-design correctness fixes.

## Decision and results

Keep live-row reuse, an accurate primary-input inversion cost, and compact storage
of straight routing runs. Keep the existing 128-cell pitch and single constant
supply. These changes reuse existing characterized components and the existing
phase-class routing discipline without introducing a new physical logic family.

Changes below are relative to the baseline; negative is better. Area is the
axis-aligned core bounding-box width × height, period is Life generations per
simulated clock cycle, and cells are **quiescent live cells** in the components.
Input tapes and data-dependent moving gliders are excluded. Component count is
reported separately in the CSV and is not a substitute for live-cell count.

| Example | Area | Clock period | Static live cells |
|---|---:|---:|---:|
| alu | -35.5% | -27.3% | -6.1% |
| blinker | -11.0% | -10.0% | -7.0% |
| counter | -24.3% | -16.6% | +2.1% |
| cpu | -36.3% | -25.1% | -0.1% |
| full_adder | -8.8% | -9.2% | -6.2% |
| half_adder | +14.2% | -0.9% | -3.0% |
| lfsr | -27.0% | -17.7% | -2.0% |
| popcount | -14.9% | -18.1% | -3.5% |
| register_file | -45.5% | -30.3% | -13.6% |
| ripple_adder | -12.9% | -11.3% | +2.9% |
| traffic_light | -31.0% | -19.9% | -2.6% |

The register file improves all three metrics substantially. Glider-8 gains about
36% in area and 25% in clock period with nearly unchanged population. This is a
balanced heuristic, not a Pareto optimum for every input: the half adder's bounding
box grows about 14%, and counter/ripple-adder population grows about 2–3%. The
additional return clearances also cost space, but are required for physical
correctness on large register banks.

## Retained techniques

- **Reuse rows after their final consumer.** Give each block fresh columns, but
  place it in the first vertical interval clear of live lanes. Reserve register-D
  rows until the perimeter, and include the complete delay footprint, including
  its backward legs. Compaction shortens the perimeter that every register must
  traverse, so it reduces both dimensions and clock period without deleting gates.
- **Charge for inverted primary inputs.** Mapping previously treated a complemented
  input tape as free. Standalone exports actually synthesize its inversion using
  a constant stream and a crossing. Charging for that hardware changes polarity
  choices and removes redundant gates, especially in the ALU and register file.
- **Store straight flight as runs.** Empty routing distances are represented by
  one length rather than one vector entry per grid cell. This has no physical
  cost and prevents compiler memory from scaling with every empty square crossed.
- **Check static components in interacting clusters.** Individually characterized
  still lifes need joint simulation only where component bounding boxes are
  within two cells. A spatial index and union-find replace a full static universe.
  The implementation is tested against the original whole-universe check, including
  overlaps and negative coordinates. In a same-design native RV32I-demo comparison,
  peak RSS dropped from 2,348,944 KiB to 241,012 KiB (about 90%). Wall time on that
  run dropped from 9.55 s to 4.76 s; timings vary with machine load.

The larger example also exposed correctness limits that smaller examples did not:
long-delay outputs could exit through a footprint gap and hit their own components;
equal-time feedback padding could put neighboring return lanes on each other; and
a constant-zero register input could disappear before construction. Outputs now
clear the complete track, long delays are simulated past their farthest component,
return rows are ordered by launch phase and westbound lanes/wave phases are spaced,
and zero-valued consumed nets retain their routing ports. Backward-time path legs
are rejected. These are prerequisites for making the larger example usable.

## Other experiments

| Technique | Outcome |
|---|---|
| Pitch 64 / 96 instead of 128 | Rejected: splitter placement/exit clearance fails with the current component library. |
| Constant-supply banks of 1, 2, 4, 8, 16, 32, 64 consumers | Some population wins, but additional feedback paths interfered in Life verification. Retained one supply; any retry needs the complete physical regression suite with the new feedback router. |
| Close scheduling slack greedily | Replaced compact delays with many routing bends and worsened the CPU's population. |
| Exhaustively search splitter candidates | Produced the same best implementation as the existing first-fit search. |
| Multiplexer-based ripple carry | Helped population count, worsened CPU results; not retained globally. |

A future carry architecture or alternative gate library could make larger gains,
but would require new component characterization and area/timing models. The
retained changes are comparatively small, measurable, and compatible with exports
that run without an external Life harness.

## Playground usability

The example picker uses short names plus descriptions, a fixed-width count column,
wrapping text, a scrollable responsive panel, search and keyboard dismissal. Counts
come from successful compilations of the actual bundled source, so they do not
become stale when the compiler changes or an edited source is compiling.

The RV32I demo also exposed excessive GPU work at overview scale. The universe now
samples dense wire geometry below 0.002 pixels/cell (all lanes return when zoomed
in), and a paused canvas redraws only when its camera, data, selection, theme or
layers change. This makes the compiled example responsive even in headless Chromium
with software rendering; compilation, opening the picker again, and a full-page
screenshot all pass. A browser check verifies that paused rendering stops submitting
unchanged geometry and that zoom/clock changes repaint.

## RISC-V and validation

The new [RV32I example](../RISCV.md) implements all base integer instructions,
32 registers, loads/stores, fetch/data stalls and explicit traps. Its bundled ROM
runs Fibonacci through ECALL. The default demo measures 27,441,156 × 25,827,199
cells, 217,408,688 generations per clock, 544,902 components, and 36,094,497 static
live cells. It is a large example; the architectural simulator is much faster
than cell-level verification.

Validation commands:

```sh
cargo test --release --workspace
for file in examples/*.goldl; do
  cargo run --release -p goldl-cli -- test "$file"
  cargo run --release -p goldl --example verify -- "$file" 4
done
cargo run --release -p goldl --example audit_routes -- examples/riscv.goldl
cargo run --release -p goldl-cli -- stats examples/riscv.goldl --top RV32I
cd web
npm run wasm
npm run check
npm run build
```

Architectural tests compare RTL, AIG and GNL on every tested cycle. Physical
checks compare emitted Life cells with the exact reconstruction at half-cycle
checkpoints. The focused route audit additionally exercises potential free-flight
collisions independently of whether those signals are active in the demo.
The UI is checked in Chromium at 1280, 768 and 390 px: long titles/counts do not
overlap or overflow, and search, empty results and keyboard dismissal work.

Final results: all 59 active Rust tests pass (two existing diagnostic tests remain
ignored), all 14 source testbenches across 12 examples pass, and every example passes
four full clock cycles of cell-by-cell Life verification. For RV32I this reaches
generation **869,634,765**, checking eight half-cycle snapshots of roughly 36 million
live cells. Its full 76-cycle Fibonacci program and the individual instruction/trap
cases are checked logically; the full 76 cycles were not evolved in Life.

The route audit passes 1,110,152 candidate static interactions and 6,635 return legs
for the demo, and 1,603,280 candidate static interactions and 8,167 return legs for
the externally driven core. `RV32I` also compiles with no layout error (786,456
components, 51,999,616 static live cells, period 325,940,344). WASM compilation,
TypeScript checking, the Vite production build, and the Chromium checks pass.
The build still reports the pre-existing Svelte reactivity warnings in the transport
and waveform views.
