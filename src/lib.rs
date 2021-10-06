use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use std::ffi::{CStr, CString, OsStr};
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
pub struct LinkNode {
    parent: Option<Arc<LinkNode>>,
    segment: CString,
}

impl LinkNode {
    pub fn to_path(&self) -> PathBuf {
        let mut rv = PathBuf::new();
        if let Some(ref parent) = self.parent {
            rv.push(parent.to_path());
        }

        rv.push(OsStr::from_bytes(self.segment.as_bytes()));
        rv
    }
}

pub fn walk<P: AsRef<Path>>(path: P) -> impl Iterator<Item = LinkNode> {
    let mut entries = Vec::new();
    let mut walk_stack = vec![(
        Arc::new(LinkNode {
            parent: None,
            segment: CString::new(path.as_ref().as_os_str().as_bytes()).unwrap(),
        }),
        None::<Arc<Dir>>,
    )];

    while let Some((node, parent_dir)) = walk_stack.pop() {
        let dir = if let Some(parent_dir) = parent_dir {
            Dir::openat(
                parent_dir.as_raw_fd(),
                node.segment.as_bytes(),
                OFlag::empty(),
                Mode::empty(),
            )
        } else {
            Dir::open(node.segment.as_bytes(), OFlag::empty(), Mode::empty())
        };

        let dir = match dir {
            Ok(x) => Arc::new(x),
            Err(Errno::ENOTDIR) => {
                entries.push(Arc::try_unwrap(node).unwrap());
                continue;
            }
            Err(Errno::ENOENT) => continue,
            Err(e) => panic!("failed to open {:?}: {}", node.segment, e),
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

            let child = Arc::new(LinkNode {
                parent: Some(node.clone()),
                segment: fname.to_owned(),
            });

            walk_stack.push((child, Some(Arc::clone(&dir))));
        }

        // don't close fd yet -- that is done when `dir` is dropped.
        mem::forget(dir2);
    }

    entries.into_iter()
}
