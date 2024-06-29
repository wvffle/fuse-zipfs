use std::{
    fs::Metadata,
    os::unix::fs::{MetadataExt, PermissionsExt},
    time::{Duration, UNIX_EPOCH},
};

use fuse3::{path::reply::FileAttr, FileType};

pub trait MetadataFileAttr {
    fn to_file_attr(&self) -> FileAttr;
}

impl MetadataFileAttr for Metadata {
    fn to_file_attr(&self) -> FileAttr {
        let kind = match self.is_dir() {
            true => FileType::Directory,
            false => FileType::RegularFile,
        };

        FileAttr {
            size: self.len(),
            blocks: self.blocks(),
            atime: UNIX_EPOCH + Duration::from_secs(self.atime() as u64),
            mtime: UNIX_EPOCH + Duration::from_secs(self.mtime() as u64),
            ctime: UNIX_EPOCH + Duration::from_secs(self.ctime() as u64),
            kind,
            perm: self.permissions().mode() as u16,
            nlink: self.nlink() as u32,
            uid: self.uid(),
            gid: self.gid(),
            rdev: self.rdev() as u32,
            blksize: self.blksize() as u32,
        }
    }
}
