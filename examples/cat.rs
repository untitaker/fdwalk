use std::io;

fn main() {
    let mut out = io::stdout();
    for fd in fdwalk::walk(".").with_open() {
        let fd = fd.unwrap();
        io::copy(&mut fd.open().unwrap(), &mut out).unwrap();
    }
}
