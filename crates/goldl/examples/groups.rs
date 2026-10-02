//! Print the design hierarchy (groups) of a design.
fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&path).unwrap();
    let (rtl, _) = goldl::elab::elaborate(&src, None).unwrap_or_else(|a| panic!("{:?}", a.diags));
    for (k, g) in rtl.groups.iter().enumerate() {
        println!(
            "{k:3} parent {:?} {:6} {:28} | {}",
            g.parent, g.kind, g.name, g.label
        );
    }
}
