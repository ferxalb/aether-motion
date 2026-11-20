//! Records the local paths a compile tries to read, while a persistent `SceneCache` compile runs,
//! so `preview --watch` can also watch the files of a compile that failed (for example an asset
//! that does not exist yet). Outside such a compile, recording is off and costs nothing.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    static READS: RefCell<Option<Vec<PathBuf>>> = const { RefCell::new(None) };
}

/// Notes that the compile on this thread resolved `path` (a document or an asset).
pub(in crate::parse) fn note(path: &Path) {
    READS.with(|reads| {
        if let Some(reads) = reads.borrow_mut().as_mut() {
            reads.push(path.to_path_buf());
        }
    });
}

/// Runs `compile` with recording on and returns its result with the paths it noted (sorted,
/// without duplicates).
pub(in crate::parse) fn recording<T>(compile: impl FnOnce() -> T) -> (T, Vec<PathBuf>) {
    let outer = READS.with(|reads| reads.replace(Some(Vec::new())));
    let result = compile();
    let mut paths = READS.with(|reads| reads.replace(outer)).unwrap_or_default();
    paths.sort();
    paths.dedup();
    (result, paths)
}
