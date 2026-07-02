use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::PatchStats;

pub fn patch_stats(patch: &str) -> PatchStats {
    let mut files = BTreeSet::new();
    let mut added_lines = 0;
    let mut removed_lines = 0;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            if let Some(path) = rest.split_whitespace().nth(1) {
                files.insert(path.trim_start_matches("b/").to_string());
            }
        } else if let Some(path) = line.strip_prefix("+++ b/") {
            files.insert(path.to_string());
        } else if line.starts_with('+') && !line.starts_with("+++") {
            added_lines += 1;
        } else if line.starts_with('-') && !line.starts_with("---") {
            removed_lines += 1;
        }
    }
    PatchStats {
        files: files.into_iter().collect(),
        changed_line_count: added_lines + removed_lines,
        added_lines,
        removed_lines,
    }
}

pub(crate) fn diff_without_unchanged_blocks(after: &str, before: &str) -> String {
    if before.trim().is_empty() || after.trim().is_empty() {
        return after.to_string();
    }

    let before_blocks = diff_blocks(before)
        .into_iter()
        .collect::<BTreeSet<String>>();
    if before_blocks.is_empty() {
        return after.to_string();
    }

    diff_blocks(after)
        .into_iter()
        .filter(|block| !before_blocks.contains(block))
        .collect::<Vec<_>>()
        .join("")
}

fn diff_blocks(patch: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();
    for line in patch.split_inclusive('\n') {
        if line.starts_with("diff --git ") && !current.is_empty() {
            blocks.push(std::mem::take(&mut current));
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

pub(crate) fn git_diff_with_untracked(root: &Path) -> Result<String> {
    let tracked = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "diff",
            "--no-ext-diff",
            "--binary",
            "--",
            ".",
            ":(exclude).mixmod",
            ":(exclude).codex",
            ":(exclude)task.json",
        ])
        .output()
        .with_context(|| format!("failed to run git diff in {}", root.display()))?;
    if !tracked.status.success() {
        bail!(
            "git diff failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&tracked.stderr).trim()
        );
    }
    let mut patch = String::from_utf8_lossy(&tracked.stdout).to_string();

    let untracked = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ])
        .output()
        .with_context(|| format!("failed to run git ls-files in {}", root.display()))?;
    if !untracked.status.success() {
        bail!(
            "git ls-files failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&untracked.stderr).trim()
        );
    }
    for raw in untracked.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let rel = String::from_utf8_lossy(raw)
            .trim_start_matches("./")
            .to_string();
        if rel.starts_with(".mixmod/")
            || rel.starts_with(".codex/")
            || rel == ".mixmod"
            || rel == "task.json"
        {
            continue;
        }
        let path = root.join(&rel);
        if path.is_dir() {
            continue;
        }
        let bytes = fs::read(&path).unwrap_or_default();
        if bytes.len() > 1_000_000 || bytes.contains(&0) {
            patch.push_str(&format!(
                "diff --git a/{rel} b/{rel}\nnew file mode 100644\nBinary files /dev/null and b/{rel} differ\n"
            ));
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        let line_count = text.lines().count().max(1);
        patch.push_str(&format!(
            "diff --git a/{rel} b/{rel}\nnew file mode 100644\n--- /dev/null\n+++ b/{rel}\n@@ -0,0 +1,{line_count} @@\n"
        ));
        if text.is_empty() {
            patch.push_str("+\n");
        } else {
            for line in text.split_inclusive('\n') {
                patch.push('+');
                patch.push_str(line);
                if !line.ends_with('\n') {
                    patch.push('\n');
                }
            }
        }
    }
    Ok(patch)
}
