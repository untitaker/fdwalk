use fdwalk::PathNode;

fn main() {
    for node in fdwalk::walk::<_, PathNode>(".") {
        let node = node.unwrap();
        println!("{}", node.to_path().display());
    }
}
