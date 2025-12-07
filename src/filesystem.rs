// TODO: LRU cache for the zip file handles
use crate::file_tree::FileTree;
use color_eyre::eyre::Result;

use fuser::{FileAttr, FileType, Filesystem};
use libc::{ENOENT, ENOSYS};
use lru::LruCache;
use std::{
    fs,
    io::{Read, Seek},
    num::NonZeroUsize,
    os::{linux::fs::MetadataExt, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::{Duration, UNIX_EPOCH},
};
use sync_file::{ReadAt, Size, SyncFile};
use tracing::{debug, error};
use zip::{CompressionMethod, ZipArchive};

type FuseError = libc::c_int;
type INode = u64;
type FileHandle = u64;
type Archive = ZipArchive<SyncFile>;

// TODO: Understand what it is
const TTL: Duration = Duration::from_secs(1);

fn map_io_error<E>(err: E) -> FuseError
where
    E: Into<std::io::Error>,
{
    let err: std::io::Error = err.into();
    err.raw_os_error().unwrap_or(libc::EIO)
}

fn map_ft(ft: fs::FileType) -> Result<fuser::FileType, FuseError> {
    match ft {
        e if e.is_dir() => Ok(fuser::FileType::Directory),
        e if e.is_file() => Ok(fuser::FileType::RegularFile),
        _ => Err(ENOSYS),
    }
}

fn metadata_to_file_attrs(metadata: fs::Metadata) -> Result<FileAttr, FuseError> {
    Ok(FileAttr {
        ino: metadata.st_ino(),
        size: metadata.st_size(),
        blocks: metadata.st_blocks(),
        atime: UNIX_EPOCH + Duration::from_secs(metadata.st_atime() as u64),
        mtime: UNIX_EPOCH + Duration::from_secs(metadata.st_mtime() as u64),
        ctime: UNIX_EPOCH + Duration::from_secs(metadata.st_ctime() as u64),
        crtime: UNIX_EPOCH + Duration::from_secs(metadata.st_ctime() as u64),
        kind: map_ft(metadata.file_type())?,
        perm: metadata.permissions().mode() as u16,
        nlink: metadata.st_nlink() as u32,
        uid: metadata.st_uid(),
        gid: metadata.st_gid(),
        rdev: metadata.st_rdev() as u32,
        blksize: metadata.st_blksize() as u32,
        flags: 0, // NOTE: macOS only
    })
}

pub struct ZipFs {
    umount: Option<Sender<()>>,
    open_files: LruCache<FileHandle, Archive>,
    tree: FileTree,
}

impl Drop for ZipFs {
    fn drop(&mut self) {
        debug!("Drop ZipFs");

        if let Some(umount) = &self.umount {
            umount.send(()).unwrap();
        }
    }
}

impl ZipFs {
    pub fn new(data_dir: PathBuf, cache_size: NonZeroUsize, umount: Option<Sender<()>>) -> Self {
        Self {
            umount,
            open_files: LruCache::new(cache_size),
            tree: FileTree::new(data_dir),
        }
    }

    fn get_data_path(&self, ino: INode) -> Result<PathBuf, FuseError> {
        let Some(path) = self.tree.find_path_by_inode(ino) else {
            error!("Path not found for ino = {}", ino);
            return Err(ENOENT);
        };

        Ok(path.to_path_buf())
    }

    fn get_or_create_inode(&mut self, path: PathBuf) -> INode {
        self.tree
            .find_inode_by_path(&path)
            .unwrap_or_else(|| self.tree.add_file(path))
    }

    fn get_zip_paths(path: &Path) -> Option<(PathBuf, PathBuf)> {
        let mut zip_index = None;

        let components = path.components().rev().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            if let Some(extension) = component.as_os_str().to_str() {
                if extension.ends_with(".zip") {
                    zip_index = Some(index);
                    break;
                }
            }
        }

        match zip_index {
            Some(index) => {
                let zip_path = components[index..].iter().rev().collect::<PathBuf>();
                let file_path = components[..index].iter().rev().collect::<PathBuf>();
                Some((zip_path, file_path))
            }
            None => None,
        }
    }

    fn getattr_(&mut self, ino: INode) -> Result<FileAttr, FuseError> {
        let path = self.get_data_path(ino)?;

        if let Some((ref zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
            let metadata = fs::metadata(zip_path).map_err(map_io_error)?;
            let mut attrs = metadata_to_file_attrs(metadata)?;
            attrs.ino = ino;

            let Some(archive) = self.open_zip(zip_path)? else {
                attrs.kind = FileType::Directory;
                attrs.perm = 0o555;
                return Ok(attrs);
            };

            let is_dir = archive
                .clone()
                .by_name(file_path.to_string_lossy().as_ref())
                .map(|entry| entry.is_dir())
                .unwrap_or(true);

            if is_dir {
                attrs.kind = FileType::Directory;
                attrs.perm = 0o555;
            } else {
                attrs.kind = FileType::RegularFile;
                attrs.perm = 0o444;
            }

            Ok(attrs)
        } else {
            let metadata = fs::metadata(&path).map_err(map_io_error)?;
            let mut attrs = metadata_to_file_attrs(metadata)?;
            attrs.ino = ino;
            Ok(attrs)
        }
    }

    fn open_zip(&mut self, zip_path: &PathBuf) -> Result<Option<Archive>, FuseError> {
        let Some(ino) = self.tree.find_inode_by_path(zip_path) else {
            // NOTE: Maybe create the inode then?
            error!("inode not found for zip = {:?}", zip_path);
            return Err(ENOENT);
        };

        // Get from cache
        if let Some(archive) = self.open_files.get(&ino) {
            return Ok(Some(archive.clone()));
        }

        let file = SyncFile::open(zip_path).map_err(map_io_error)?;
        let archive = ZipArchive::new(file);

        let archive = match archive {
            Ok(archive) => archive,
            Err(err) => {
                error!("Error opening zip file: {:?}", err);
                return Ok(None);
            }
        };

        self.open_files.put(ino, archive.clone());
        Ok(Some(archive))
    }

    fn readdir_zip(
        &mut self,
        ino: INode,
        offset: i64,
        zip_path: &Path,
        file_path: &Path,
        reply: &mut fuser::ReplyDirectory,
    ) -> Result<(), FuseError> {
        debug!("zip_path = {:?}, file_path = {:?}", zip_path, file_path);
        if offset != 0 {
            return Ok(());
        }

        let Some(mut archive) = self.open_zip(&zip_path.to_path_buf())? else {
            return Ok(());
        };

        let file_string = file_path.to_string_lossy().to_string() + "/";
        let file_string = file_string.strip_prefix('/').unwrap_or(&file_string);
        let slash_count = file_string.chars().filter(|c| *c == '/').count();

        let cloned_archive = archive.clone();
        let mut file_names = cloned_archive
            .file_names()
            .filter(|name| name.starts_with(file_string))
            .filter_map(|name| name.split('/').nth(slash_count))
            .collect::<Vec<_>>();

        file_names.dedup();
        file_names.sort_by_key(|name| name.len());

        debug!("slash_count = {}", slash_count);
        debug!("file_string = {:?}", file_string);
        debug!("file_names = {:?}", file_names);

        for (i, name) in file_names.iter().enumerate() {
            let Ok(entry) = archive.by_name(name) else {
                if reply.add(ino, offset + i as i64 + 1, FileType::Directory, name) {
                    break;
                }

                continue;
            };

            let Some(file_name) = entry.enclosed_name() else {
                error!("file name invalid = {:?}", name);
                continue;
            };

            debug!("file_name = {:?}", file_name);
            let ino = self.get_or_create_inode(zip_path.join(&file_name));

            debug!("offset = {}", offset,);

            if reply.add(
                ino,
                offset + i as i64 + 1,
                FileType::RegularFile,
                &file_name,
            ) {
                break;
            }
        }

        Ok(())
    }

    fn readdir_(
        &mut self,
        ino: INode,
        _fh: FileHandle,
        offset: i64,
        reply: &mut fuser::ReplyDirectory,
    ) -> Result<(), FuseError> {
        let path = self.get_data_path(ino)?;

        if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
            return self.readdir_zip(ino, offset, &zip_path, &file_path, reply);
        }

        let metadata = fs::metadata(&path).map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(libc::ENOTDIR);
        }

        let entries = fs::read_dir(&path).map_err(map_io_error)?;

        for (i, entry) in entries.skip(offset as usize).enumerate() {
            let entry = entry.map_err(map_io_error)?;

            let file_type = entry.file_type().map_err(map_io_error).map(map_ft)??;
            let file_name = entry.file_name().to_string_lossy().to_string();

            let file_path = path.join(&file_name);

            // TODO: If extension is a '.zip', say it's a directory

            let ino = self.get_or_create_inode(file_path);
            if reply.add(ino, offset + i as i64 + 1, file_type, file_name) {
                break;
            }
        }

        Ok(())
    }

    fn lookup_(
        &mut self,
        parent: INode,
        name: &std::ffi::OsStr,
    ) -> std::result::Result<FileAttr, FuseError> {
        let parent_path = self.get_data_path(parent)?;
        let path = parent_path.join(name);
        let ino = self.get_or_create_inode(path);
        self.getattr_(ino)
    }

    fn read_(
        &mut self,
        ino: INode,
        _fh: FileHandle,
        offset: i64,
        size: u32,
    ) -> std::result::Result<Vec<u8>, FuseError> {
        let path = self.get_data_path(ino)?;

        if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
            if let Some(mut archive) = self.open_zip(&zip_path.to_path_buf())? {
                let mut file = archive.clone().into_inner();

                let entry = archive
                    .by_name(file_path.to_string_lossy().as_ref())
                    .map_err(map_io_error)?;

                assert!(offset >= 0);
                let offset = offset as u64;
                let size: usize = std::cmp::min(
                    size as usize,
                    (entry.size() as usize).saturating_sub(offset as usize),
                );

                return match entry.compression() {
                    CompressionMethod::Stored => {
                        let mut buf = vec![0; size];

                        file.seek(std::io::SeekFrom::Start(offset + entry.data_start()))
                            .map_err(map_io_error)?;

                        file.read_exact(&mut buf).map_err(map_io_error)?;
                        Ok(buf)
                    }

                    _ => {
                        // FIXME: reader.seek does not work here, let's read entire file for now :/
                        // let mut reader = BufReader::with_capacity(size as usize, &mut entry);
                        // reader.seek(offset as usize);
                        // let mut buf = Vec::<u8>::with_capacity(size as usize);
                        // reader.read_exact(&mut buf).map_err(map_io_error)?;
                        // Ok(buf)

                        let data = entry
                            .bytes()
                            .skip(offset as usize)
                            .take(size)
                            .collect::<std::result::Result<Vec<u8>, _>>()
                            .map_err(map_io_error)?;

                        Ok(data)
                    }
                };
            }
        }

        let data = fs::read(path)
            .map_err(map_io_error)?
            .into_iter()
            .skip(offset as usize)
            .take(size as usize)
            .collect::<Vec<u8>>();

        Ok(data)
    }
}

impl Filesystem for ZipFs {
    fn getattr(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: INode,
        fh: Option<u64>,
        reply: fuser::ReplyAttr,
    ) {
        debug!("getattr: ino={}, fh={:?}", ino, fh);

        match self.getattr_(ino) {
            Ok(attrs) => reply.attr(&TTL, &attrs),
            Err(errno) => reply.error(errno),
        }
    }

    fn readdir(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: INode,
        fh: FileHandle,
        offset: i64,
        mut reply: fuser::ReplyDirectory,
    ) {
        debug!("readdir: ino={}, fh={}, offset={}", ino, fh, offset);

        match self.readdir_(ino, fh, offset, &mut reply) {
            Ok(_) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    fn lookup(
        &mut self,
        _req: &fuser::Request<'_>,
        parent: INode,
        name: &std::ffi::OsStr,
        reply: fuser::ReplyEntry,
    ) {
        debug!("lookup: parent={}, name={:?}", parent, name);

        match self.lookup_(parent, name) {
            Ok(attrs) => reply.entry(&TTL, &attrs, 0),
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &mut self,
        _req: &fuser::Request<'_>,
        ino: INode,
        fh: FileHandle,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: fuser::ReplyData,
    ) {
        debug!(
            "read: ino={}, fh={}, offset={}, size={}, flags={}, lock_owner={:?}",
            ino, fh, offset, size, flags, lock_owner
        );

        match self.read_(ino, fh, offset, size) {
            Ok(data) => reply.data(&data),
            Err(errno) => reply.error(errno),
        }
    }
}
