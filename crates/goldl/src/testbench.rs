//! Test benches: `test "name" for Module { a = 1; step; assert y == 2 }`.
//!
//! The module under test is elaborated on its own and driven by the RTL simulator. Pokes
//! set inputs, `step` advances the clock, and `assert` checks an expression over the
//! module's ports (evaluated combinationally with the current inputs and state).

use crate::elab::{elaborate, find_top};
use crate::rtl::{mask, RtlSim};
use crate::syntax::ast::{BinOp, Expr, ExprKind, Item, TestStmt, UnOp};
use crate::syntax::Span;
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct TestResult {
    pub name: String,
    pub span: Span,
    pub passed: bool,
    /// Failure message and location.
    pub failure: Option<(Span, String)>,
    pub cycles: u64,
}

/// Run every test bench in `src`.
pub fn run_tests(src: &str) -> Vec<TestResult> {
    let (file, _) = crate::syntax::parser::parse(src);
    let mut out = Vec::new();
    for it in &file.items {
        let Item::Test(t) = it else { continue };
        let fail = |span: Span, msg: String, cycles: u64| TestResult {
            name: t.name.clone(),
            span: t.span,
            passed: false,
            failure: Some((span, msg)),
            cycles,
        };
        if find_top(&file, Some(&t.module.name)).is_none() {
            out.push(fail(
                t.module.span,
                format!("unknown module `{}`", t.module.name),
                0,
            ));
            continue;
        }
        let rtl = match elaborate(src, Some(&t.module.name)) {
            Ok((r, _)) => r,
            Err(an) => {
                let msg = an
                    .diags
                    .first()
                    .map_or("elaboration failed".to_string(), |d| d.message.clone());
                out.push(fail(t.module.span, msg, 0));
                continue;
            }
        };
        let mut sim = RtlSim::new(&rtl);
        let mut inputs: Vec<u64> = vec![0; rtl.inputs.len()];
        let mut result = None;
        'body: for st in &t.body {
            match st {
                TestStmt::Poke { port, value, span } => {
                    let Some(p) = rtl.inputs.iter().position(|x| x.name == port.name) else {
                        result = Some(fail(
                            port.span,
                            format!("`{}` is not an input of `{}`", port.name, t.module.name),
                            sim.cycle,
                        ));
                        break 'body;
                    };
                    match eval(value, &HashMap::new()) {
                        Ok(v) => inputs[p] = (v as u64) & mask(rtl.inputs[p].width),
                        Err(e) => {
                            result = Some(fail(*span, e, sim.cycle));
                            break 'body;
                        }
                    }
                }
                TestStmt::Step { count, span } => {
                    let n = match count {
                        None => 1,
                        Some(e) => match eval(e, &HashMap::new()) {
                            Ok(v) => v.max(0) as u64,
                            Err(e) => {
                                result = Some(fail(*span, e, sim.cycle));
                                break 'body;
                            }
                        },
                    };
                    for _ in 0..n {
                        sim.step(&rtl, &inputs);
                    }
                }
                TestStmt::Assert { cond, span } => {
                    sim.eval(&rtl, &inputs);
                    let mut env: HashMap<String, i128> = HashMap::new();
                    for (k, p) in rtl.inputs.iter().enumerate() {
                        env.insert(p.name.clone(), inputs[k] as i128);
                    }
                    for (p, o) in &rtl.outputs {
                        env.insert(p.name.clone(), sim.vals[*o as usize] as i128);
                    }
                    match eval(cond, &env) {
                        Ok(0) => {
                            let mut vals: Vec<String> = Vec::new();
                            collect_names(cond, &mut |n| {
                                if let Some(v) = env.get(n) {
                                    let s = format!("{n} = {v}");
                                    if !vals.contains(&s) {
                                        vals.push(s);
                                    }
                                }
                            });
                            let src_text = src
                                .get(cond.span.start as usize..cond.span.end as usize)
                                .unwrap_or("");
                            result = Some(fail(
                                *span,
                                format!(
                                    "assertion `{src_text}` failed in cycle {} ({})",
                                    sim.cycle,
                                    vals.join(", ")
                                ),
                                sim.cycle,
                            ));
                            break 'body;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            result = Some(fail(*span, e, sim.cycle));
                            break 'body;
                        }
                    }
                }
            }
        }
        out.push(result.unwrap_or(TestResult {
            name: t.name.clone(),
            span: t.span,
            passed: true,
            failure: None,
            cycles: sim.cycle,
        }));
    }
    out
}

fn collect_names(e: &Expr, f: &mut dyn FnMut(&str)) {
    match &e.kind {
        ExprKind::Ident(i) => f(&i.name),
        ExprKind::Unary(_, a) | ExprKind::Paren(a) => collect_names(a, f),
        ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
            collect_names(a, f);
            collect_names(b, f);
        }
        ExprKind::Slice(a, _, _) => collect_names(a, f),
        _ => {}
    }
}

/// Integer interpreter for test expressions (port values are unsigned).
fn eval(e: &Expr, env: &HashMap<String, i128>) -> Result<i128, String> {
    Ok(match &e.kind {
        ExprKind::Int(v) => *v as i128,
        ExprKind::Bool(b) => *b as i128,
        ExprKind::Paren(a) => eval(a, env)?,
        ExprKind::Ident(i) => *env
            .get(&i.name)
            .ok_or_else(|| format!("unknown name `{}` in test", i.name))?,
        ExprKind::Unary(op, a) => {
            let v = eval(a, env)?;
            match op {
                UnOp::LNot => (v == 0) as i128,
                UnOp::Neg => -v,
                UnOp::Not => !v,
            }
        }
        ExprKind::Binary(op, a, b) => {
            let (x, y) = (eval(a, env)?, eval(b, env)?);
            match op {
                BinOp::Add => x + y,
                BinOp::Sub => x - y,
                BinOp::Mul => x * y,
                BinOp::Div => x.checked_div(y).ok_or("division by zero")?,
                BinOp::Rem => x.checked_rem(y).ok_or("division by zero")?,
                BinOp::Shl => x << (y.clamp(0, 120) as u32),
                BinOp::Shr | BinOp::Sar => x >> (y.clamp(0, 120) as u32),
                BinOp::And => x & y,
                BinOp::Or => x | y,
                BinOp::Xor => x ^ y,
                BinOp::Eq => (x == y) as i128,
                BinOp::Ne => (x != y) as i128,
                BinOp::Lt => (x < y) as i128,
                BinOp::Le => (x <= y) as i128,
                BinOp::Gt => (x > y) as i128,
                BinOp::Ge => (x >= y) as i128,
                BinOp::LAnd => (x != 0 && y != 0) as i128,
                BinOp::LOr => (x != 0 || y != 0) as i128,
                BinOp::Concat => return Err("`++` is not supported in tests".into()),
            }
        }
        ExprKind::Index(a, i) => (eval(a, env)? >> (eval(i, env)?.clamp(0, 120) as u32)) & 1,
        ExprKind::Slice(a, hi, lo) => {
            let (v, h, l) = (eval(a, env)?, eval(hi, env)?, eval(lo, env)?);
            if h < l {
                return Err("empty slice".into());
            }
            (v >> (l as u32)) & ((1i128 << ((h - l + 1) as u32)) - 1)
        }
        _ => return Err("unsupported expression in test".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_tests() {
        let src = r#"
module C(en: bit) -> (q: bits<4>) {
  reg r: bits<4> = 0
  r.next = if en { r + 1 } else { r }
  q = r
}
test "counts" for C {
  en = 1
  step(3)
  assert q == 3
  en = 0
  step
  assert q == 3
}
test "fails" for C {
  en = 1
  step
  assert q == 5
}
"#;
        let r = run_tests(src);
        assert_eq!(r.len(), 2);
        assert!(r[0].passed, "{:?}", r[0]);
        assert!(!r[1].passed);
        assert!(
            r[1].failure.as_ref().unwrap().1.contains("q = 1"),
            "{:?}",
            r[1]
        );
    }
}
