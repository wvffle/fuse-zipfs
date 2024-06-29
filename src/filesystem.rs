use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::File as StdFile,
    io::{Read, SeekFrom},
    num::{NonZeroU32, NonZeroUsize},
    os::fd::{AsFd, AsRawFd},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
    vec::IntoIter,
};

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

type ZipFile = ZipArchive<Arc<StdFile>>;

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
    open_zip_files: Arc<RwLock<LruCache<PathBuf, ZipFile>>>,
    open_files: Arc<RwLock<BTreeMap<u64, OpenFile>>>,
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
            open_zip_files: Arc::new(RwLock::new(LruCache::new(cache_size))),
            open_files: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }

    fn get_data_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        let path = path.as_ref().to_string_lossy();
        let path = path.strip_prefix('/').unwrap_or(&path);
        self.data_dir.join(path)
    }

    fn get_zip_paths<P: AsRef<Path>>(path: P) -> Option<(PathBuf, PathBuf)> {
        let mut zip_index = None;

        let components = path.as_ref().components().rev().collect::<Vec<_>>();
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

    async fn open_zip(&self, zip_path: &PathBuf) -> Result<Option<ZipFile>> {
        // Get from cache
        if let Some(archive) = self.open_zip_files.write().await.get(zip_path) {
            return Ok(Some(archive.clone()));
        }

        let file = StdFile::open(zip_path)?;
        let archive = ZipArchive::new(Arc::new(file));

        let archive = match archive {
            Ok(archive) => archive,
            Err(err) => {
                error!("Error opening zip file: {:?}", err);
                return Ok(None);
            }
        };

        self.open_zip_files
            .write()
            .await
            .put(zip_path.to_path_buf(), archive.clone());

        Ok(Some(archive))
    }

    async fn readdir_zip<'a>(
        &self,
        offset: u64,
        zip_path: &Path,
        file_path: &Path,
    ) -> Result<ReplyDirectoryPlus<<Self as PathFilesystem>::DirEntryPlusStream<'a>>> {
        debug!(
            "readdir_zip: zip_path = {:?}, file_path = {:?}",
            zip_path, file_path
        );

        if offset != 0 {
            return Ok(ReplyDirectoryPlus {
                entries: tokio_stream::iter(vec![]),
            });
        }

        let Some(archive) = self.open_zip(&zip_path.to_path_buf()).await? else {
            // NOTE: This archive is corrupt, let's just say it does not contain any entries
            return Ok(ReplyDirectoryPlus {
                entries: tokio_stream::iter(vec![]),
            });
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

        debug!(
            "readdir_zip: slash_count = {}, file_string = {:?}, file_names = {:?}",
            slash_count, file_string, file_names
        );

        let file_attr = tokio::fs::metadata(zip_path).await?.to_file_attr();
        let mut dir_attr = file_attr;
        dir_attr.kind = FileType::Directory;
        dir_attr.perm = 0o555;

        debug!(
            "readdir_zip: archive files: {:?}",
            archive.file_names().collect::<Vec<_>>()
        );

        let dir = PathBuf::from(file_string);
        let entries = tokio_stream::iter(&file_names)
            .enumerate()
            .skip(offset as usize)
            .map({
                |(i, name)| {
                    debug!("readdir_zip: name = {:?}", name);

                    let is_file = archive
                        .clone()
                        .by_name(&dir.join(name).to_string_lossy())
                        .map(|entry| entry.is_file())
                        .unwrap_or(false);

                    let attr = match is_file {
                        true => file_attr,
                        false => dir_attr,
                    };

                    Ok(DirectoryEntryPlus {
                        kind: attr.kind,
                        name: name.into(),
                        offset: offset as i64 + i as i64 + 1,
                        attr,
                        entry_ttl: TTL,
                        attr_ttl: TTL,
                    })
                }
            })
            .collect::<Vec<_>>()
            .await;

        Ok(ReplyDirectoryPlus {
            entries: tokio_stream::iter(entries),
        })
    }
}

impl PathFilesystem for ZipFs {
    type DirEntryStream<'a> = Empty<Result<DirectoryEntry>> where Self: 'a;
    type DirEntryPlusStream<'a> = Iter<IntoIter<Result<DirectoryEntryPlus>>> where Self: 'a;

    async fn init(&self, _req: Request) -> Result<ReplyInit> {
        Ok(ReplyInit {
            max_write: NonZeroU32::new(16 * 1024).unwrap(),
        })
    }

    async fn destroy(&self, _req: Request) {}

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

        if let Some((ref zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
            let mut attr = tokio::fs::metadata(zip_path).await?.to_file_attr();

            let Some(archive) = self.open_zip(zip_path).await? else {
                // NOTE: This archive is corrupt, let's just say it does not contain any entries
                attr.kind = FileType::Directory;
                attr.perm = 0o555;
                return Ok(ReplyAttr { ttl: TTL, attr });
            };

            let is_dir = archive
                .clone()
                .by_name(file_path.to_string_lossy().as_ref())
                .map(|entry| entry.is_dir())
                // In case path does not exist in zip file, let's assume it's a directory
                .unwrap_or(true);

            if is_dir {
                attr.kind = FileType::Directory;
                attr.perm = 0o555;
            }

            return Ok(ReplyAttr { ttl: TTL, attr });
        }

        let metadata = tokio::fs::metadata(path).await?;
        let attr = metadata.to_file_attr();

        Ok(ReplyAttr { ttl: TTL, attr })
    }

    async fn lookup(&self, req: Request, parent: &OsStr, name: &OsStr) -> Result<ReplyEntry> {
        let path = PathBuf::from(parent).join(name);
        let path = path.to_str().map(|x| x.as_ref());
        let attr = self.getattr(req, path, None, 0).await?.attr;
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

        if ZipFs::get_zip_paths(path).is_some() {
            return Ok(ReplyOpen {
                fh: 0,
                flags: libc::O_RDONLY as u32,
            });
        }

        let file = File::open(self.get_data_path(path)).await?;
        let fh = 1 + file.as_fd().as_raw_fd() as u64;
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

        if fh == 0 {
            let path = self.get_data_path(path.ok_or_else(Errno::new_not_exist)?);

            if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
                if let Some(mut archive) = self.open_zip(&zip_path.to_path_buf()).await? {
                    let entry = archive
                        .by_name(file_path.to_string_lossy().as_ref())
                        .map_err(|_| Errno::new_not_exist())?;

                    let data = entry
                        .bytes()
                        .skip(offset as usize)
                        .take(size as usize)
                        .collect::<std::result::Result<Bytes, _>>()?;

                    return Ok(ReplyData { data });
                }

                error!("Error opening zip file");
                return Err(Errno::new_not_exist());
            }
        }

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

        if let Some((zip_path, file_path)) = ZipFs::get_zip_paths(&path) {
            debug!(
                "readdirplus: zip_path={:?}, file_path={:?}",
                zip_path, file_path
            );
            return self.readdir_zip(offset, &zip_path, &file_path).await;
        }

        let metadata = tokio::fs::metadata(&path).await?;
        if !metadata.is_dir() {
            return Err(Errno::new_is_not_dir());
        }

        let entries = ReadDirStream::new(tokio::fs::read_dir(&path).await?)
            .enumerate()
            .skip(offset as usize)
            .then(|(i, entry)| async move {
                let entry = entry?;
                let metadata = entry.metadata().await?;
                let mut attr = metadata.to_file_attr();

                if entry.file_name().to_string_lossy().ends_with(".zip") {
                    attr.kind = FileType::Directory;
                    attr.perm = 0o555;
                }

                Ok(DirectoryEntryPlus {
                    kind: attr.kind,
                    name: entry.file_name(),
                    offset: offset as i64 + i as i64 + 1,
                    attr,
                    entry_ttl: TTL,
                    attr_ttl: TTL,
                })
            })
            .collect::<Vec<_>>()
            .await;

        debug!("readdirplus: entries={:?}", entries);

        Ok(ReplyDirectoryPlus {
            entries: tokio_stream::iter(entries),
        })
    }
}
