//! Render the schematic of a design to SVG (debug aid; the playground has its own renderer).
use goldl::schematic::build;

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&path).unwrap();
    let (rtl, _) = goldl::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
    let s = build(&rtl);
    let mut o = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{}' height='{}' style='background:#111'>",
        s.size.0, s.size.1
    );
    for w in &s.wires {
        let pts: Vec<String> = w
            .points
            .iter()
            .map(|p| format!("{},{}", p.0, p.1))
            .collect();
        let col = if w.feedback { "#e9a" } else { "#7bd" };
        o += &format!(
            "<polyline points='{}' fill='none' stroke='{col}' stroke-width='{}'/>",
            pts.join(" "),
            if w.width > 1 { 2.2 } else { 1.0 }
        );
    }
    for n in &s.nodes {
        if n.kind == "dummy" {
            continue;
        }
        o += &format!(
            "<rect x='{}' y='{}' width='{}' height='{}' fill='#223' stroke='#ccc'/>",
            n.x, n.y, n.w, n.h
        );
        o += &format!(
            "<text x='{}' y='{}' fill='#fff' font-size='9' font-family='monospace'>{} {}</text>",
            n.x + 2.0,
            n.y + 10.0,
            n.kind,
            n.label.replace('<', "&lt;")
        );
    }
    o += "</svg>";
    print!("{o}");
}
