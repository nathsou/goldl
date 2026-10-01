fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(&path).unwrap();
    let t = std::time::Instant::now();
    match goldl::driver::compile(&src, &Default::default()) {
        Ok(c) => println!("{:#?}\ncompiled in {:?}", c.stats, t.elapsed()),
        Err((d, _)) => println!("errors: {d:?}"),
    }
}
