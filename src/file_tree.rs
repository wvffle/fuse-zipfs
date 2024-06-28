use std::{collections::BTreeMap, ffi::OsString, time::SystemTime};

use bytes::BytesMut;
use fuse3::{path::reply::FileAttr, FileType, SetAttr};
use libc::mode_t;

#[derive(Debug)]
enum Entry {
    Dir(Dir),
    File(File),
}

impl Entry {
    fn attr(&self) -> FileAttr {
        match self {
            Entry::Dir(dir) => FileAttr {
                size: 0,
                blocks: 0,
                atime: SystemTime::UNIX_EPOCH,
                mtime: SystemTime::UNIX_EPOCH,
                ctime: SystemTime::UNIX_EPOCH,
                kind: FileType::Directory,
                perm: fuse3::perm_from_mode_and_kind(FileType::Directory, dir.mode),
                nlink: 0,
                uid: 0,
                gid: 0,
                rdev: 0,
                blksize: 0,
            },

            Entry::File(file) => FileAttr {
                size: file.content.len() as _,
                blocks: 0,
                atime: SystemTime::UNIX_EPOCH,
                mtime: SystemTime::UNIX_EPOCH,
                ctime: SystemTime::UNIX_EPOCH,
                kind: FileType::RegularFile,
                perm: fuse3::perm_from_mode_and_kind(FileType::RegularFile, file.mode),
                nlink: 0,
                uid: 0,
                gid: 0,
                rdev: 0,
                blksize: 0,
            },
        }
    }

    fn set_attr(&mut self, set_attr: SetAttr) -> FileAttr {
        match self {
            Entry::Dir(dir) => {
                if let Some(mode) = set_attr.mode {
                    dir.mode = mode;
                }
            }

            Entry::File(file) => {
                if let Some(size) = set_attr.size {
                    file.content.truncate(size as _);
                }

                if let Some(mode) = set_attr.mode {
                    file.mode = mode;
                }
            }
        }

        self.attr()
    }

    fn is_dir(&self) -> bool {
        matches!(self, Entry::Dir(_))
    }

    fn is_file(&self) -> bool {
        !self.is_dir()
    }

    fn kind(&self) -> FileType {
        if self.is_dir() {
            FileType::Directory
        } else {
            FileType::RegularFile
        }
    }
}

#[derive(Debug)]
pub struct Dir {
    pub name: OsString,
    pub children: BTreeMap<OsString, Entry>,
    pub mode: mode_t,
}

#[derive(Debug)]
pub struct File {
    pub name: OsString,
    pub content: BytesMut,
    pub mode: mode_t,
}

#[derive(Debug)]
pub struct InnerFs {
    pub root: Entry,
}

impl Default for InnerFs {
    fn default() -> Self {
        Self {
            root: Entry::Dir(Dir {
                name: OsString::from("/"),
                children: Default::default(),
                mode: 0o755,
            }),
        }
    }
}
