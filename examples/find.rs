use fdwalk::PathNode;

fn main() {
    for node in fdwalk::walk::<_, PathNode>(".") {
        println!("{}", node.to_path().display());
    }
}
