use anyhow::Context;
use std::{
    borrow::Cow,
    collections::HashSet,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};
use thiserror::Error;

use crate::{reader::RoseFileReader, RoseFile};

pub enum VfsFile<'a> {
    Buffer(Vec<u8>),
    View(&'a [u8]),
}

#[derive(Error, Debug)]
pub enum VfsError {
    #[error("File {0} not found")]
    FileNotFound(PathBuf),
}

impl<'a> From<&'a VfsFile<'a>> for RoseFileReader<'a> {
    fn from(file: &'a VfsFile<'a>) -> Self {
        match file {
            VfsFile::Buffer(vec) => RoseFileReader::from(vec),
            VfsFile::View(buf) => RoseFileReader::from(*buf),
        }
    }
}

#[derive(Default, Debug, Hash, Clone, PartialEq, Eq)]
pub struct VfsPathBuf {
    path: PathBuf,
}

impl VfsPathBuf {
    pub fn new(path: &str) -> Self {
        VfsPathBuf {
            path: VfsPath::normalise_path(path),
        }
    }

    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl<'a> From<&VfsPath<'a>> for VfsPathBuf {
    fn from(path: &VfsPath<'a>) -> Self {
        VfsPathBuf {
            path: path.path.to_path_buf(),
        }
    }
}

#[derive(Debug, Hash, Clone)]
pub struct VfsPath<'a> {
    path: Cow<'a, Path>,
}

impl<'a> VfsPath<'a> {
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Converts `\` to `/`, replaces `//` with `/`, uppercases and trims surrounding
    /// whitespace, in that order, building the result in one allocation.
    pub fn normalise_path(path: &str) -> PathBuf {
        // Trimming first gives the same result: separators are not whitespace and
        // uppercasing neither creates nor removes whitespace.
        let path = path.trim();
        let mut normalised = String::with_capacity(path.len());
        let mut separator_run = 0;
        for c in path.chars() {
            if c == '\\' || c == '/' {
                separator_run += 1;
                continue;
            }

            push_collapsed_separators(&mut normalised, separator_run);
            separator_run = 0;
            if c.is_ascii() {
                normalised.push(c.to_ascii_uppercase());
            } else {
                // Same per-character mapping as `str::to_uppercase`.
                normalised.extend(c.to_uppercase());
            }
        }
        push_collapsed_separators(&mut normalised, separator_run);
        normalised.into()
    }
}

/// `replace("//", "/")` replaces non-overlapping pairs left to right, so a run of
/// `run` separators becomes `ceil(run / 2)` separators.
fn push_collapsed_separators(normalised: &mut String, run: usize) {
    for _ in 0..run.div_ceil(2) {
        normalised.push('/');
    }
}

impl<'a> From<&'a str> for VfsPath<'_> {
    fn from(path: &'a str) -> Self {
        VfsPath {
            path: Cow::Owned(VfsPath::normalise_path(path)),
        }
    }
}

impl<'a> From<&'a String> for VfsPath<'_> {
    fn from(path: &'a String) -> Self {
        VfsPath {
            path: Cow::Owned(VfsPath::normalise_path(path.as_str())),
        }
    }
}

impl<'a> From<&'a Path> for VfsPath<'_> {
    fn from(path: &'a Path) -> Self {
        VfsPath {
            path: Cow::Owned(VfsPath::normalise_path(path.to_string_lossy().as_ref())),
        }
    }
}

impl From<PathBuf> for VfsPath<'_> {
    fn from(path: PathBuf) -> Self {
        VfsPath {
            path: Cow::Owned(VfsPath::normalise_path(path.to_string_lossy().as_ref())),
        }
    }
}

impl<'a> From<&'a VfsPath<'a>> for VfsPath<'a> {
    fn from(path: &'a VfsPath<'a>) -> Self {
        VfsPath {
            path: Cow::Borrowed(&path.path),
        }
    }
}

impl<'a> From<&'a VfsPathBuf> for VfsPath<'a> {
    fn from(path: &'a VfsPathBuf) -> Self {
        VfsPath {
            path: Cow::Borrowed(&path.path),
        }
    }
}

pub trait VirtualFilesystemDevice {
    fn open_file(&self, path: &VfsPath) -> Result<VfsFile, anyhow::Error>;
    fn exists(&self, path: &VfsPath) -> bool;

    /// Same as `open_file`, but a missing file is `Ok(None)` instead of a
    /// `VfsError`, so lookups that fall through to the next device allocate no error.
    fn try_open_file(&self, path: &VfsPath) -> Result<Option<VfsFile>, anyhow::Error> {
        match self.open_file(path) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.downcast_ref::<VfsError>().is_some() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Names of the files directly inside `dir`, or `None` when this device cannot
    /// list it exactly (the default).
    ///
    /// A `Some` listing is a promise: for every path whose parent equals `dir` and
    /// whose file name is not listed, `open_file` fails with `VfsError::FileNotFound`
    /// and `exists` is false. Listing extra names is allowed.
    fn list_dir(&self, _dir: &VfsPath) -> Option<Vec<OsString>> {
        None
    }
}

pub struct HostFilesystemDevice {
    pub root_path: PathBuf,
}

impl HostFilesystemDevice {
    pub fn new(root_path: PathBuf) -> Self {
        Self { root_path }
    }
}

/// True when the OS resolves `name` the same as the last component of a directory
/// path and as an inner component of a file path. Windows trims trailing dots and
/// spaces and maps device names (CON, COM1, ...) only in the last component.
fn is_plain_directory_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.is_empty()
        || name.ends_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return false;
    }

    let stem = name.split('.').next().unwrap_or(name);
    let is_device_name = ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
        || (stem.len() == 4
            && (stem[..3].eq_ignore_ascii_case("COM") || stem[..3].eq_ignore_ascii_case("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    !is_device_name
}

impl VirtualFilesystemDevice for HostFilesystemDevice {
    fn open_file(&self, vfs_path: &VfsPath) -> Result<VfsFile, anyhow::Error> {
        self.try_open_file(vfs_path)?
            .ok_or_else(|| VfsError::FileNotFound(vfs_path.path().into()).into())
    }

    fn try_open_file(&self, vfs_path: &VfsPath) -> Result<Option<VfsFile>, anyhow::Error> {
        // Every read error counts as a missing file, so later devices are still tried.
        Ok(std::fs::read(self.root_path.join(vfs_path.path()))
            .ok()
            .map(VfsFile::Buffer))
    }

    fn exists(&self, vfs_path: &VfsPath) -> bool {
        self.root_path.join(vfs_path.path()).exists()
    }

    fn list_dir(&self, dir: &VfsPath) -> Option<Vec<OsString>> {
        // Only a missing directory is answered: which names open inside an existing
        // host directory depends on the OS (case folding, 8.3 aliases), so those
        // files are probed by the caller instead.
        if !dir.path().file_name().is_some_and(is_plain_directory_name) {
            return None;
        }

        match std::fs::metadata(self.root_path.join(dir.path())) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Some(Vec::new()),
            _ => None,
        }
    }
}

/// Files that may exist directly inside one directory of a `VirtualFilesystem`,
/// from `VirtualFilesystem::list_dir`.
pub struct VfsDirListing {
    /// The listed directory and the names listed by any device, or `None` when a
    /// device could not list it.
    listing: Option<(PathBuf, HashSet<OsString>)>,
}

impl VfsDirListing {
    /// False only when no device can open `path`: its parent is the listed directory
    /// and no device listed its file name. Any other path may exist.
    pub fn may_exist(&self, path: &VfsPath) -> bool {
        let Some((dir, names)) = &self.listing else {
            return true;
        };

        match (path.path().parent(), path.path().file_name()) {
            (Some(parent), Some(name)) if parent == dir.as_path() => names.contains(name),
            _ => true,
        }
    }

    /// True when every device listed the directory, so `may_exist` can rule paths out.
    pub fn is_complete(&self) -> bool {
        self.listing.is_some()
    }
}

pub struct VirtualFilesystem {
    pub devices: Vec<Box<dyn VirtualFilesystemDevice + Send + Sync>>,
}

impl VirtualFilesystem {
    pub fn new(devices: Vec<Box<dyn VirtualFilesystemDevice + Send + Sync>>) -> Self {
        Self { devices }
    }

    pub fn exists<'a, P: Into<VfsPath<'a>>>(&self, path: P) -> bool {
        let vfs_path: VfsPath = path.into();

        for device in &self.devices {
            if device.exists(&vfs_path) {
                return true;
            }
        }

        false
    }

    /// Devices are tried in order and the first one holding the file wins. A device
    /// error other than a missing file ends the lookup with that error.
    pub fn open_file<'a>(&self, path: impl Into<VfsPath<'a>>) -> Result<VfsFile, anyhow::Error> {
        let vfs_path: VfsPath = path.into();

        for device in &self.devices {
            if let Some(file) = device.try_open_file(&vfs_path)? {
                return Ok(file);
            }
        }

        Err(VfsError::FileNotFound(vfs_path.path().into()).into())
    }

    /// Lists the files directly inside `dir` across all devices, so callers probing
    /// many generated names can skip the ones that cannot exist. When any device
    /// cannot list `dir`, every path may exist and must be probed as before.
    pub fn list_dir<'a>(&self, dir: impl Into<VfsPath<'a>>) -> VfsDirListing {
        let dir: VfsPath = dir.into();
        let mut names = HashSet::new();

        for device in &self.devices {
            match device.list_dir(&dir) {
                Some(device_names) => names.extend(device_names),
                None => return VfsDirListing { listing: None },
            }
        }

        VfsDirListing {
            listing: Some((dir.path().to_path_buf(), names)),
        }
    }

    pub fn read_file<'a, T: RoseFile + Sized, P: Into<VfsPath<'a>>>(
        &self,
        path: P,
    ) -> Result<T, anyhow::Error> {
        let vfs_path: VfsPath = path.into();

        let file = self.open_file(&vfs_path)?;
        RoseFile::read(RoseFileReader::from(&file), &Default::default())
            .with_context(|| format!("Failed to read {}", vfs_path.path().to_string_lossy()))
    }

    pub fn read_file_with<'a, T: RoseFile + Sized, P: Into<VfsPath<'a>>>(
        &self,
        path: P,
        options: &T::ReadOptions,
    ) -> Result<T, anyhow::Error> {
        let vfs_path: VfsPath = path.into();

        let file = self.open_file(&vfs_path)?;
        RoseFile::read(RoseFileReader::from(&file), options)
            .with_context(|| format!("Failed to read {}", vfs_path.path().to_string_lossy()))
    }
}
