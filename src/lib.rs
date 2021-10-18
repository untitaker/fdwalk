#![deny(missing_docs)]

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
struct PathEntryInner {
    parent: Option<WithPath>,
    segment: OsString,
}

pub trait Entry {
    /// Construct a new "root" entry. This is called with potentially an entire directory path,
    /// i.e. whatever `walk` was called with.
    fn root(segment: &OsStr) -> Self;

    /// Get the current path segment.
    fn segment(&self) -> &OsStr;

    /// Create a child entry based on current one.
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self;
}

pub trait DirSink {
    /// Store the parent directory in a new instance (or don't...)
    fn dir_sink(dir: Arc<Dir>) -> Self;
}

/// A type parameter for `FileEntry` that is used to store parent FD.
pub struct WithOpen(Arc<Dir>);

impl DirSink for WithOpen {
    fn dir_sink(dir: Arc<Dir>) -> Self {
        WithOpen(dir)
    }

    // TODO: open() and friends should exist here, meaning DirSinks should keep track of basename.
}

/// A type parameter for `FileEntry` that is used to avoid storing any informationn for opening
/// files. Using this will avoid keeping file descriptors open.
pub struct WithoutOpen;

impl DirSink for WithoutOpen {
    fn dir_sink(_dir: Arc<Dir>) -> Self {
        WithoutOpen
    }
}

/// A path sink is a type that can be used to keep track of filesystem paths while walking
/// directories.
pub trait PathSink: Sized {
    /// Construct a "child entry" from a given one + a path segment.
    fn path_sink(base: Option<&Self>, segment: &OsStr) -> Self;
}

/// A type parameter for `FileEntry` that is used to keep track of paths.
///
/// Paths are tracked as a linked list constructed of Arcs right now, this may be improved in the
/// future. You can implement your own way of keeping track of (parts of) file paths by
/// implementing `PathSink`.
#[derive(Debug, Clone)]
pub struct WithPath(Arc<PathEntryInner>);

impl PathSink for WithPath {
    fn path_sink(base: Option<&Self>, segment: &OsStr) -> Self {
        WithPath(Arc::new(PathEntryInner {
            parent: base.map(|x| x.clone()),
            segment: segment.to_owned(),
        }))
    }
}

/// A type parameter for `FileEntry` to avoid storing paths.
pub struct WithoutPath;

impl PathSink for WithoutPath {
    fn path_sink(_base: Option<&Self>, _segment: &OsStr) -> Self {
        WithoutPath
    }
}

/// A value returned by the `Walk` iterator. Represents a file, socket, or anything that is not a
/// directory.
pub struct FileEntry<D = WithoutOpen, P = WithoutPath> {
    parent_node: Option<P>,
    parent_dir: Option<D>,
    segment: OsString,
}

impl<P: PathSink> FileEntry<WithOpen, P> {
    /// call `stat()` for this file.
    pub fn stat(&self) -> Result<FileStat, Errno> {
        fstatat(
            self.parent_dir.as_ref().unwrap().0.as_raw_fd(),
            self.segment.as_os_str(),
            AtFlags::AT_SYMLINK_NOFOLLOW,
        )
    }

    /// Open the file for reading.
    pub fn open(&self) -> Result<File, Errno> {
        self.open_options(OFlag::empty(), Mode::empty())
    }

    /// Open the file with custom flags and open mode.
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

impl<D: DirSink> FileEntry<D, WithPath> {
    /// Return the file entry's path from a linked list kept in memory.
    ///
    /// This may return paths that exceed the size of paths that can be passed to syscalls.
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

impl<D: DirSink, P: PathSink> Entry for FileEntry<D, P> {
    fn root(segment: &OsStr) -> Self {
        FileEntry {
            parent_node: Some(P::path_sink(None, segment)),
            parent_dir: None,
            segment: segment.to_owned(),
        }
    }

    fn segment(&self) -> &OsStr {
        &self.segment
    }

    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self {
        FileEntry {
            parent_dir: Some(D::dir_sink(parent_dir.clone())),
            parent_node: Some(P::path_sink(self.parent_node.as_ref(), segment)),
            segment: segment.to_owned(),
        }
    }
}

/// The iterator returned from `walk`. Use its methods to configure directory walking.
pub struct Walk<N: Entry = FileEntry> {
    path: OsString,
    follow_symlinks: bool,
    walk_stack: Vec<(N, Option<Arc<Dir>>)>,
}

impl<N: Entry> Walk<N> {
    fn new(path: OsString, follow_symlinks: bool) -> Self {
        let walk_stack = vec![(N::root(&path), None)];

        Walk {
            path,
            follow_symlinks,
            walk_stack,
        }
    }

    fn with_node<N2: Entry>(self) -> Walk<N2> {
        Walk::new(self.path, self.follow_symlinks)
    }

    /// Follow symlinks.
    ///
    /// This may lead across filesystem boundaries and outside of the specified directory tree.
    pub fn follow_symlinks(mut self) -> Self {
        self.follow_symlinks = true;
        self
    }

    /// Do not follow symlinks (default).
    pub fn no_follow_symlinks(mut self) -> Self {
        self.follow_symlinks = false;
        self
    }
}

impl<D: DirSink, P: PathSink> Walk<FileEntry<D, P>> {
    /// Enable ability to get the path of the currrent file entry.
    ///
    /// This increases memory usage as now paths need to be kept in memory.
    pub fn with_paths(self) -> Walk<FileEntry<D, WithPath>> {
        self.with_node()
    }

    /// Disables ability to get the path of the current file entry (default).
    pub fn without_paths(self) -> Walk<FileEntry<D, WithoutPath>> {
        self.with_node()
    }

    /// Enable ability to open file directly from entry.
    ///
    /// This makes `open` and `open_options` available.
    pub fn with_open(self) -> Walk<FileEntry<WithOpen, P>> {
        self.with_node()
    }

    /// Disables ability to open file from entry (default).
    ///
    /// With neither paths nor open enabled, a file entry can only be used to count files.
    pub fn without_open(self) -> Walk<FileEntry<WithoutOpen, P>> {
        self.with_node()
    }
}

impl<N: Entry> Iterator for Walk<N> {
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

unsafe impl<N: Entry> Send for Walk<N> {}

/// Start recursively walking the directory given at `path`.
///
/// To configure the directory walker, use the methods on the return value:
///
///
/// ```rust
/// for entry in walk(".").with_paths().follow_symlinks() {
///     println!("{}", entry.to_path().display());
/// }
/// ```
pub fn walk<P: AsRef<Path>>(path: P) -> Walk {
    Walk::new(path.as_ref().as_os_str().to_owned(), false)
}
