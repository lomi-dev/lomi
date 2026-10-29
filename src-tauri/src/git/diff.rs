use super::{
    checked,
    history::{limit_patch, patch_bytes, CommitDiff, PATCH_LIMIT},
    relative, status,
};
use crate::files::inside;
use serde::Serialize;
use std::{fs, io::Read, path::Path};

#[derive(Serialize)]
pub struct FileDiff {
    #[serde(flatten)]
    diff: CommitDiff,
    notice: Option<String>,
}

pub(super) fn read(root: &str, path: &str, staged: bool) -> Result<FileDiff, String> {
    relative(path)?;
    let status = status(root)?.ok_or("This directory is not a Git repository.")?;
    let root = Path::new(&status.root);
    let change = status.changes.iter().find(|change| change.path == path);
    let untracked = change.is_some_and(|change| change.worktree == '?');
    let conflict = change.is_some_and(|change| {
        change.index == 'U'
            || change.worktree == 'U'
            || matches!((change.index, change.worktree), ('A', 'A') | ('D', 'D'))
    });
    if staged && conflict {
        return Err("Resolve this file's merge conflicts before comparing staged changes. Open the file to resolve them.".into());
    }
    let mut notice = conflict
        .then(|| "Unmerged file: comparing our index version with the file on disk.".into());
    if !untracked || staged {
        let mut args = vec![
            "--literal-pathspecs",
            "diff",
            "--patch",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=dirty",
            "--no-color",
            "--find-renames",
            "--unified=2147483647",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--output-indicator-new=+",
            "--output-indicator-old=-",
            "--output-indicator-context= ",
        ];
        if staged {
            args.push("--cached");
        } else if conflict {
            args.push("--ours");
        }
        args.extend(["--", path]);
        if staged {
            if let Some(original) = change.and_then(|change| change.original_path.as_deref()) {
                args.push(original);
            }
        }
        let bytes = patch_bytes(root, &args)?;
        let text = String::from_utf8_lossy(&bytes);
        if text.lines().any(|line| line.starts_with("Binary files ")) {
            return Ok(FileDiff {
                diff: limit_patch(&[]),
                notice: Some("Binary file: line-by-line changes are unavailable.".into()),
            });
        }
        if text.lines().any(|line| line.starts_with("@@ -")) || bytes.len() > PATCH_LIMIT {
            return Ok(FileDiff {
                diff: limit_patch(&bytes),
                notice,
            });
        }
        if change.is_some_and(|change| {
            if staged {
                change.index == 'D'
            } else {
                change.worktree == 'D'
            }
        }) {
            return Ok(FileDiff {
                diff: limit_patch(&[]),
                notice: Some("This empty file was deleted.".into()),
            });
        }
        let index = checked(
            root,
            &[
                "--literal-pathspecs",
                "ls-files",
                "--stage",
                "-z",
                "--",
                path,
            ],
        )?;
        if index
            .split(|byte| *byte == 0)
            .any(|entry| entry.starts_with(b"160000 "))
        {
            return Ok(FileDiff {
                diff: limit_patch(&[]),
                notice: Some("Submodule: open its repository to compare its working files. Gitlink changes are shown when its checked-out commit changes.".into()),
            });
        }
        notice = Some(if bytes.is_empty() {
            "No text changes. Showing the complete file.".into()
        } else {
            "Only file metadata changed. Showing the complete file.".into()
        });
    }
    let bytes = if staged {
        patch_bytes(root, &["show", &format!(":{path}")])?
    } else {
        let relative = Path::new(path);
        let parent = relative.parent().and_then(Path::to_str).unwrap_or("");
        let file = inside(&status.root, parent)?
            .join(relative.file_name().ok_or("Select a file to compare.")?);
        let metadata = fs::symlink_metadata(&file).map_err(|error| error.to_string())?;
        if metadata.is_symlink() {
            fs::read_link(&file)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .as_bytes()
                .to_vec()
        } else {
            if !metadata.is_file() {
                return Err("Only regular files can be displayed in this view.".into());
            }
            let mut bytes = Vec::new();
            fs::File::open(file)
                .map_err(|error| error.to_string())?
                .take((PATCH_LIMIT + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            bytes
        }
    };
    if bytes.contains(&0) {
        return Ok(FileDiff {
            diff: limit_patch(&[]),
            notice: Some("Binary or UTF-16 file: a UTF-8 line comparison is unavailable.".into()),
        });
    }
    let added = untracked && !staged;
    let text = String::from_utf8_lossy(&bytes);
    let count = text.lines().count();
    let mut patch = format!(
        "@@ -{},{} +1,{} @@\n",
        if added { 0 } else { 1 },
        if added { 0 } else { count },
        count
    );
    for line in text.split_inclusive('\n') {
        patch.push(if added { '+' } else { ' ' });
        patch.push_str(line);
        if !line.ends_with('\n') {
            patch.push_str("\n\\ No newline at end of file\n");
        }
    }
    if bytes.is_empty() {
        notice = Some("This file is empty.".into());
    }
    Ok(FileDiff {
        diff: limit_patch(patch.as_bytes()),
        notice,
    })
}
