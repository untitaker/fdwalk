use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{openat, OFlag};
use nix::sys::stat::Mode;
use std::ffi::{CStr, OsStr, OsString};
use std::fs::File;
use std::mem::ManuallyDrop;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
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
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self;
}

impl Node for PathNode {
    fn root(segment: &OsStr) -> PathNode {
        PathNode(Arc::new(PathNodeInner {
            parent: None,
            segment: segment.to_owned(),
        }))
    }

    fn segment(&self) -> &OsStr {
        &self.0.segment
    }

    fn new_child(&self, _parent_dir: &Arc<Dir>, segment: &OsStr) -> Self {
        PathNode(Arc::new(PathNodeInner {
            parent: Some(self.clone()),
            segment: segment.to_owned(),
        }))
    }
}

pub struct FileNode {
    parent_dir: Option<Arc<Dir>>,
    segment: OsString,
}

impl FileNode {
    pub fn open(&self) -> Option<Result<File, Errno>> {
        self.open_options(OFlag::empty(), Mode::empty())
    }

    pub fn open_options(&self, oflag: OFlag, mode: Mode) -> Option<Result<File, Errno>> {
        let fd = openat(
            self.parent_dir.as_ref()?.as_raw_fd(),
            self.segment.as_os_str(),
            oflag,
            mode,
        );
        Some(fd.map(|x| unsafe { File::from_raw_fd(x) }))
    }
}

impl Node for FileNode {
    fn root(segment: &OsStr) -> Self {
        FileNode {
            parent_dir: None,
            segment: segment.to_owned(),
        }
    }
    fn segment(&self) -> &OsStr {
        &self.segment
    }
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self {
        FileNode {
            parent_dir: Some(parent_dir.clone()),
            segment: segment.to_owned(),
        }
    }
}

pub struct Walk<N> {
    walk_stack: Vec<(N, Option<Arc<Dir>>)>,
}

impl<N: Node> Iterator for Walk<N> {
    type Item = Result<N, Errno>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (node, parent_dir) = self.walk_stack.pop()?;

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
                    return Some(Ok(node));
                }
                Err(Errno::ENOENT) => continue,
                Err(e) => return Some(Err(e)),
            };

            let mut dir_iter = ManuallyDrop::new(Dir::from_fd(dir.as_raw_fd()).unwrap().into_iter());

            for entry in &mut *dir_iter {
                let entry = match entry {
                    Ok(x) => x,
                    Err(e) => return Some(Err(e)),
                };
                let fname = entry.file_name();
                if fname == unsafe { CStr::from_bytes_with_nul_unchecked(b".\0") }
                    || fname == unsafe { CStr::from_bytes_with_nul_unchecked(b"..\0") }
                {
                    continue;
                }

                let child = node.new_child(&dir, OsStr::from_bytes(fname.to_bytes()));
                self.walk_stack.push((child, Some(Arc::clone(&dir))));
            }
        }
    }
}

pub fn walk<P: AsRef<Path>, N: Node>(path: P) -> Walk<N> {
    Walk {
        walk_stack: vec![(N::root(path.as_ref().as_os_str()), None)],
    }
}
