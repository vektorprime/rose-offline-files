use encoding_rs::EUC_KR;
use memmap::{Mmap, MmapOptions};
use std::{
    collections::{hash_map::Entry, HashMap},
    ffi::OsString,
    fs::File,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use crate::{reader::RoseFileReader, VfsError, VfsFile, VfsPath, VirtualFilesystemDevice};

struct FileEntry {
    storage: usize,
    offset: usize,
    size: usize,
}

#[derive(Default)]
pub struct VfsIndex {
    pub base_version: u32,
    pub current_version: u32,
    storages: Vec<Mmap>,
    /// Files of all storages in one map, so a lookup is a single hash probe.
    /// Precedence matches searching the storages in index order: a path present in
    /// several storages maps to the first storage holding it, and within one storage
    /// the last non-deleted record for a path wins.
    files: HashMap<PathBuf, FileEntry>,
    /// Directory -> names of the files directly inside it, built on first `list_dir`.
    directories: OnceLock<HashMap<PathBuf, Vec<OsString>>>,
}

impl VfsIndex {
    pub fn load(index_path: &Path) -> Result<VfsIndex, anyhow::Error> {
        let index_root_path = index_path
            .parent()
            .map(|path| path.into())
            .unwrap_or_else(PathBuf::new);
        let data = std::fs::read(index_path)?;
        let mut reader = RoseFileReader::from(&data);

        let base_version = reader.read_u32()?;
        let current_version = reader.read_u32()?;

        let num_vfs = reader.read_u32()? as usize;
        let mut storages = Vec::with_capacity(num_vfs);
        let mut files: HashMap<PathBuf, FileEntry> = HashMap::new();
        for _ in 0..num_vfs {
            let (filename, _, _) =
                EUC_KR.decode(reader.read_u16_length_bytes()?.split_last().unwrap().1);
            let offset = reader.read_u32()? as u64;

            let next_vfs_position = reader.position();
            reader.set_position(offset);

            let num_files = reader.read_u32()? as usize;
            let _ = reader.read_u32()?;
            let _ = reader.read_u32()?;

            if filename.to_uppercase() == "ROOT.VFS" {
                reader.set_position(next_vfs_position);
                continue;
            }

            let file = File::open(index_root_path.join(String::from(filename)))?;
            let mmap = unsafe { MmapOptions::new().map(&file)? };

            let storage = storages.len();
            files.reserve(num_files);

            for _ in 0..num_files {
                let (filename, _, _) =
                    EUC_KR.decode(reader.read_u16_length_bytes()?.split_last().unwrap().1);
                let offset = reader.read_u32()? as usize;
                let size = reader.read_u32()? as usize;
                let _block_size = reader.read_u32()?;
                let is_deleted = reader.read_u8()?;
                let _is_compressed = reader.read_u8()?;
                let _is_encrypted = reader.read_u8()?;
                let _version = reader.read_u32()?;
                let _crc = reader.read_u32()?;

                if is_deleted == 0 {
                    let entry = FileEntry {
                        storage,
                        offset,
                        size,
                    };
                    match files.entry(VfsPath::normalise_path(&filename)) {
                        Entry::Vacant(vacant) => {
                            vacant.insert(entry);
                        }
                        Entry::Occupied(mut occupied) => {
                            // Earlier storages keep precedence; a repeated record in
                            // the same storage replaces the previous one.
                            if occupied.get().storage == storage {
                                occupied.insert(entry);
                            }
                        }
                    }
                }
            }

            storages.push(mmap);
            reader.set_position(next_vfs_position);
        }

        Ok(VfsIndex {
            base_version,
            current_version,
            storages,
            files,
            directories: OnceLock::new(),
        })
    }

    fn find(&self, vfs_path: &VfsPath) -> Option<&[u8]> {
        let entry = self.files.get(vfs_path.path())?;
        Some(&self.storages[entry.storage][entry.offset..entry.offset + entry.size])
    }

    fn directories(&self) -> &HashMap<PathBuf, Vec<OsString>> {
        self.directories.get_or_init(|| {
            let mut directories: HashMap<PathBuf, Vec<OsString>> = HashMap::new();
            for path in self.files.keys() {
                let (Some(parent), Some(name)) = (path.parent(), path.file_name()) else {
                    continue;
                };
                if let Some(names) = directories.get_mut(parent) {
                    names.push(name.to_os_string());
                } else {
                    directories.insert(parent.to_path_buf(), vec![name.to_os_string()]);
                }
            }
            directories
        })
    }
}

impl VirtualFilesystemDevice for VfsIndex {
    fn open_file(&self, vfs_path: &VfsPath) -> Result<VfsFile, anyhow::Error> {
        self.find(vfs_path)
            .map(VfsFile::View)
            .ok_or_else(|| VfsError::FileNotFound(vfs_path.path().into()).into())
    }

    fn try_open_file(&self, vfs_path: &VfsPath) -> Result<Option<VfsFile>, anyhow::Error> {
        Ok(self.find(vfs_path).map(VfsFile::View))
    }

    fn exists(&self, vfs_path: &VfsPath) -> bool {
        self.files.contains_key(vfs_path.path())
    }

    fn list_dir(&self, dir: &VfsPath) -> Option<Vec<OsString>> {
        // Paths are compared by components (as the file lookup does), so a file
        // matches `dir` exactly when its parent equals `dir`.
        Some(
            self.directories()
                .get(dir.path())
                .cloned()
                .unwrap_or_default(),
        )
    }
}
