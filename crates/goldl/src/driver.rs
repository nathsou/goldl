//! Compilation pipeline: source → RTL → AIG → glider netlist → Life layout.

use crate::aig::{bitblast, Aig};
use crate::elab::{elaborate, Analysis};
use crate::gnl::{Gnl, GnlStats};
use crate::layout::{layout, LayoutResult};
use crate::rtl::Rtl;
use crate::syntax::Diag;

pub struct Compiled {
    pub rtl: Rtl,
    pub aig: Aig,
    pub gnl: Gnl,
    pub layout: Option<LayoutResult>,
    pub analysis: Analysis,
    pub stats: Stats,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub rtl_nodes: usize,
    pub regs: usize,
    pub reg_bits: usize,
    pub aig_ands: usize,
    pub gnl: GnlStats,
    pub components: usize,
    pub period: i64,
    pub width: i64,
    pub height: i64,
    pub cells: usize,
    pub layout_error: Option<String>,
}

pub struct Options {
    pub top: Option<String>,
    pub layout: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { top: None, layout: true }
    }
}

pub fn compile(src: &str, opts: &Options) -> Result<Compiled, (Vec<Diag>, Analysis)> {
    let (rtl, analysis) = match elaborate(src, opts.top.as_deref()) {
        Ok(x) => x,
        Err(an) => return Err((an.diags.clone(), an)),
    };
    let aig = bitblast(&rtl);
    let mut gnl = crate::map::map(&aig, &rtl);
    let mut stats = Stats {
        rtl_nodes: rtl.live_order().len(),
        regs: rtl.regs.len(),
        reg_bits: rtl.regs.iter().map(|r| r.width as usize).sum(),
        aig_ands: aig.n_ands(),
        gnl: gnl.stats(),
        ..Default::default()
    };
    let lay = if opts.layout {
        match layout(&gnl) {
            Ok((gb, l)) => {
                gnl = gb;
                stats.gnl = gnl.stats();
                let p = &l.phys;
                stats.components = p.insts.len();
                stats.period = p.period;
                stats.width = p.bbox.2 - p.bbox.0 + 1;
                stats.height = p.bbox.3 - p.bbox.1 + 1;
                stats.cells = p.insts.iter().map(|i| i.oriented().cells.len()).sum();
                Some(l)
            }
            Err(e) => {
                stats.layout_error = Some(e);
                None
            }
        }
    } else {
        None
    };
    Ok(Compiled { rtl, aig, gnl, layout: lay, analysis, stats })
}
