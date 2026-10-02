//! WebAssembly interface of the GoLDL toolchain (raw C ABI, no bindings generator).
//!
//! Requests and responses are JSON strings in linear memory:
//! `goldl_alloc(n)` → pointer for the request, `goldl_call(ptr, len)` → pointer to a result
//! buffer `[u32 len][bytes]` (valid until the next call). Rendering queries that run every
//! frame use binary results instead (`goldl_cells`, `goldl_gliders`, ...), returned in a
//! reusable buffer whose first word is the element count.

use goldl::json::Json;
use goldl::lsp::Server;
use goldl::schematic;
use goldl::session::Session;
use goldl::sim::Rect;
use goldl::syntax::{LineIndex, Severity};
use std::cell::RefCell;

struct State {
    lsp: Server,
    session: Option<Session>,
    out: Vec<u8>,
    bin_i32: Vec<i32>,
    bin_f64: Vec<f64>,
    panic: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State { lsp: Server::new(), session: None, out: Vec::new(), bin_i32: Vec::new(), bin_f64: Vec::new(), panic: String::new() });
}

#[no_mangle]
pub extern "C" fn goldl_alloc(n: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(n.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// # Safety
/// `p` must come from `goldl_alloc(n)`.
#[no_mangle]
pub unsafe extern "C" fn goldl_free(p: *mut u8, n: usize) {
    drop(Vec::from_raw_parts(p, 0, n.max(1)));
}

#[no_mangle]
pub extern "C" fn goldl_init() {
    std::panic::set_hook(Box::new(|info| {
        let msg = info.to_string();
        // try_with: the state may be borrowed while panicking.
        let _ = STATE.try_with(|s| {
            if let Ok(mut s) = s.try_borrow_mut() {
                s.panic = msg.clone();
            }
        });
        PANIC_MSG.with(|p| *p.borrow_mut() = msg);
    }));
}

thread_local! {
    static PANIC_MSG: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Message of the last panic (after a trap), as a result buffer.
#[no_mangle]
pub extern "C" fn goldl_last_panic() -> *const u8 {
    let msg = PANIC_MSG.with(|p| p.borrow().clone());
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        write_out(&mut s.out, msg.as_bytes())
    })
}

fn write_out(out: &mut Vec<u8>, bytes: &[u8]) -> *const u8 {
    out.clear();
    out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(bytes);
    out.as_ptr()
}

/// # Safety
/// `p` must point to `n` readable bytes of UTF-8.
#[no_mangle]
pub unsafe extern "C" fn goldl_call(p: *const u8, n: usize) -> *const u8 {
    let req = std::str::from_utf8(std::slice::from_raw_parts(p, n)).unwrap_or("");
    let resp = match Json::parse(req) {
        Ok(j) => STATE.with(|s| handle(&mut s.borrow_mut(), &j)),
        Err(e) => Json::obj().with("error", format!("bad request: {e}")),
    };
    let text = resp.to_string();
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        write_out(&mut s.out, text.as_bytes())
    })
}

fn diags_json(src: &str, diags: &[goldl::syntax::Diag]) -> Json {
    let li = LineIndex::new(src);
    Json::Arr(
        diags
            .iter()
            .map(|d| {
                let (l0, c0) = li.line_col(d.span.start);
                let (l1, c1) = li.line_col(d.span.end);
                Json::obj()
                    .with("message", d.message.clone())
                    .with("severity", match d.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                        Severity::Info => "info",
                    })
                    .with("from", vec![l0, c0])
                    .with("to", vec![l1, c1])
                    .with("span", vec![d.span.start, d.span.end])
            })
            .collect(),
    )
}

fn handle(st: &mut State, j: &Json) -> Json {
    let cmd = j.get("cmd").as_str().unwrap_or("");
    match cmd {
        "lsp" => Json::Arr(st.lsp.handle(j.get("msg"))),
        "compile" => {
            let src = j.get("src").as_str().unwrap_or("");
            let top = j.get("top").as_str();
            match Session::new(src, top) {
                Err(d) => {
                    st.session = None;
                    Json::obj().with("ok", false).with("diags", diags_json(src, &d))
                }
                Ok(s) => {
                    let r = describe(&s);
                    st.session = Some(s);
                    r
                }
            }
        }
        "tests" => {
            let src = j.get("src").as_str().unwrap_or("");
            let li = LineIndex::new(src);
            let v: Vec<Json> = goldl::testbench::run_tests(src)
                .into_iter()
                .map(|t| {
                    let mut o = Json::obj().with("name", t.name).with("passed", t.passed).with("cycles", t.cycles).with("line", li.line_col(t.span.start).0);
                    if let Some((sp, m)) = t.failure {
                        o.set("message", m);
                        o.set("failLine", li.line_col(sp.start).0);
                    }
                    o
                })
                .collect();
            Json::Arr(v)
        }
        _ => {
            let Some(s) = st.session.as_mut() else { return Json::obj().with("error", "no design compiled") };
            match cmd {
                "set_input" => {
                    s.set_input(j.get("port").as_i64().unwrap_or(0) as usize, j.get("value").as_f64().unwrap_or(0.0) as u64, j.get("from").as_i64().unwrap_or(0) as usize);
                    Json::obj().with("ok", true)
                }
                "trace" => {
                    // Values per cycle of the requested RTL nodes (strings: values may exceed 2^53).
                    let from = j.get("from").as_i64().unwrap_or(0).max(0) as usize;
                    let to = j.get("to").as_i64().unwrap_or(from as i64 + 1).max(from as i64) as usize;
                    let rids: Vec<u32> = j.get("rids").as_arr().iter().filter_map(|x| x.as_i64()).map(|x| x as u32).collect();
                    s.ensure(to);
                    let mut rows = Vec::new();
                    for c in from..to {
                        let ins = s.inputs_at(c);
                        let outs = s.outputs(c);
                        let regs = s.regs(c);
                        let vals: Vec<Json> = rids.iter().map(|&r| Json::from(s.rtl_value(r, c).to_string())).collect();
                        let f = |v: Vec<u64>| Json::Arr(v.into_iter().map(|x| Json::from(x.to_string())).collect());
                        rows.push(Json::obj().with("inputs", f(ins)).with("outputs", f(outs)).with("regs", f(regs)).with("values", vals));
                    }
                    Json::Arr(rows)
                }
                "rle" => {
                    let g = j.get("gen").as_f64().unwrap_or(0.0) as i64;
                    let cells = s.cells(g, Rect::ALL);
                    let p = goldl_life::Pattern::from_cells(cells);
                    Json::obj().with("rle", format!("#N goldl generation {g}\n#C Generated by the GoLDL compiler\n{}", p.to_rle()))
                }
                "verify" => {
                    // Run the real Life rules with HashLife and compare with the reconstruction.
                    let g = j.get("gen").as_f64().unwrap_or(0.0) as i64;
                    let p0 = goldl_life::Pattern::from_cells(s.cells(0, Rect::ALL));
                    let mut h = goldl_life::hashlife::HashLife::from_pattern(&p0);
                    h.step(g.max(0) as u64);
                    let real = h.to_pattern();
                    let model = goldl_life::Pattern::from_cells(s.cells(g, Rect::ALL));
                    let rs = real.to_set();
                    let ms = model.to_set();
                    let diff = rs.symmetric_difference(&ms).count();
                    Json::obj().with("gen", g).with("cells", real.len()).with("mismatches", diff).with("ok", diff == 0)
                }
                _ => Json::obj().with("error", format!("unknown command `{cmd}`")),
            }
        }
    }
}

/// Everything the playground needs to know about a freshly compiled design.
fn describe(s: &Session) -> Json {
    let c = &s.c;
    let st = &c.stats;
    let ports = |v: &[goldl::gnl::PortInfo]| Json::Arr(v.iter().map(|p| Json::obj().with("name", p.name.clone()).with("width", p.width)).collect());
    let outs: Vec<goldl::gnl::PortInfo> = c.rtl.outputs.iter().map(|(p, _)| p.clone()).collect();
    let regs: Vec<Json> = c.rtl.regs.iter().map(|r| Json::obj().with("name", r.name.clone()).with("width", r.width).with("group", r.group).with("init", r.init.to_string())).collect();
    let probes: Vec<Json> = c.rtl.probes.iter().map(|(n, rid, g)| Json::obj().with("name", n.clone()).with("rid", *rid).with("group", *g).with("width", c.rtl.nodes[*rid as usize].width)).collect();
    let groups: Vec<Json> = c.gnl.groups.iter().map(|g| Json::obj().with("name", g.name.clone()).with("kind", g.kind.clone()).with("parent", g.parent.map(|p| p as i64))).collect();
    let regions: Vec<Json> = s
        .regions()
        .into_iter()
        .map(|(g, runs)| Json::obj().with("group", g).with("runs", Json::Arr(runs.into_iter().map(|r| Json::from(vec![r.0, r.1, r.2, r.3])).collect())))
        .collect();
    let stats = Json::obj()
        .with("rtlNodes", st.rtl_nodes)
        .with("regBits", st.reg_bits)
        .with("aigAnds", st.aig_ands)
        .with("crossings", st.gnl.cross)
        .with("splitters", st.gnl.split)
        .with("delays", st.gnl.delay)
        .with("nets", st.gnl.nets)
        .with("components", st.components)
        .with("period", st.period)
        .with("width", st.width)
        .with("height", st.height)
        .with("cells", st.cells)
        .with("layoutError", st.layout_error.clone());
    let mut o = Json::obj()
        .with("ok", true)
        .with("name", c.rtl.name.clone())
        .with("diags", diags_json(&s.src, &c.analysis.diags))
        .with("inputs", ports(&c.rtl.inputs))
        .with("outputs", ports(&outs))
        .with("regs", regs)
        .with("probes", probes)
        .with("groups", groups)
        .with("regions", regions)
        .with("stats", stats)
        .with("schematic", schematic::build(&c.rtl).to_json(&c.rtl));
    if let Some(ph) = s.phys() {
        o.set("period", ph.period);
        o.set("bbox", vec![ph.bbox.0, ph.bbox.1, ph.bbox.2, ph.bbox.3]);
        o.set("grid", ph.grid);
        let kinds = ["snark", "cc", "dup", "eater"];
        o.set("componentKinds", kinds.to_vec());
    }
    o
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect { x0: x0.floor() as i64, y0: y0.floor() as i64, x1: x1.ceil() as i64, y1: y1.ceil() as i64 }
}

/// Live cells in a rectangle at generation `g`: `[n, x0, y0, x1, y1, ...]` (i32), at most
/// `max` cells (n = -1 when truncated).
#[no_mangle]
pub extern "C" fn goldl_cells(g: f64, x0: f64, y0: f64, x1: f64, y1: f64, max: u32) -> *const i32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = &mut *s;
        st.bin_i32.clear();
        st.bin_i32.push(0);
        if let Some(sess) = st.session.as_mut() {
            let cells = sess.cells(g as i64, rect(x0, y0, x1, y1));
            if cells.len() > max as usize {
                st.bin_i32[0] = -1;
            } else {
                st.bin_i32[0] = cells.len() as i32;
                for (x, y) in cells {
                    st.bin_i32.push(x as i32);
                    st.bin_i32.push(y as i32);
                }
            }
        }
        st.bin_i32.as_ptr()
    })
}

/// Gliders in flight: `[n, x, y, dir, sig, ...]` (f64; dir = (dx+1) + 3(dy+1)).
#[no_mangle]
pub extern "C" fn goldl_gliders(g: f64, x0: f64, y0: f64, x1: f64, y1: f64, max: u32) -> *const f64 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = &mut *s;
        st.bin_f64.clear();
        st.bin_f64.push(0.0);
        if let Some(sess) = st.session.as_mut() {
            let gl = sess.gliders(g as i64, rect(x0, y0, x1, y1));
            let n = gl.len().min(max as usize);
            st.bin_f64[0] = n as f64;
            for p in gl.into_iter().take(n) {
                st.bin_f64.extend([p.x, p.y, ((p.dx + 1) + 3 * (p.dy + 1)) as f64, p.sig as f64]);
            }
        }
        st.bin_f64.as_ptr()
    })
}

/// Components: `[n, x0, y0, x1, y1, kind, group, ...]` (i32).
#[no_mangle]
pub extern "C" fn goldl_components() -> *const i32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = &mut *s;
        st.bin_i32.clear();
        st.bin_i32.push(0);
        if let Some(ph) = st.session.as_ref().and_then(|x| x.phys()) {
            st.bin_i32[0] = ph.insts.len() as i32;
            for i in &ph.insts {
                let b = i.bbox();
                let kind = match i.oriented().kind {
                    goldl::tech::component::Kind::Snark => 0,
                    goldl::tech::component::Kind::Cc => 1,
                    goldl::tech::component::Kind::Dup => 2,
                    goldl::tech::component::Kind::Eater => 3,
                };
                st.bin_i32.extend([b.0 as i32, b.1 as i32, b.2 as i32, b.3 as i32, kind, i.group as i32]);
            }
        }
        st.bin_i32.as_ptr()
    })
}

/// Components reacting around generation `g` inside the rectangle: `[n, index, ...]`.
#[no_mangle]
pub extern "C" fn goldl_active(g: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> *const i32 {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let st = &mut *s;
        st.bin_i32.clear();
        st.bin_i32.push(0);
        if let Some(sess) = st.session.as_mut() {
            let a = sess.active(g as i64, rect(x0, y0, x1, y1));
            st.bin_i32[0] = a.len() as i32;
            st.bin_i32.extend(a.into_iter().map(|x| x as i32));
        }
        st.bin_i32.as_ptr()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(req: &str) -> Json {
        let r = STATE.with(|s| handle(&mut s.borrow_mut(), &Json::parse(req).unwrap()));
        Json::parse(&r.to_string()).unwrap()
    }

    #[test]
    fn compile_and_query() {
        let src = "module C(en: bit) -> (q: bits<2>) {\\n  reg r: bits<2> = 0\\n  r.next = if en { r + 1 } else { r }\\n  q = r\\n}\\n";
        let r = call(&format!(r#"{{"cmd":"compile","src":"{src}"}}"#));
        assert_eq!(r.get("ok").as_bool(), Some(true), "{r}");
        assert!(r.get("period").as_i64().unwrap() > 0);
        call(r#"{"cmd":"set_input","port":0,"value":1,"from":0}"#);
        let t = call(r#"{"cmd":"trace","from":0,"to":4,"rids":[]}"#);
        assert_eq!(t.idx(3).get("outputs").idx(0).as_str(), Some("3"));
        let p = goldl_cells(100.0, -1e7, -1e7, 1e7, 1e7, 1_000_000);
        let n = unsafe { *p };
        assert!(n > 100);
    }
}
