use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::{openat, AtFlags, OFlag};
use nix::sys::stat::{fstatat, FileStat, Mode};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::mem::ManuallyDrop;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug)]
struct PathNodeInner {
    parent: Option<WithPath>,
    segment: OsString,
}

pub trait Node {
    fn root(segment: &OsStr) -> Self;
    fn segment(&self) -> &OsStr;
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self;
}

pub trait DirSink {
    fn dir_sink(dir: Arc<Dir>) -> Self;
}

pub struct WithOpen(Arc<Dir>);

impl DirSink for WithOpen {
    fn dir_sink(dir: Arc<Dir>) -> Self {
        WithOpen(dir)
    }
}

pub struct WithoutOpen;

impl DirSink for WithoutOpen {
    fn dir_sink(_dir: Arc<Dir>) -> Self {
        WithoutOpen
    }
}

pub trait PathSink: Sized {
    fn path_sink(base: Option<&Self>, segment: &OsStr) -> Self;
}

#[derive(Debug, Clone)]
pub struct WithPath(Arc<PathNodeInner>);

impl PathSink for WithPath {
    fn path_sink(base: Option<&Self>, segment: &OsStr) -> Self {
        WithPath(Arc::new(PathNodeInner {
            parent: base.map(|x| x.clone()),
            segment: segment.to_owned(),
        }))
    }
}

pub struct WithoutPath;

impl PathSink for WithoutPath {
    fn path_sink(_base: Option<&Self>, _segment: &OsStr) -> Self {
        WithoutPath
    }
}

pub struct FileNode<D = WithoutOpen, P = WithoutPath> {
    parent_node: Option<P>,
    parent_dir: Option<D>,
    segment: OsString,
}

impl<P: PathSink> FileNode<WithOpen, P> {
    pub fn stat(&self) -> Result<FileStat, Errno> {
        fstatat(
            self.parent_dir.as_ref().unwrap().0.as_raw_fd(),
            self.segment.as_os_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        )
    }

    pub fn open(&self) -> Result<File, Errno> {
        self.open_options(OFlag::empty(), Mode::empty())
    }

    pub fn open_options(&self, oflag: OFlag, mode: Mode) -> Result<File, Errno> {
        let fd = openat(
            self.parent_dir.as_ref().unwrap().0.as_raw_fd(),
            self.segment.as_os_str(),
            oflag,
            mode,
        );
        fd.map(|x| unsafe { File::from_raw_fd(x) })
    }
}

impl<D: DirSink> FileNode<D, WithPath> {
    pub fn to_path(&self) -> PathBuf {
        // XXX: slow, also self.segment apparently == self.parent_node.segment?
        let mut segments = vec![];

        let mut current_opt: Option<&WithPath> = self.parent_node.as_ref();

        while let Some(ref mut current) = current_opt {
            segments.push(&current.0.segment);
            current_opt = current.0.parent.as_ref();
        }

        let mut rv = PathBuf::new();

        for segment in segments.into_iter().rev() {
            rv.push(segment);
        }

        rv
    }
}

impl<D: DirSink, P: PathSink> Node for FileNode<D, P> {
    fn root(segment: &OsStr) -> Self {
        FileNode {
            parent_node: None,
            parent_dir: None,
            segment: segment.to_owned(),
        }
    }
    fn segment(&self) -> &OsStr {
        &self.segment
    }
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self {
        FileNode {
            parent_dir: Some(D::dir_sink(parent_dir.clone())),
            parent_node: Some(P::path_sink(self.parent_node.as_ref(), segment)),
            segment: segment.to_owned(),
        }
    }
}

pub struct Walk<N: Node = FileNode> {
    path: OsString,
    follow_symlinks: bool,
    walk_stack: Vec<(N, Option<Arc<Dir>>)>,
}

impl<N: Node> Walk<N> {
    fn new(path: OsString, follow_symlinks: bool) -> Self {
        let walk_stack = vec![(N::root(&path), None)];

        Walk {
            path,
            follow_symlinks,
            walk_stack,
        }
    }

    pub fn with_node<N2: Node>(self) -> Walk<N2> {
        Walk::new(self.path, self.follow_symlinks)
    }

    pub fn follow_symlinks(mut self) -> Self {
        self.follow_symlinks = true;
        self
    }

    pub fn no_follow_symlinks(mut self) -> Self {
        self.follow_symlinks = false;
        self
    }
}

impl<D: DirSink, P: PathSink> Walk<FileNode<D, P>> {
    pub fn with_paths(self) -> Walk<FileNode<D, WithPath>> {
        self.with_node()
    }

    pub fn without_paths(self) -> Walk<FileNode<D, WithoutPath>> {
        self.with_node()
    }

    pub fn with_open(self) -> Walk<FileNode<WithOpen, P>> {
        self.with_node()
    }

    pub fn without_open(self) -> Walk<FileNode<WithoutOpen, P>> {
        self.with_node()
    }
}

impl<N: Node> Iterator for Walk<N> {
    type Item = Result<N, Errno>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (node, parent_dir) = self.walk_stack.pop()?;

            let oflags = if self.follow_symlinks {
                OFlag::empty()
            } else {
                OFlag::O_NOFOLLOW
            };

            let dir = if let Some(parent_dir) = parent_dir {
                Dir::openat(
                    parent_dir.as_raw_fd(),
                    node.segment(),
                    oflags,
                    Mode::empty(),
                )
            } else {
                Dir::open(node.segment(), oflags, Mode::empty())
            };

            let dir = match dir {
                Ok(x) => Arc::new(x),
                Err(Errno::ENOTDIR) => {
                    return Some(Ok(node));
                }
                Err(Errno::ENOENT) => continue,
                // emitted when follow_symlinks = false and we have a symlink
                Err(Errno::ELOOP) => continue,
                Err(e) => return Some(Err(e)),
            };

            let mut dir_iter =
                ManuallyDrop::new(Dir::from_fd(dir.as_raw_fd()).unwrap().into_iter());

            for entry in &mut *dir_iter {
                let entry = match entry {
                    Ok(x) => x,
                    Err(e) => return Some(Err(e)),
                };
                let fname = entry.file_name();
                if fname.to_bytes() == b"." || fname.to_bytes() == b".." {
                    continue;
                }

                let child = node.new_child(&dir, OsStr::from_bytes(fname.to_bytes()));
                self.walk_stack.push((child, Some(Arc::clone(&dir))));
            }
        }
    }
}

pub fn walk<P: AsRef<Path>>(path: P) -> Walk {
    Walk::new(path.as_ref().as_os_str().to_owned(), false)
}
