// TODO: LRU cache for the zip file handles
use std::{
    ffi::{OsStr, OsString},
    io::Read,
    num::{NonZeroU32, NonZeroUsize},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{mpsc::Sender, Arc},
    time::{Duration, UNIX_EPOCH},
    vec::IntoIter,
};

type FileHandle = u64;

use crate::file_tree::InnerFs;
use fuse3::path::prelude::*;
use fuse3::{Errno, Result};
use futures_util::{FutureExt, StreamExt};
// use futures_util::stream::{self, Empty, Iter};
use lru::LruCache;
use tokio::fs::{DirEntry, File};
use tokio_stream::{wrappers::ReadDirStream, Empty, Iter};
use tracing::{debug, error};
use zip::ZipArchive;

const TTL: Duration = Duration::from_secs(1);

pub struct ZipFs {
    data_dir: PathBuf,
    umount: Option<Sender<()>>,
    open_files: LruCache<FileHandle, ZipArchive<Arc<File>>>,
    tree: InnerFs,
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
            data_dir,
            umount,
            open_files: LruCache::new(cache_size),
            // tree: FileTree::new(data_dir),
            tree: InnerFs::default(),
        }
    }

    fn get_data_path(&self, path: &OsStr) -> OsString {
        self.data_dir.join(path).into()
    }

    // fn get_or_create_inode(&mut self, path: PathBuf) -> Inode {
    //     self.tree
    //         .find_inode_by_path(&path)
    //         .unwrap_or_else(|| self.tree.add_file(path))
    // }

    // fn get_zip_paths(path: &Path) -> Option<(PathBuf, PathBuf)> {
    //     let mut zip_index = None;

    //     let components = path.components().rev().collect::<Vec<_>>();
    //     for (index, component) in components.iter().enumerate() {
    //         if let Some(extension) = component.as_os_str().to_str() {
    //             if extension.ends_with(".zip") {
    //                 zip_index = Some(index);
    //                 break;
    //             }
    //         }
    //     }

    //     match zip_index {
    //         Some(index) => {
    //             let zip_path = components[index..].iter().rev().collect::<PathBuf>();
    //             let file_path = components[..index].iter().rev().collect::<PathBuf>();
    //             Some((zip_path, file_path))
    //         }
    //         None => None,
    //     }
    // }

    // fn getattr_(&mut self, ino: Inode) -> Result<FileAttr> {
    //     let path = self.get_data_path(ino)?;

    //     if let Some((ref zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
    //         let metadata = fs::metadata(zip_path).map_err(map_io_error)?;
    //         let mut attrs = metadata_to_file_attrs(metadata)?;
    //         // attrs.ino = ino;

    //         let Some(archive) = self.open_zip(zip_path)? else {
    //             attrs.kind = FileType::Directory;
    //             attrs.perm = 0o555;
    //             return Ok(attrs);
    //         };

    //         let is_dir = archive
    //             .clone()
    //             .by_name(file_path.to_string_lossy().as_ref())
    //             .map(|entry| entry.is_dir())
    //             .unwrap_or(true);

    //         if is_dir {
    //             attrs.kind = FileType::Directory;
    //             attrs.perm = 0o555;
    //         } else {
    //             attrs.kind = FileType::RegularFile;
    //             attrs.perm = 0o444;
    //         }

    //         Ok(attrs)
    //     } else {
    //         let metadata = fs::metadata(&path).map_err(map_io_error)?;
    //         let mut attrs = metadata_to_file_attrs(metadata)?;
    //         attrs.ino = ino;
    //         Ok(attrs)
    //     }
    // }

    // fn open_zip(&mut self, zip_path: &PathBuf) -> Result<Option<ZipArchive<Arc<File>>>> {
    //     let Some(ino) = self.tree.find_inode_by_path(zip_path) else {
    //         // NOTE: Maybe create the inode then?
    //         error!("inode not found for zip = {:?}", zip_path);
    //         return Err(Errno::new_not_exist());
    //     };

    //     // Get from cache
    //     if let Some(archive) = self.open_files.get(&ino) {
    //         return Ok(Some(archive.clone()));
    //     }

    //     let file = fs::File::open(zip_path).map_err(map_io_error)?;
    //     let archive = ZipArchive::new(Arc::new(file));

    //     let archive = match archive {
    //         Ok(archive) => archive,
    //         Err(err) => {
    //             error!("Error opening zip file: {:?}", err);
    //             return Ok(None);
    //         }
    //     };

    //     self.open_files.put(ino, archive.clone());
    //     Ok(Some(archive))
    // }

    // fn readdir_zip(
    //     &mut self,
    //     ino: Inode,
    //     offset: i64,
    //     zip_path: &Path,
    //     file_path: &Path,
    //     reply: &mut ReplyDirectory,
    // ) -> Result<()> {
    //     debug!("zip_path = {:?}, file_path = {:?}", zip_path, file_path);
    //     if offset != 0 {
    //         return Ok(());
    //     }

    //     let Some(mut archive) = self.open_zip(&zip_path.to_path_buf())? else {
    //         return Ok(());
    //     };

    //     let file_string = file_path.to_string_lossy().to_string() + "/";
    //     let file_string = file_string.strip_prefix('/').unwrap_or(&file_string);
    //     let slash_count = file_string.chars().filter(|c| *c == '/').count();

    //     let cloned_archive = archive.clone();
    //     let mut file_names = cloned_archive
    //         .file_names()
    //         .filter(|name| name.starts_with(file_string))
    //         .filter_map(|name| name.split('/').nth(slash_count))
    //         .collect::<Vec<_>>();

    //     file_names.dedup();
    //     file_names.sort_by_key(|name| name.len());

    //     debug!("slash_count = {}", slash_count);
    //     debug!("file_string = {:?}", file_string);
    //     debug!("file_names = {:?}", file_names);

    //     for (i, name) in file_names.iter().enumerate() {
    //         let Ok(entry) = archive.by_name(name) else {
    //             if reply.add(ino, offset + i as i64 + 1, FileType::Directory, name) {
    //                 break;
    //             }

    //             continue;
    //         };

    //         let Some(file_name) = entry.enclosed_name() else {
    //             error!("file name invalid = {:?}", name);
    //             continue;
    //         };

    //         debug!("file_name = {:?}", file_name);
    //         let ino = self.get_or_create_inode(zip_path.join(&file_name));

    //         debug!("offset = {}", offset,);

    //         if reply.add(
    //             ino,
    //             offset + i as i64 + 1,
    //             FileType::RegularFile,
    //             &file_name,
    //         ) {
    //             break;
    //         }
    //     }

    //     Ok(())
    // }

    // fn readdir_(
    //     &mut self,
    //     ino: Inode,
    //     _fh: FileHandle,
    //     offset: i64,
    //     reply: &mut ReplyDirectory,
    // ) -> Result<()> {
    //     let path = self.get_data_path(ino)?;

    //     if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
    //         return self.readdir_zip(ino, offset, &zip_path, &file_path, reply);
    //     }

    //     let metadata = fs::metadata(&path).map_err(map_io_error)?;
    //     if !metadata.is_dir() {
    //         return Err(Errno::new_is_not_dir());
    //     }

    //     let entries = fs::read_dir(&path).map_err(map_io_error)?;

    //     for (i, entry) in entries.skip(offset as usize).enumerate() {
    //         let entry = entry.map_err(map_io_error)?;

    //         let file_type = entry.file_type().map_err(map_io_error).map(map_ft)??;
    //         let file_name = entry.file_name().to_string_lossy().to_string();

    //         let file_path = path.join(&file_name);

    //         // TODO: If extension is .zip, say it's a directory

    //         let ino = self.get_or_create_inode(file_path);
    //         if reply.add(ino, offset + i as i64 + 1, file_type, file_name) {
    //             break;
    //         }
    //     }

    //     Ok(())
    // }

    // fn lookup_(&mut self, parent: Inode, name: &std::ffi::OsStr) -> Result<FileAttr> {
    //     let parent_path = self.get_data_path(parent)?;
    //     let path = parent_path.join(name);
    //     let ino = self.get_or_create_inode(path);
    //     self.getattr_(ino)
    // }

    // fn read_(&mut self, ino: Inode, _fh: FileHandle, offset: i64, size: u32) -> Result<Vec<u8>> {
    //     let path = self.get_data_path(ino)?;

    //     if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
    //         if let Some(mut archive) = self.open_zip(&zip_path.to_path_buf())? {
    //             let entry = archive
    //                 .by_name(file_path.to_string_lossy().as_ref())
    //                 .map_err(map_io_error)?;

    //             let data = entry
    //                 .bytes()
    //                 .skip(offset as usize)
    //                 .take(size as usize)
    //                 .collect::<std::result::Result<Vec<u8>, _>>()
    //                 .map_err(map_io_error)?;

    //             return Ok(data);
    //         }
    //     }

    //     let data = fs::read(path)
    //         .map_err(map_io_error)?
    //         .into_iter()
    //         .skip(offset as usize)
    //         .take(size as usize)
    //         .collect::<Vec<u8>>();

    //     Ok(data)
    // }
}

impl PathFilesystem for ZipFs {
    type DirEntryStream<'a> = Empty<Result<DirectoryEntry>> where Self: 'a;
    type DirEntryPlusStream<'a> = Iter<IntoIter<Result<DirectoryEntryPlus>>> where Self: 'a;

    async fn init(&self, _req: Request) -> Result<ReplyInit> {
        Ok(ReplyInit {
            max_write: NonZeroU32::new(16 * 1024).unwrap(),
        })
    }

    async fn destroy(&self, _req: Request) -> () {}

    // fn getattr(
    // fn readdir(
    // fn lookup(
    // fn read(

    async fn readdirplus<'a>(
        &'a self,
        _req: Request,
        parent: &'a OsStr,
        _fh: u64,
        offset: u64,
        _lock_owner: u64,
    ) -> Result<ReplyDirectoryPlus<Self::DirEntryPlusStream<'a>>> {
        debug!("readdirplus: parent={:?}, offset={}", parent, offset);

        let path = self.get_data_path(parent);

        //     if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
        //         return self.readdir_zip(ino, offset, &zip_path, &file_path, reply);
        //     }

        let metadata = tokio::fs::metadata(&path).await?;
        if !metadata.is_dir() {
            return Err(Errno::new_is_not_dir());
        }

        let entries = ReadDirStream::new(tokio::fs::read_dir(&path).await?)
            .enumerate()
            .then(|(i, entry)| async move {
                let entry = entry?;
                let attr = get_attr(&entry).await?;
                Ok(DirectoryEntryPlus {
                    kind: attr.kind,
                    name: entry.file_name(),
                    offset: offset as i64 + i as i64 + 1,
                    attr,
                    entry_ttl: TTL,
                    attr_ttl: TTL,
                })
            })
            .skip(offset as usize)
            .collect::<Vec<_>>()
            .await;

        debug!("readdirplus: entries={:?}", entries);

        Ok(ReplyDirectoryPlus {
            entries: tokio_stream::iter(entries),
        })
    }
}

async fn get_attr(entry: &DirEntry) -> Result<FileAttr> {
    let metadata = entry.metadata().await?;

    let kind = if metadata.is_dir() {
        FileType::Directory
    } else {
        FileType::RegularFile
    };

    Ok(FileAttr {
        size: metadata.len(),
        blocks: metadata.blocks(),
        atime: UNIX_EPOCH + Duration::from_secs(metadata.atime() as u64),
        mtime: UNIX_EPOCH + Duration::from_secs(metadata.mtime() as u64),
        ctime: UNIX_EPOCH + Duration::from_secs(metadata.ctime() as u64),
        kind,
        perm: metadata.permissions().mode() as u16,
        nlink: metadata.nlink() as u32,
        uid: metadata.uid(),
        gid: metadata.gid(),
        rdev: metadata.rdev() as u32,
        blksize: metadata.blksize() as u32,
    })
}
