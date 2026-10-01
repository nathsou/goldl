# GoLDL

A hardware description language whose compilation target is Conway's Game of Life:
logic gates become glider collisions and stable reflector/splitter circuitry, signals are
gliders, and a hierarchical simulator can jump to any generation and reconstruct the exact
cell state without stepping every generation. A web playground (editor + LSP, Life view,
schematic view) runs the Rust compiler and simulator as WebAssembly.

Status: planning. See [docs/PLAN.md](docs/PLAN.md).
