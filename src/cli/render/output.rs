//! Output kinds and the collision policy shared by both render modes.
//!
//! The rule depends only on the file type and on whether the path was named, never on how a flag
//! was spelled: a PNG is overwritten (stills are re-rendered while iterating); an existing MP4 or
//! part file named with `-o`/`--output` needs `--force`; the default project export
//! (`out/<name>.mp4`, never named by the user) is replaced, as every re-render of a project does.
//! Replacement goes through [`Staged`], so a failure never destroys the previous file.

use crate::cli::args::{usage, UsageError};
use crate::encode::PART_EXTENSION;
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// What an output path produces, from its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Mp4,
    /// A part file of a distributed render.
    Part,
}

impl Kind {
    pub fn of(path: &Path) -> Result<Self> {
        match path.extension().and_then(|x| x.to_str()) {
            Some("png") => Ok(Kind::Png),
            Some("mp4") => Ok(Kind::Mp4),
            Some(PART_EXTENSION) => Ok(Kind::Part),
            _ => Err(usage(format!(
                "the output must end in .png, .mp4, or .{PART_EXTENSION} (got {})",
                path.display()
            ))),
        }
    }
}

/// What must happen to an existing file before rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    /// Nothing is there, or the renderer overwrites it (PNG).
    None,
    /// The file is removed first (`--force`, or the default project export).
    Replace,
}

/// Decides, without touching the file, what to do with an existing `path`; refuses a named MP4
/// or part file without `force`. Call it before the composition is parsed, so a refusal is fast.
pub fn plan(path: &Path, kind: Kind, named: bool, force: bool) -> Result<Collision> {
    if kind == Kind::Png || !path.exists() {
        return Ok(Collision::None);
    }
    if named && !force {
        return Err(
            UsageError::new(format!("{} already exists", path.display()))
                .with_hint("pass --force (-f) to replace it")
                .into(),
        );
    }
    Ok(Collision::Replace)
}

/// Where an export is written until it succeeds. A file being replaced is never removed first:
/// the new one is written to a hidden sibling and renamed over it only after it is complete, so a
/// failed render or encode leaves the previous output intact. Dropping an uncommitted `Staged`
/// removes the sibling.
pub struct Staged {
    target: PathBuf,
    write_to: PathBuf,
    committed: bool,
}

impl Staged {
    /// Creates the output folder and picks the path to write (the target itself when nothing is
    /// replaced).
    pub fn new(target: &Path, collision: Collision) -> Result<Self> {
        if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let write_to = match collision {
            Collision::None => target.to_path_buf(),
            Collision::Replace => sibling(target, "replace"),
        };
        Ok(Self {
            target: target.to_path_buf(),
            write_to,
            committed: false,
        })
    }

    /// The path the encoder writes.
    pub fn path(&self) -> &Path {
        &self.write_to
    }

    /// Moves the finished file into place (an atomic rename over the previous one).
    pub fn commit(mut self) -> Result<()> {
        if self.write_to != self.target {
            fs::rename(&self.write_to, &self.target)?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.committed && self.write_to != self.target {
            let _ = fs::remove_file(&self.write_to);
        }
    }
}

/// A hidden sibling of `path` for this process, keeping its extension (encoders and the PNG
/// writer pick the format from it): `dir/.name.mp4.aether-<pid>.<tag>.mp4`.
pub fn sibling(path: &Path, tag: &str) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(Default::default, |n| n.to_string_lossy());
    let ext = path
        .extension()
        .map_or_else(Default::default, |x| x.to_string_lossy());
    path.with_file_name(format!(".{name}.aether-{}.{tag}.{ext}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pngs_overwrite_and_named_videos_need_force() {
        let dir = std::env::temp_dir().join(format!("aether-output-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let file = |name: &str| -> PathBuf {
            let path = dir.join(name);
            fs::write(&path, b"x").unwrap();
            path
        };
        let png = file("a.png");
        let mp4 = file("a.mp4");
        let part = file("a.aseg");
        assert_eq!(plan(&png, Kind::Png, true, false).unwrap(), Collision::None);
        let refused = plan(&mp4, Kind::Mp4, true, false).unwrap_err();
        assert!(refused.downcast_ref::<UsageError>().is_some());
        assert!(plan(&part, Kind::Part, true, false).is_err());
        assert_eq!(
            plan(&mp4, Kind::Mp4, true, true).unwrap(),
            Collision::Replace
        );
        assert_eq!(
            plan(&mp4, Kind::Mp4, false, false).unwrap(),
            Collision::Replace
        );
        assert_eq!(
            plan(&dir.join("new.mp4"), Kind::Mp4, true, false).unwrap(),
            Collision::None
        );
        // A replacement that fails leaves the old file; one that commits swaps it in.
        let staged = Staged::new(&mp4, Collision::Replace).unwrap();
        fs::write(staged.path(), b"half").unwrap();
        drop(staged);
        assert_eq!(fs::read(&mp4).unwrap(), b"x");
        let staged = Staged::new(&mp4, Collision::Replace).unwrap();
        fs::write(staged.path(), b"new").unwrap();
        staged.commit().unwrap();
        assert_eq!(fs::read(&mp4).unwrap(), b"new");
        assert_eq!(
            fs::read_dir(&dir).unwrap().count(),
            3,
            "a staging file was left behind"
        );
        assert_eq!(Kind::of(Path::new("x.aseg")).unwrap(), Kind::Part);
        assert!(Kind::of(Path::new("x.gif")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
