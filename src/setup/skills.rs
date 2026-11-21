//! Agent Skills bundled into the binary, written to the project for Claude Code and skills.sh agents.

use super::Step;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Skill directories that agents scan, relative to the project root.
const TARGETS: [&str; 2] = [".claude/skills", ".agents/skills"];

/// Every bundled skill file as (path under `skills/`, contents).
pub const FILES: &[(&str, &str)] = &[
    (
        "aether-motion/SKILL.md",
        include_str!("../../skills/aether-motion/SKILL.md"),
    ),
    (
        "aether-motion/references/composition-contract.md",
        include_str!("../../skills/aether-motion/references/composition-contract.md"),
    ),
    (
        "aether-motion-3d/SKILL.md",
        include_str!("../../skills/aether-motion-3d/SKILL.md"),
    ),
    (
        "aether-motion-blocks/SKILL.md",
        include_str!("../../skills/aether-motion-blocks/SKILL.md"),
    ),
    (
        "aether-motion-cinematic/SKILL.md",
        include_str!("../../skills/aether-motion-cinematic/SKILL.md"),
    ),
    (
        "aether-motion-cli/SKILL.md",
        include_str!("../../skills/aether-motion-cli/SKILL.md"),
    ),
    (
        "aether-motion-import/SKILL.md",
        include_str!("../../skills/aether-motion-import/SKILL.md"),
    ),
    (
        "aether-motion-import/references/hyperframes.md",
        include_str!("../../skills/aether-motion-import/references/hyperframes.md"),
    ),
    (
        "aether-motion-import/references/remotion.md",
        include_str!("../../skills/aether-motion-import/references/remotion.md"),
    ),
    (
        "aether-motion-svg/SKILL.md",
        include_str!("../../skills/aether-motion-svg/SKILL.md"),
    ),
];

/// Writes the skills under each target; existing files that differ are kept unless `force`.
pub fn install(root: &Path, force: bool) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for target in TARGETS {
        let (mut created, mut updated, mut kept) = (0, 0, 0);
        for (rel, content) in FILES {
            let path = root.join(target).join(rel);
            match fs::read_to_string(&path) {
                Ok(existing) if existing == *content => continue,
                Ok(_) if !force => {
                    kept += 1;
                    continue;
                }
                Ok(_) => updated += 1,
                Err(_) => created += 1,
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, content)
                .with_context(|| format!("could not write {}", path.display()))?;
        }
        let label = format!("{target}/aether-motion*");
        steps.push(if created + updated > 0 {
            if created > 0 && updated == 0 {
                Step::Created(label)
            } else {
                Step::Updated(label)
            }
        } else if kept > 0 {
            Step::Skipped(label, "locally modified; use --force to overwrite".into())
        } else {
            Step::Unchanged(label)
        });
    }
    Ok(steps)
}

/// True when every bundled skill file is present in `root` for the given target.
pub fn installed(root: &Path, target: &str) -> bool {
    FILES
        .iter()
        .all(|(rel, _)| root.join(target).join(rel).exists())
}

pub fn targets() -> &'static [&'static str] {
    &TARGETS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walk(dir: &Path, out: &mut Vec<String>, base: &Path) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, out, base);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    #[test]
    fn table_covers_every_skill_file() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut on_disk = Vec::new();
        walk(&base, &mut on_disk, &base);
        on_disk.sort();
        let mut table: Vec<String> = FILES.iter().map(|(p, _)| p.to_string()).collect();
        table.sort();
        assert_eq!(on_disk, table);
    }

    #[test]
    fn install_is_idempotent_and_keeps_local_edits() {
        let dir = std::env::temp_dir().join(format!("aether-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(matches!(install(&dir, false).unwrap()[0], Step::Created(_)));
        assert!(matches!(
            install(&dir, false).unwrap()[0],
            Step::Unchanged(_)
        ));
        fs::write(dir.join(".claude/skills/aether-motion/SKILL.md"), "mine").unwrap();
        assert!(matches!(
            install(&dir, false).unwrap()[0],
            Step::Skipped(..)
        ));
        assert!(matches!(install(&dir, true).unwrap()[0], Step::Updated(_)));
        fs::remove_dir_all(&dir).unwrap();
    }
}
