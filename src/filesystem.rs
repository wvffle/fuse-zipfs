// TODO: LRU cache for the zip file handles
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    io::SeekFrom,
    num::{NonZeroU32, NonZeroUsize},
    os::fd::{AsFd, AsRawFd},
    path::PathBuf,
    sync::Arc,
    time::Duration,
    vec::IntoIter,
};

use crate::file_tree::InnerFs;
use crate::metadata::MetadataFileAttr;
use bytes::Bytes;
use fuse3::path::prelude::*;
use fuse3::{Errno, Result};
use futures_util::StreamExt;
use lru::LruCache;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt},
    sync::RwLock,
};
use tokio_stream::{wrappers::ReadDirStream, Empty, Iter};
use tracing::{debug, error};
use zip::ZipArchive;

const TTL: Duration = Duration::from_secs(1);

struct OpenFile {
    file: File,
    offset: usize,
}

impl OpenFile {
    fn new(file: File) -> Self {
        Self { file, offset: 0 }
    }

    async fn seek_to_offset(&mut self, offset: usize) -> Result<()> {
        self.offset = self.file.seek(SeekFrom::Start(offset as u64)).await? as usize;

        Ok(())
    }

    async fn read_bytes(&mut self, size: usize) -> tokio::io::Result<Bytes> {
        let mut data = vec![0; size];

        let n = self.file.read(&mut data).await?;
        self.offset += n;

        data.truncate(n);
        Ok(data.into())
    }
}

pub struct ZipFs {
    data_dir: PathBuf,
    open_zip_files: LruCache<u64, ZipArchive<Arc<File>>>,
    open_files: Arc<RwLock<BTreeMap<u64, OpenFile>>>,
    tree: InnerFs,
}

impl Drop for ZipFs {
    fn drop(&mut self) {
        debug!("Drop ZipFs");
    }
}

impl ZipFs {
    pub fn new(data_dir: PathBuf, cache_size: NonZeroUsize) -> Self {
        Self {
            data_dir,
            open_zip_files: LruCache::new(cache_size),
            open_files: Arc::new(RwLock::new(BTreeMap::new())),
            // tree: FileTree::new(data_dir),
            tree: InnerFs::default(),
        }
    }

    fn get_data_path(&self, path: &OsStr) -> PathBuf {
        let path = path.to_string_lossy();
        let path = path.strip_prefix('/').unwrap_or(&path);
        self.data_dir.join(path)
    }

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

    // fn open_zip(&mut self, zip_path: &PathBuf) -> Result<Option<ZipArchive<Arc<File>>>> {
    //     let Some(ino) = self.tree.find_inode_by_path(zip_path) else {
    //         // NOTE: Maybe create the inode then?
    //         error!("inode not found for zip = {:?}", zip_path);
    //         return Err(Errno::new_not_exist());
    //     };

    //     // Get from cache
    //     if let Some(archive) = self.open_zip_files.get(&ino) {
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

    //     self.open_zip_files.put(ino, archive.clone());
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

    async fn getattr(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        _fh: Option<u64>,
        _flags: u32,
    ) -> Result<ReplyAttr> {
        debug!("getattr: path={:?}", path);
        let path = self.get_data_path(path.ok_or_else(Errno::new_not_exist)?);
        debug!("getattr: data_path={:?}", path);

        // if let Some((ref zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
        //     let metadata = fs::metadata(zip_path).map_err(map_io_error)?;
        //     let mut attrs = metadata_to_file_attrs(metadata)?;
        //     // attrs.ino = ino;

        //     let Some(archive) = self.open_zip(zip_path)? else {
        //         attrs.kind = FileType::Directory;
        //         attrs.perm = 0o555;
        //         return Ok(attrs);
        //     };

        //     let is_dir = archive
        //         .clone()
        //         .by_name(file_path.to_string_lossy().as_ref())
        //         .map(|entry| entry.is_dir())
        //         .unwrap_or(true);

        //     if is_dir {
        //         attrs.kind = FileType::Directory;
        //         attrs.perm = 0o555;
        //     } else {
        //         attrs.kind = FileType::RegularFile;
        //         attrs.perm = 0o444;
        //     }

        //     return Ok(attrs);
        // }
        //
        //

        let metadata = tokio::fs::metadata(path).await?;
        let attr = metadata.to_file_attr();

        Ok(ReplyAttr { ttl: TTL, attr })
    }

    async fn lookup(&self, _req: Request, parent: &OsStr, name: &OsStr) -> Result<ReplyEntry> {
        debug!("lookup: parent={:?}, name={:?}", parent, name);

        let path = self.get_data_path(parent);
        let path = path.join(name);

        let metadata = tokio::fs::metadata(&path).await?;
        let attr = metadata.to_file_attr();

        Ok(ReplyEntry { ttl: TTL, attr })
    }

    async fn open(
        &self,
        _req: Request,
        path: &OsStr,
        // NOTE: We only support read-only mode, so we can safely ignore the flags
        _flags: u32,
    ) -> Result<ReplyOpen> {
        debug!("open: path={:?}", path);

        let file = File::open(self.get_data_path(path)).await?;
        let fh = file.as_fd().as_raw_fd() as u64;
        debug!("open: fh={}", fh);
        self.open_files
            .write()
            .await
            .insert(fh, OpenFile::new(file));

        Ok(ReplyOpen {
            fh,
            flags: libc::O_RDONLY as u32,
        })
    }

    async fn release(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        flags: u32,
        lock_owner: u64,
        flush: bool,
    ) -> Result<()> {
        debug!(
            "release: path={:?}, fh={}, flags={}, lock_owner={}, flush={}",
            path, fh, flags, lock_owner, flush
        );
        self.open_files.write().await.remove(&fh);
        Ok(())
    }

    async fn read(
        &self,
        _req: Request,
        path: Option<&OsStr>,
        fh: u64,
        offset: u64,
        size: u32,
    ) -> Result<ReplyData> {
        debug!("read: path={:?}, offset={}, size={}", path, offset, size);

        // let path = self.get_data_path(path.ok_or_else(Errno::new_not_exist)?);

        // if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
        //     if let Some(mut archive) = self.open_zip(&zip_path.to_path_buf())? {
        //         let entry = archive
        //             .by_name(file_path.to_string_lossy().as_ref())
        //             .map_err(map_io_error)?;

        //         let data = entry
        //             .bytes()
        //             .skip(offset as usize)
        //             .take(size as usize)
        //             .collect::<std::result::Result<Vec<u8>, _>>()
        //             .map_err(map_io_error)?;

        //         return Ok(data);
        //     }
        // }

        let mut files = self.open_files.write().await;
        let Some(file) = files.get_mut(&fh) else {
            error!("Bad file descriptor, fh={}", fh);
            return Err(Errno::from(libc::EBADF));
        };

        file.seek_to_offset(offset as usize).await?;
        let data = file.read_bytes(size as usize).await?;

        Ok(ReplyData { data })
    }

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
                let metadata = entry.metadata().await?;
                let attr = metadata.to_file_attr();
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
