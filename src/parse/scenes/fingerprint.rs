//! Fingerprints of the files a compiled scene read, so a scene kept between compiles
//! (`preview --watch`) is reused only while none of them changed, appeared, or disappeared.
//!
//! A file is identified by its size and modification time, plus a hash of its bytes when it is
//! small (transcripts, SVG artwork) or an HTML document of any size, since documents decide what
//! a scene compiles to; a directory (an image-sequence folder) by its
//! modification time and the sorted names it lists; a missing path by its absence, so a missing
//! asset that appears invalidates every scene that referred to it.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Files up to this size are also compared by content.
const HASHED_BYTES: u64 = 1 << 20;

/// Files modified this long before a compile started, or later, may have changed while it read
/// them (or within the timestamp resolution of the filesystem), so scenes that read them are not
/// kept for the next compile.
pub(super) const SETTLE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::parse) enum Fingerprint {
    Missing,
    File {
        len: u64,
        modified: Option<SystemTime>,
        hash: Option<u64>,
    },
    Dir {
        modified: Option<SystemTime>,
        names: u64,
    },
}

impl Fingerprint {
    pub(in crate::parse) fn of(path: &Path) -> Self {
        let Ok(meta) = fs::metadata(path) else {
            return Self::Missing;
        };
        let modified = meta.modified().ok();
        if meta.is_dir() {
            let mut names: Vec<_> = fs::read_dir(path)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok().map(|e| e.file_name()))
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            let mut hasher = DefaultHasher::new();
            names.hash(&mut hasher);
            return Self::Dir {
                modified,
                names: hasher.finish(),
            };
        }
        let len = meta.len();
        let document = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm"));
        let hash = (len <= HASHED_BYTES || document)
            .then(|| fs::read(path).ok())
            .flatten()
            .map(|bytes| {
                let mut hasher = DefaultHasher::new();
                bytes.hash(&mut hasher);
                hasher.finish()
            });
        Self::File {
            len,
            modified,
            hash,
        }
    }

    /// Whether the path was modified at or after `since` (or has no usable time).
    pub(super) fn recent(&self, since: SystemTime) -> bool {
        match self {
            Self::Missing => false,
            Self::File { modified, .. } | Self::Dir { modified, .. } => {
                modified.is_none_or(|time| time >= since)
            }
        }
    }
}

/// Fingerprints taken during one revalidation, so files shared by several scenes are read once.
#[derive(Default)]
pub(super) struct Memo(HashMap<PathBuf, Fingerprint>);

impl Memo {
    pub(super) fn get(&mut self, path: &Path) -> &Fingerprint {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| Fingerprint::of(path))
    }
}
