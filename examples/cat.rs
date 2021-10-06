use fdwalk::FileNode;
use std::io;

fn main() {
    let mut out = io::stdout();
    for fd in fdwalk::walk::<_, FileNode>(".") {
        io::copy(&mut fd.open().unwrap().unwrap(), &mut out).unwrap();
    }
}
