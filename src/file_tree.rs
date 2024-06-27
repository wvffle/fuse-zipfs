use std::path::PathBuf;

use bimap::BiMap;
use fuser::FUSE_ROOT_ID;

type INode = u64;

#[derive(Debug)]
pub struct FileTree {
    entries: BiMap<INode, PathBuf>,
}

impl FileTree {
    pub fn new(data_dir: PathBuf) -> Self {
        let mut tree = Self {
            entries: BiMap::new(),
        };

        tree.add_file(data_dir);
        tree
    }

    pub fn add_file(&mut self, path: PathBuf) -> INode {
        let ino = FUSE_ROOT_ID + self.entries.len() as INode;

        self.entries.insert(ino, path);
        return ino;
    }

    pub fn find_path_by_inode(&self, inode: INode) -> Option<&PathBuf> {
        self.entries.get_by_left(&inode)
    }

    pub fn find_inode_by_path(&self, path: &PathBuf) -> Option<INode> {
        self.entries.get_by_right(path).copied()
    }
}
