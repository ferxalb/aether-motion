//! Records the local paths a compile tries to read, while a persistent `SceneCache` compile runs,
//! with a fingerprint of each taken when it is first noted, just before it is read. A watcher can
//! then also watch the files of a compile that failed (for example an asset that does not exist
//! yet), and the cache keeps a scene only if its files still match what the compile read.
//! Outside such a compile, recording is off and costs nothing.

use super::scenes::Fingerprint;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Each noted path with its fingerprint at the moment it was first noted.
pub(in crate::parse) type Reads = BTreeMap<PathBuf, Fingerprint>;

thread_local! {
    static READS: RefCell<Option<Reads>> = const { RefCell::new(None) };
}

/// Notes that the compile on this thread is about to read `path` (a document or an asset).
pub(in crate::parse) fn note(path: &Path) {
    READS.with(|reads| {
        if let Some(reads) = reads.borrow_mut().as_mut() {
            if !reads.contains_key(path) {
                reads.insert(path.to_path_buf(), Fingerprint::of(path));
            }
        }
    });
}

/// Runs `compile` with recording on and returns its result with the paths it noted.
pub(in crate::parse) fn recording<T>(compile: impl FnOnce() -> T) -> (T, Reads) {
    let outer = READS.with(|reads| reads.replace(Some(Reads::new())));
    let result = compile();
    let reads = READS.with(|reads| reads.replace(outer)).unwrap_or_default();
    (result, reads)
}
