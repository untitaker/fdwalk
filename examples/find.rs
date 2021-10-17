// supposed to be equivalent to rg --files -uuu

fn main() {
    for node in fdwalk::walk(".").with_paths().with_open() {
        let node = node.unwrap();
        println!("{}", node.to_path().display());
    }
}
