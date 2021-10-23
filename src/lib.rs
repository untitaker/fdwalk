#![deny(missing_docs)]
//! TODO
pub mod fileentry;

use fileentry::{OpenProvider, FileEntry, PathProvider, WithOpen, WithPath, WithoutOpen, WithoutPath};

use nix::dir::Dir;
use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::sys::stat::Mode;
use std::ffi::{OsStr, OsString};
use std::mem::ManuallyDrop;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::sync::Arc;

/// A directory entry. You can potentially swap out the entire struct used to represent directory
/// entries for your own instead of using `FileEntry`, and customize every aspect of what data is
/// tracked while walking through directories.
pub trait Entry {
    /// Construct a new "root" entry. This is called with potentially an entire directory path,
    /// i.e. whatever `walk` was called with.
    fn root(segment: &OsStr) -> Self;

    /// Get the current path segment.
    fn segment(&self) -> &OsStr;

    /// Create a child entry based on current one.
    fn new_child(&self, parent_dir: &Arc<Dir>, segment: &OsStr) -> Self;
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

    fn with_entry<N2: Entry>(self) -> Walk<N2> {
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

impl<D: OpenProvider, P: PathProvider> Walk<FileEntry<D, P>> {
    /// Enable ability to get the path of the currrent file entry.
    ///
    /// This increases memory usage as now paths need to be kept in memory.
    pub fn with_paths(self) -> Walk<FileEntry<D, WithPath>> {
        self.with_entry()
    }

    /// Disables ability to get the path of the current file entry (default).
    pub fn without_paths(self) -> Walk<FileEntry<D, WithoutPath>> {
        self.with_entry()
    }

    /// Enable ability to open file directly from entry.
    ///
    /// This makes `open` and `open_options` available.
    pub fn with_open(self) -> Walk<FileEntry<WithOpen, P>> {
        self.with_entry()
    }

    /// Disables ability to open file from entry (default).
    ///
    /// With neither paths nor open enabled, a file entry can only be used to count files.
    pub fn without_open(self) -> Walk<FileEntry<WithoutOpen, P>> {
        self.with_entry()
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
