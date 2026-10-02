//! `goldl` command line: check, build, simulate, test and serve GoLDL designs.

use goldl::driver::{compile, Options};
use goldl::json::Json;
use goldl::rtl::RtlSim;
use goldl::session::Session;
use goldl::sim::Rect;
use goldl::syntax::{Diag, LineIndex, Severity};
use std::io::{BufRead, Read, Write};
use std::process::exit;

const USAGE: &str = "usage: goldl <command> [args]

commands:
  check FILE                 parse and type-check, print diagnostics
  build FILE [-o OUT.rle] [--top NAME] [--gen G]
                             compile to a Life pattern (RLE) at generation G (default 0)
  stats FILE [--top NAME]    print compilation statistics
  run FILE [CYCLES] [--top NAME] [--set PORT=VALUE ...]
                             simulate (RTL) and print outputs per cycle
  test FILE                  run the test benches in FILE
  verify FILE [CYCLES]       check the Life pattern against the logic simulation with HashLife
  lsp                        start the language server on stdin/stdout";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        eprintln!("{USAGE}");
        exit(2);
    };
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let read = |i: usize| -> (String, String) {
        let path = args.get(i).cloned().unwrap_or_else(|| {
            eprintln!("{USAGE}");
            exit(2)
        });
        let src = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            eprintln!("error: cannot read {path}: {e}");
            exit(1)
        });
        (path, src)
    };
    match cmd.as_str() {
        "check" => {
            let (path, src) = read(1);
            let an = goldl::elab::analyze(&src);
            print_diags(&path, &src, &an.diags);
            if an.diags.iter().any(|d| d.severity == Severity::Error) {
                exit(1);
            }
        }
        "stats" => {
            let (path, src) = read(1);
            match compile(&src, &Options { top: flag("--top"), layout: true }) {
                Ok(c) => println!("{:#?}", c.stats),
                Err((d, _)) => {
                    print_diags(&path, &src, &d);
                    exit(1)
                }
            }
        }
        "build" => {
            let (path, src) = read(1);
            let mut s = Session::new(&src, flag("--top").as_deref()).unwrap_or_else(|d| {
                print_diags(&path, &src, &d);
                exit(1)
            });
            if let Some(e) = &s.c.stats.layout_error {
                eprintln!("error: layout failed: {e}");
                exit(1);
            }
            let g: i64 = flag("--gen").map_or(0, |v| v.parse().unwrap_or(0));
            let p = goldl_life::Pattern::from_cells(s.cells(g, Rect::ALL));
            let rle = format!("#N {}\n#C GoLDL design `{}`, generation {g}, clock period {}\n{}", path, s.c.rtl.name, s.period(), p.to_rle());
            match flag("-o") {
                Some(out) => {
                    std::fs::write(&out, rle).unwrap_or_else(|e| {
                        eprintln!("error: cannot write {out}: {e}");
                        exit(1)
                    });
                    eprintln!("wrote {out}: {} cells, period {}", p.len(), s.period());
                }
                None => print!("{rle}"),
            }
        }
        "run" => {
            let (path, src) = read(1);
            let cycles: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(16);
            let (rtl, _) = goldl::elab::elaborate(&src, flag("--top").as_deref()).unwrap_or_else(|an| {
                print_diags(&path, &src, &an.diags);
                exit(1)
            });
            let mut ins = vec![0u64; rtl.inputs.len()];
            for (i, a) in args.iter().enumerate() {
                if a == "--set" {
                    if let Some((k, v)) = args.get(i + 1).and_then(|s| s.split_once('=')) {
                        if let Some(p) = rtl.inputs.iter().position(|x| x.name == k) {
                            ins[p] = parse_int(v);
                        }
                    }
                }
            }
            let mut sim = RtlSim::new(&rtl);
            for c in 0..cycles {
                let o = sim.step(&rtl, &ins);
                let cols: Vec<String> = rtl.outputs.iter().zip(&o).map(|((p, _), v)| format!("{}={v}", p.name)).collect();
                println!("{c:5}: {}", cols.join("  "));
            }
        }
        "test" => {
            let (path, src) = read(1);
            let li = LineIndex::new(&src);
            let res = goldl::testbench::run_tests(&src);
            let mut failed = 0;
            for t in &res {
                if t.passed {
                    println!("ok    {} ({} cycles)", t.name, t.cycles);
                } else {
                    failed += 1;
                    let (sp, m) = t.failure.clone().unwrap();
                    let (l, c) = li.line_col(sp.start);
                    println!("FAIL  {} — {path}:{}:{}: {m}", t.name, l + 1, c + 1);
                }
            }
            println!("{} passed, {failed} failed", res.len() - failed);
            if failed > 0 {
                exit(1);
            }
        }
        "verify" => {
            let (path, src) = read(1);
            let cycles: i64 = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(2);
            let mut s = Session::new(&src, None).unwrap_or_else(|d| {
                print_diags(&path, &src, &d);
                exit(1)
            });
            let t = s.period();
            s.ensure(cycles as usize + 2);
            let p0 = goldl_life::Pattern::from_cells(s.cells(0, Rect::ALL));
            let mut h = goldl_life::hashlife::HashLife::from_pattern(&p0);
            let mut now = 0;
            for k in 1..=2 * cycles {
                let g = k * t / 2 + 11;
                h.step((g - now) as u64);
                now = g;
                let real = h.to_pattern();
                let model = goldl_life::Pattern::from_cells(s.cells(g, Rect::ALL));
                if real != model {
                    println!("MISMATCH at generation {g}");
                    exit(1);
                }
                println!("generation {g}: {} cells match", real.len());
            }
        }
        "lsp" => serve_lsp(),
        _ => {
            eprintln!("{USAGE}");
            exit(2);
        }
    }
}

fn parse_int(v: &str) -> u64 {
    if let Some(h) = v.strip_prefix("0x") {
        u64::from_str_radix(h, 16).unwrap_or(0)
    } else if let Some(b) = v.strip_prefix("0b") {
        u64::from_str_radix(b, 2).unwrap_or(0)
    } else {
        v.parse().unwrap_or(0)
    }
}

fn print_diags(path: &str, src: &str, diags: &[Diag]) {
    let li = LineIndex::new(src);
    let lines: Vec<&str> = src.lines().collect();
    for d in diags {
        let (l, c) = li.line_col(d.span.start);
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        eprintln!("{sev}: {}\n  --> {path}:{}:{}", d.message, l + 1, c + 1);
        if let Some(text) = lines.get(l as usize) {
            let (_, c1) = li.line_col(d.span.end.max(d.span.start + 1));
            let width = if li.line_col(d.span.end).0 == l { (c1 - c).max(1) } else { 1 };
            eprintln!("   |\n{:>3}| {text}\n   | {}{}", l + 1, " ".repeat(c as usize), "^".repeat(width as usize));
        }
        for (s, m) in &d.notes {
            let (l, c) = li.line_col(s.start);
            eprintln!("  note: {m} ({path}:{}:{})", l + 1, c + 1);
        }
    }
}

/// LSP over stdio with `Content-Length` framing.
fn serve_lsp() {
    let mut server = goldl::lsp::Server::new();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    loop {
        let mut len = None;
        loop {
            let mut line = String::new();
            if input.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(v) = line.strip_prefix("Content-Length:") {
                len = v.trim().parse::<usize>().ok();
            }
        }
        let Some(len) = len else { continue };
        let mut buf = vec![0u8; len];
        if input.read_exact(&mut buf).is_err() {
            return;
        }
        let Ok(msg) = Json::parse(&String::from_utf8_lossy(&buf)) else { continue };
        let exit_now = msg.get("method").as_str() == Some("exit");
        for out in server.handle(&msg) {
            let body = out.to_string();
            let mut o = stdout.lock();
            let _ = write!(o, "Content-Length: {}\r\n\r\n{body}", body.len());
            let _ = o.flush();
        }
        if exit_now {
            return;
        }
    }
}
