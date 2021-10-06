use fdwalk::SegmentNode;

fn main() {
    let mut i = 0;
    for _ in fdwalk::walk::<_, SegmentNode>(".") {
        i += 1;
    }

    println!("{} files", i);
}
