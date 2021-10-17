// supposed to be equivalent to rg --files -uuu
use fdwalk::FileNode;
use nix::sys::stat::SFlag;

fn main() {
    for node in fdwalk::walk::<_, FileNode>(".") {
        let node = node.unwrap();
        let stat = node.stat().unwrap();

        // skip symlinks
        if unsafe { SFlag::from_bits_unchecked(stat.st_mode & SFlag::S_IFMT.bits()) }
            != SFlag::S_IFREG
        {
            continue;
        }

        println!("{}", node.to_path().display());
    }
}
