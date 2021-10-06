fn main() {
    for node in fdwalk::walk(".") {
        println!("{}", node.to_path().display());
    }
}
