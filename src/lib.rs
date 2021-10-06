use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use std::ffi::{CStr, OsStr, OsString};
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PathNode(Arc<PathNodeInner>);

#[derive(Debug)]
struct PathNodeInner {
    parent: Option<PathNode>,
    segment: OsString,
}

impl PathNode {
    pub fn to_path(&self) -> PathBuf {
        let mut rv = PathBuf::new();
        if let Some(ref parent) = self.0.parent {
            rv.push(parent.to_path());
        }

        rv.push(OsStr::from_bytes(self.0.segment.as_bytes()));
        rv
    }
}

pub trait Node {
    fn root(segment: &OsStr) -> Self;
    fn segment(&self) -> &OsStr;
    fn new_child(&self, segment: &OsStr) -> Self;
}

impl Node for PathNode {
    fn root(segment: &OsStr) -> PathNode {
        PathNode(Arc::new(PathNodeInner {
            parent: None,
            segment: segment.to_owned()
        }))
    }

    fn segment(&self) -> &OsStr {
        &self.0.segment
    }

    fn new_child(&self, segment: &OsStr) -> PathNode {
        PathNode(Arc::new(PathNodeInner {
            parent: Some(self.clone()),
            segment: segment.to_owned()
        }))
    }
}

pub struct SegmentNode(OsString);

impl Node for SegmentNode {
    fn root(segment: &OsStr) -> Self { SegmentNode(segment.to_owned()) }
    fn segment(&self) -> &OsStr { &self.0 }
    fn new_child(&self, segment: &OsStr) -> Self { SegmentNode(segment.to_owned()) }
}

pub fn walk<P: AsRef<Path>, N: Node>(path: P) -> impl Iterator<Item = N> {
    let mut entries = Vec::new();
    let mut walk_stack = vec![(
        N::root(path.as_ref().as_os_str()),
        None::<Arc<Dir>>,
    )];

    while let Some((node, parent_dir)) = walk_stack.pop() {
        let dir = if let Some(parent_dir) = parent_dir {
            Dir::openat(
                parent_dir.as_raw_fd(),
                node.segment(),
                OFlag::empty(),
                Mode::empty(),
            )
        } else {
            Dir::open(node.segment(), OFlag::empty(), Mode::empty())
        };

        let dir = match dir {
            Ok(x) => Arc::new(x),
            Err(Errno::ENOTDIR) => {
                entries.push(node);
                continue;
            }
            Err(Errno::ENOENT) => continue,
            Err(e) => panic!("failed to open {:?}: {}", node.segment(), e),
        };

        let mut dir2 = Dir::from_fd(dir.as_raw_fd()).unwrap();

        for entry in dir2.iter() {
            let entry = entry.unwrap();
            let fname = entry.file_name();
            if fname == unsafe { CStr::from_bytes_with_nul_unchecked(b".\0") }
                || fname == unsafe { CStr::from_bytes_with_nul_unchecked(b"..\0") }
            {
                continue;
            }

            let child = node.new_child(OsStr::from_bytes(fname.to_bytes()));
            walk_stack.push((child, Some(Arc::clone(&dir))));
        }

        // don't close fd yet -- that is done when `dir` is dropped.
        mem::forget(dir2);
    }

    entries.into_iter()
}
