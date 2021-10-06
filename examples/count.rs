use fdwalk::FileNode;

fn main() {
    let mut i = 0;
    for _ in fdwalk::walk::<_, FileNode>(".") {
        i += 1;
    }

    println!("{} files", i);
}
