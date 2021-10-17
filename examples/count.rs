use fdwalk::FileNode;

fn main() {
    let mut i = 0;
    for fd in fdwalk::walk::<_, FileNode>(".") {
        let _ = fd.unwrap();
        i += 1;
    }

    println!("{} files", i);
}
