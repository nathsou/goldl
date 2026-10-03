# RV32I example

[`examples/riscv.goldl`](../examples/riscv.goldl) contains a complete RV32I base
integer core and a self-running Fibonacci system. Select **RISC-V RV32I** in the
playground, or run:

```sh
cargo run --release -p goldl-cli -- test examples/riscv.goldl
cargo run --release -p goldl-cli -- run examples/riscv.goldl 80
cargo run --release -p goldl-cli -- stats examples/riscv.goldl --top RV32I
cargo run --release -p goldl-cli -- build examples/riscv.goldl -o riscv.rle
```

The default top module, `RiscVDemo`, includes a 16-word instruction ROM and a
memory-mapped output word at address zero. It writes twelve Fibonacci values
`0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89`, then halts on ECALL. Its outputs are
`out`, `valid`, `halted`, `pc`, and `cause`. `valid` marks a new output value for
one clock cycle. The two source testbenches check the program and x0 behavior.

## Instruction set

The core implements all 40 RV32I base instructions:

| Family | Instructions |
|---|---|
| Upper immediates | LUI, AUIPC |
| Jumps | JAL, JALR |
| Branches | BEQ, BNE, BLT, BGE, BLTU, BGEU |
| Loads | LB, LH, LW, LBU, LHU |
| Stores | SB, SH, SW |
| Immediate ALU | ADDI, SLTI, SLTIU, XORI, ORI, ANDI, SLLI, SRLI, SRAI |
| Register ALU | ADD, SUB, SLL, SLT, SLTU, XOR, SRL, SRA, OR, AND |
| Ordering / environment | FENCE, ECALL, EBREAK |

There are 32 32-bit integer registers; reads of x0 return zero and writes to x0
are ignored. Arithmetic wraps modulo 2³². Instructions are 32-bit aligned and
memory is little endian. FENCE completes immediately because this core has no
outstanding transactions or buffered writes. Reserved instruction encodings trap;
RV32I HINT encodings execute with their specified lack of architectural effects.

RISC-V extensions are separate: this example does not implement multiplication,
atomics, compressed instructions, floating point, Zicsr, or Zifencei. It exposes
traps to its execution environment instead of implementing privileged CSRs,
interrupt delivery, or a trap-handler privilege mode.

## Connecting the core

Instantiate `RV32I<0>` (also the default generic argument), or choose `--top RV32I`.
One clock edge retires an instruction when its fetch and any data access are ready.
Keep the instruction and its response stable until `retired` or a trap; the core
holds its PC and registers during a stall.

| Signal | Meaning |
|---|---|
| `reset`, `enable` | Reset PC and trap state; pause requests and state when disabled |
| `pc`, `fetch_valid` | Instruction address and active fetch request |
| `instr`, `instr_ready`, `instr_fault` | Fetched instruction, acknowledgement, access fault |
| `mem_valid`, `mem_write`, `mem_addr` | Data request, write flag, **byte** address |
| `rdata`, `mem_ready`, `mem_fault` | Aligned 32-bit read word, acknowledgement, access fault |
| `wdata`, `wstrb` | Write data and four byte enables, positioned within the aligned word |
| `retired` | The current instruction commits at this edge |
| `wb_valid`, `wb_rd`, `wb_data` | Register writeback at this edge; x0 writes are suppressed |
| `trap`, `cause`, `tval`, `epc` | Latched exception and faulting instruction PC |
| `next_pc` | Combinational candidate next PC; only used when the instruction retires |

A store takes effect only on `mem_valid && mem_write && mem_ready && !mem_fault`.
Align `mem_addr` down to four bytes when indexing a word-addressed memory. Byte
and halfword load selection and extension are performed inside the core. Faults
are sampled only when the corresponding ready signal is asserted.

Reset has priority over enable. It clears PC and trap state; integer register
contents are architecturally unspecified after reset, except x0. GoLDL starts
uninitialized memories at zero, but software must not depend on that after a
subsequent reset. `out` and `out_valid` belong to the demo interface.

## Traps

Traps suppress retirement, register writes and memory side effects. The PC stays
at the faulting instruction and the core halts until reset. Misaligned untaken
branches do not trap, and JALR clears target bit zero before checking alignment.

| Cause | Meaning | `tval` |
|---:|---|---|
| 0 | Instruction address misaligned | Bad target/address |
| 1 | Instruction access fault | Fetch PC |
| 2 | Illegal instruction | Instruction bits |
| 3 | Breakpoint (EBREAK) | Faulting PC |
| 4 / 6 | Load / store address misaligned | Data byte address |
| 5 / 7 | Load / store access fault | Data byte address |
| 11 | ECALL in the machine execution environment | Zero |

## Validation

`cargo test --release -p goldl --test riscv` checks every instruction family,
all registers, signed boundaries, shift masking, immediate and jump boundaries,
byte lanes, stalls, reset, illegal encodings and precise traps. Every tested
cycle also compares the RTL outputs with the AIG and glider netlist. The source
testbenches execute the bundled Fibonacci program through its final ECALL.

Physical Life verification is separate and much more expensive for this example:

```sh
cargo run --release -p goldl --example verify -- examples/riscv.goldl 4
cargo run --release -p goldl --example audit_routes -- examples/riscv.goldl
```

The route audit checks free flights against static components and return-flight
pairs, including adjacent cycles. Full verification evolves the emitted cells
with HashLife and compares them against the reconstruction at half-cycle
checkpoints. See [the experiment report](experiments/README.md) for measured size,
timing and validation results.
