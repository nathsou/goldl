//! GoLDL compiler: HDL → glider logic → Game of Life patterns, plus the hierarchical simulator.

pub mod aig;
pub mod asm;
pub mod driver;
pub mod elab;
pub mod gnl;
pub mod json;
pub mod layout;
pub mod lsp;
pub mod map;
pub mod phys;
pub mod rtl;
pub mod schematic;
pub mod session;
pub mod sim;
pub mod syntax;
pub mod tech;
pub mod testbench;

mod static_drc;
