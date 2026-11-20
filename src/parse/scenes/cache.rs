//! Compilation of imported scenes, with reuse inside one compile and, for `preview --watch`,
//! across compiles.
//!
//! `SceneStack` compiles each imported document once per set of variable values: later instances
//! reuse a clone of the compiled scene (whose element bodies are shared, see
//! [`crate::model::Element`]) instead of re-reading and re-parsing it, with the same import-depth,
//! instance, and cycle checks a fresh compilation would make. A scene without `data-hf-split-wrap`
//! text compiles identically on both parses of a composition, so the second parse reuses the
//! first's; a scene with it is reused only by a parse that gives it the same line breaks.
//!
//! [`SceneCache`] holds the compiled scenes. A one-shot compile drops it afterwards; a persistent
//! cache (`SceneCache::persistent`) keeps every scene the last successful compile used, with
//! [fingerprints](super::fingerprint) of every file each one read (its documents, nested scenes,
//! and assets). The next compile drops a scene as soon as one of those files changed, appeared,
//! or disappeared, which also invalidates every scene that imports it, since an importing scene's
//! files include its imports'.

use super::fill_canvas;
use super::fingerprint::{Fingerprint, Memo, SETTLE};
use crate::model::Composition;
use crate::parse::split::SplitPlan;
use crate::parse::{parse_pass, VariableValues};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_DEPTH: usize = 8;
const MAX_INSTANCES: usize = 256;

/// Canonical document path, the canonical directory its relative assets resolve from (the
/// directory of the path as imported, which differs from the target's for a symlinked scene),
/// resolved variable values (as JSON), and whether the parse still measures `data-hf-split-wrap`
/// breaks (the first of the two parses).
type Key = (PathBuf, PathBuf, String, bool);

/// A compiled scene kept for reuse.
struct Compiled {
    /// With its root already filling its own canvas and marked as a scene root.
    scene: Composition,
    /// Import levels below the scene's own document.
    depth: usize,
    /// Instances compiled inside it (nested imports, counted each time they appear).
    instances: usize,
    /// Canonical paths of every document in its import tree, its own included.
    files: Vec<PathBuf>,
    /// First `data-hf-split-wrap` ordinal it claimed, and how many.
    wraps: (u32, u32),
    /// Breaks of those wrapping elements, in order (`None` on the first parse).
    breaks: Option<Vec<Vec<usize>>>,
    /// Fingerprints of `scene.dependencies`, taken after the compile that built it succeeded;
    /// empty until then.
    seen: Vec<Fingerprint>,
    /// Whether the current compile used it.
    used: bool,
}

/// Compiled scenes shared by the parses of one compile and, when persistent, by later compiles.
#[derive(Default)]
pub struct SceneCache {
    entries: HashMap<Key, Compiled>,
    persistent: bool,
    started: Option<SystemTime>,
    reads: Vec<PathBuf>,
}

impl SceneCache {
    /// A cache that keeps the scenes of each successful compile for the next one, as long as
    /// the files they read are unchanged.
    pub fn persistent() -> Self {
        Self {
            persistent: true,
            ..Self::default()
        }
    }

    /// Number of compiled scenes (distinct documents, values, and parses) currently kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Paths the last compile read or tried to read (documents and assets, including missing
    /// ones and those of reused scenes), sorted and without duplicates.
    pub fn last_reads(&self) -> &[PathBuf] {
        &self.reads
    }

    pub(in crate::parse) fn set_reads(&mut self, reads: Vec<PathBuf>) {
        self.reads = reads;
    }

    /// Before a compile: drops every kept scene that was never verified or whose files changed.
    pub(in crate::parse) fn begin(&mut self) {
        if !self.persistent {
            return;
        }
        self.started = Some(SystemTime::now());
        let mut memo = Memo::default();
        self.entries.retain(|_, compiled| {
            compiled.used = false;
            compiled.seen.len() == compiled.scene.dependencies.len()
                && compiled
                    .seen
                    .iter()
                    .zip(&compiled.scene.dependencies)
                    .all(|(seen, path)| memo.get(path) == seen)
        });
    }

    /// After a successful compile: keeps only the scenes it used. A new scene takes the
    /// fingerprints its files had when the compile first noted them (`observed`, taken just
    /// before reading) and is kept only if every file was noted and still matches them, so a file
    /// replaced while it was compiled is never trusted. A scene that read a file modified shortly
    /// before the compile began (or during it) is not kept either, since timestamps can be coarse.
    pub(in crate::parse) fn finish(&mut self, observed: &crate::parse::reads::Reads) {
        let Some(started) = self.started.filter(|_| self.persistent) else {
            self.entries.clear();
            return;
        };
        let since = started.checked_sub(SETTLE).unwrap_or(started);
        let mut memo = Memo::default();
        self.entries.retain(|_, compiled| {
            if !compiled.used {
                return false;
            }
            if compiled.seen.len() != compiled.scene.dependencies.len() {
                let read: Option<Vec<Fingerprint>> = compiled
                    .scene
                    .dependencies
                    .iter()
                    .map(|path| observed.get(path).filter(|seen| memo.get(path) == *seen))
                    .map(|seen| seen.cloned())
                    .collect();
                let Some(read) = read else {
                    return false;
                };
                compiled.seen = read;
            }
            !compiled.seen.iter().any(|seen| seen.recent(since))
        });
    }
}

pub(in crate::parse) struct SceneStack<'c> {
    active: Vec<PathBuf>,
    instances: usize,
    /// Deepest import level reached in the subtree being compiled.
    deepest: usize,
    /// Documents compiled under the scene being compiled (for the cache entry's cycle check).
    files: Vec<PathBuf>,
    cache: &'c mut SceneCache,
}

impl<'c> SceneStack<'c> {
    pub(in crate::parse) fn new(cache: &'c mut SceneCache) -> Self {
        Self {
            active: Vec::new(),
            instances: 0,
            deepest: 0,
            files: Vec::new(),
            cache,
        }
    }

    pub(in crate::parse) fn parse(
        &mut self,
        path: &Path,
        values: &VariableValues,
        splits: &SplitPlan,
    ) -> Result<Composition> {
        crate::parse::reads::note(path);
        let canonical = path
            .canonicalize()
            .with_context(|| format!("could not read {}", path.display()))?;
        if self.active.contains(&canonical) {
            bail!(
                "cycle in data-composition-src: {} is already active",
                path.display()
            );
        }
        let level = self.active.len();
        if level > MAX_DEPTH {
            bail!("data-composition-src supports at most {MAX_DEPTH} nested levels");
        }
        let imported = level > 0;
        if imported {
            self.count_instances(1)?;
        }
        let base = path
            .parent()
            .and_then(|dir| dir.canonicalize().ok())
            .unwrap_or_default();
        let key = (
            canonical.clone(),
            base,
            serde_json::to_string(values)?,
            splits.unresolved(),
        );
        if let Some(hit) = imported.then(|| self.reuse(&key, level, splits)).flatten() {
            // Another name for the same document (a symlink or relative alias in the same
            // directory) is a file of this composition too.
            return hit.map(|mut scene| {
                if !scene.dependencies.iter().any(|p| p == path) {
                    scene.dependencies.push(path.to_path_buf());
                }
                scene
            });
        }
        let (outer_deepest, outer_files) = (self.deepest, std::mem::take(&mut self.files));
        let (instances, wrap_start) = (self.instances, splits.cursor());
        self.deepest = level;
        self.active.push(canonical.clone());
        let result = parse_pass(path, values, splits, self, imported);
        self.active.pop();
        let depth = self.deepest - level;
        self.deepest = outer_deepest.max(self.deepest);
        let mut files = std::mem::replace(&mut self.files, outer_files);
        files.push(canonical);
        files.sort();
        files.dedup();
        self.files.extend(files.iter().cloned());
        let mut scene = result?;
        if imported {
            // Every instance fills its own canvas the same way, so the shared root does too.
            let size = (scene.width, scene.height);
            let root = &mut scene.elements[0];
            fill_canvas(root, size);
            root.layout.scene_root = true;
            let count = splits.cursor() - wrap_start;
            let compiled = Compiled {
                scene: scene.clone(),
                depth,
                instances: self.instances - instances,
                files,
                wraps: (wrap_start, count),
                breaks: splits.breaks_at(wrap_start, count),
                seen: Vec::new(),
                used: true,
            };
            self.cache.entries.insert(key, compiled);
        }
        Ok(scene)
    }

    fn count_instances(&mut self, count: usize) -> Result<()> {
        self.instances += count;
        if self.instances > MAX_INSTANCES {
            bail!(
                "data-composition-src supports at most {MAX_INSTANCES} instances per composition"
            );
        }
        Ok(())
    }

    /// A clone of the scene compiled earlier with the same values, checked as a fresh compilation
    /// would be; `None` when there is none, or when this parse gives its `data-hf-split-wrap`
    /// elements other breaks than it was built with (it is then compiled again).
    fn reuse(
        &mut self,
        key: &Key,
        level: usize,
        splits: &SplitPlan,
    ) -> Option<Result<Composition>> {
        let at = splits.cursor();
        let cached = lookup(&mut self.cache.entries, key)?;
        let (from, count) = cached.wraps;
        if count > 0 && cached.breaks != splits.breaks_at(at, count) {
            return None;
        }
        cached.used = true;
        let active = &self.active;
        if let Some(file) = cached.files.iter().find(|f| active.contains(f)) {
            let error = anyhow!(
                "cycle in data-composition-src: {} is already active",
                file.display()
            );
            return Some(Err(error));
        }
        if level + cached.depth > MAX_DEPTH {
            return Some(Err(anyhow!(
                "data-composition-src supports at most {MAX_DEPTH} nested levels"
            )));
        }
        let (depth, instances, files) = (cached.depth, cached.instances, cached.files.clone());
        let mut scene = cached.scene.clone();
        for path in &scene.dependencies {
            crate::parse::reads::note(path);
        }
        if let Err(error) = self.count_instances(instances) {
            return Some(Err(error));
        }
        self.deepest = self.deepest.max(level + depth);
        self.files.extend(files);
        splits.skip(count);
        // The first parse numbers wrapping elements in document order; renumber the clone as a
        // fresh compilation at this position would have been (only those elements are copied).
        if splits.unresolved() && at != from {
            for element in &mut scene.elements {
                if element.layout.split_wrap.is_some() {
                    if let Some(ordinal) = &mut element.layout.split_wrap {
                        *ordinal = *ordinal - from + at;
                    }
                }
            }
        }
        Some(Ok(scene))
    }
}

/// The compiled scene for `key`, or the one compiled by the other parse when it has no wrapping
/// text (both parses then build the same scene).
fn lookup<'m>(entries: &'m mut HashMap<Key, Compiled>, key: &Key) -> Option<&'m mut Compiled> {
    if entries.contains_key(key) {
        return entries.get_mut(key);
    }
    let other = (key.0.clone(), key.1.clone(), key.2.clone(), !key.3);
    entries
        .get_mut(&other)
        .filter(|compiled| compiled.wraps.1 == 0)
}
