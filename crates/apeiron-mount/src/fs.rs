//! The FUSE side of the mount (MOUNT.md §3): inode numbers for the nodes
//! of [`Tree`], and files read piece by piece through
//! [`apeiron::FileReader::read_at`].

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use apeiron::{FileReader, Kind, ListRequest, Repository, Time};
use fuser::{
    BackgroundSession, Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, Request,
};

use crate::tree::{Child, Node, Tree};

/// Snapshots never change; ref folders may, so they are kept only briefly.
const SNAPSHOT_TTL: Duration = Duration::from_secs(3600);
const REF_TTL: Duration = Duration::from_secs(1);

struct Inodes {
    /// By inode number - 1; the first is the root.
    nodes: Vec<Child>,
    numbers: HashMap<Node, u64>,
}

struct Mount {
    tree: Tree,
    inodes: Mutex<Inodes>,
    open: Mutex<HashMap<u64, FileReader>>,
    next_handle: Mutex<u64>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn errno(e: &apeiron::Error) -> Errno {
    match e.kind {
        apeiron::ErrorKind::NotFound => Errno::ENOENT,
        _ => Errno::EIO,
    }
}

fn system_time(t: Time) -> SystemTime {
    let at = Duration::new(t.seconds.max(0).unsigned_abs(), t.nanos);
    UNIX_EPOCH + at
}

impl Mount {
    /// The inode number of `child`, given the first time it is seen.
    fn number(&self, child: &Child) -> u64 {
        let mut inodes = lock(&self.inodes);
        if let Some(&n) = inodes.numbers.get(&child.node) {
            return n;
        }
        inodes.nodes.push(child.clone());
        let n = inodes.nodes.len() as u64;
        inodes.numbers.insert(child.node.clone(), n);
        n
    }

    fn get(&self, ino: INodeNo) -> Option<Child> {
        let index = usize::try_from(ino.0).ok()?.checked_sub(1)?;
        lock(&self.inodes).nodes.get(index).cloned()
    }

    fn attr(req: &Request, ino: u64, child: &Child) -> FileAttr {
        let kind = match child.kind {
            Kind::Dir => FileType::Directory,
            Kind::File => FileType::RegularFile,
            Kind::Symlink => FileType::Symlink,
        };
        let mtime = system_time(child.mtime);
        FileAttr {
            ino: INodeNo(ino),
            size: child.size,
            blocks: child.size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind,
            // Read-only, whatever the backup recorded.
            perm: child.mode & 0o555,
            nlink: if kind == FileType::Directory { 2 } else { 1 },
            uid: req.uid(),
            gid: req.gid(),
            rdev: 0,
            blksize: 4096,
            flags: 0,
        }
    }

    fn ttl(child: &Child) -> Duration {
        match child.node {
            Node::Snap { .. } => SNAPSHOT_TTL,
            _ => REF_TTL,
        }
    }
}

impl Filesystem for Mount {
    fn lookup(&self, req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(parent) = self.get(parent) else {
            return reply.error(Errno::ENOENT);
        };
        match self.tree.child(&parent.node, name.as_bytes()) {
            Ok(Some(child)) => {
                let ino = self.number(&child);
                reply.entry(
                    &Self::ttl(&child),
                    &Self::attr(req, ino, &child),
                    Generation(0),
                );
            }
            Ok(None) => reply.error(Errno::ENOENT),
            Err(e) => reply.error(errno(&e)),
        }
    }

    fn getattr(&self, req: &Request, ino: INodeNo, _fh: Option<FileHandle>, reply: ReplyAttr) {
        match self.get(ino) {
            Some(child) => reply.attr(&Self::ttl(&child), &Self::attr(req, ino.0, &child)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn readlink(&self, _req: &Request, ino: INodeNo, reply: ReplyData) {
        match self.get(ino).and_then(|c| c.target) {
            Some(target) => reply.data(&target),
            None => reply.error(Errno::EINVAL),
        }
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(dir) = self.get(ino) else {
            return reply.error(Errno::ENOENT);
        };
        let children = match self.tree.children(&dir.node) {
            Ok(children) => children,
            Err(e) => return reply.error(errno(&e)),
        };
        let mut entries = vec![
            (ino.0, FileType::Directory, b".".to_vec()),
            (ino.0, FileType::Directory, b"..".to_vec()),
        ];
        for child in children {
            let kind = match child.kind {
                Kind::Dir => FileType::Directory,
                Kind::File => FileType::RegularFile,
                Kind::Symlink => FileType::Symlink,
            };
            entries.push((self.number(&child), kind, child.name));
        }
        for (i, (n, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(n), i as u64 + 1, kind, OsStr::from_bytes(&name)) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        if flags.0 & (libc_write_flags()) != 0 {
            return reply.error(Errno::EROFS);
        }
        let Some(Child {
            node: Node::Snap { id, path },
            kind: Kind::File,
            ..
        }) = self.get(ino)
        else {
            return reply.error(Errno::EISDIR);
        };
        let request = ListRequest {
            snapshot: id.to_hex(),
            path,
        };
        match self.tree.repo().open_file(&request) {
            Ok(reader) => {
                let mut next = lock(&self.next_handle);
                *next += 1;
                lock(&self.open).insert(*next, reader);
                reply.opened(FileHandle(*next), FopenFlags::FOPEN_KEEP_CACHE);
            }
            Err(e) => reply.error(errno(&e)),
        }
    }

    fn read(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let mut open = lock(&self.open);
        let Some(reader) = open.get_mut(&fh.0) else {
            return reply.error(Errno::EBADF);
        };
        let mut buf = vec![0; size as usize];
        match reader.read_at(offset, &mut buf) {
            Ok(n) => reply.data(&buf[..n]),
            Err(e) => reply.error(errno(&e)),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        lock(&self.open).remove(&fh.0);
        reply.ok();
    }
}

/// `O_WRONLY | O_RDWR`, the same on Linux and macOS.
const fn libc_write_flags() -> i32 {
    0o1 | 0o2
}

/// A mounted repository; dropping it unmounts it.
pub struct Mounted {
    session: BackgroundSession,
}

impl Mounted {
    /// Whether it is still mounted: unmounting it from outside, as with
    /// `fusermount -u` or `umount`, ends it.
    pub fn is_mounted(&self) -> bool {
        !self.session.guard.is_finished()
    }

    /// Unmounts it and waits until that is done.
    pub fn unmount(self) -> io::Result<()> {
        self.session.umount_and_join()
    }
}

/// Mounts the snapshots of `repo` read-only at the empty folder `at`.
pub fn mount(repo: Repository, at: &Path) -> io::Result<Mounted> {
    let root = Child {
        name: Vec::new(),
        node: Node::Folder(String::new()),
        kind: Kind::Dir,
        size: 0,
        mode: 0o555,
        mtime: Time {
            seconds: 0,
            nanos: 0,
        },
        target: None,
    };
    let mount = Mount {
        tree: Tree::new(repo),
        inodes: Mutex::new(Inodes {
            numbers: HashMap::from([(root.node.clone(), 1)]),
            nodes: vec![root],
        }),
        open: Mutex::default(),
        next_handle: Mutex::new(0),
    };
    let mut config = Config::default();
    config.mount_options = vec![
        MountOption::RO,
        MountOption::FSName("apeiron".to_owned()),
        MountOption::Subtype("apeiron".to_owned()),
        MountOption::NoDev,
        MountOption::NoSuid,
    ];
    Ok(Mounted {
        session: fuser::spawn_mount(mount, at, &config)?,
    })
}
