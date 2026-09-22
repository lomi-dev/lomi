use super::{directory, inside, main_window};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};
use tauri::{Manager, Window};
use tauri_plugin_opener::OpenerExt;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Operation {
    NewFile {
        name: String,
    },
    NewFolder {
        name: String,
    },
    Rename {
        name: String,
    },
    Copy {
        #[serde(rename = "sourceRoot")]
        source_root: String,
        source: String,
    },
    Move {
        #[serde(rename = "sourceRoot")]
        source_root: String,
        source: String,
    },
    Duplicate,
    Trash,
    Delete,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
}

fn name(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value == ".git"
        || value.contains(['/', '\\', '\0'])
        || Path::new(value).components().count() != 1
        || !matches!(
            Path::new(value).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err("Enter a single file or folder name (other than .git).".into());
    }
    Ok(())
}

fn validate_relative(relative: &str) -> Result<(), String> {
    if Path::new(relative).components().any(|part| {
        !matches!(part, Component::Normal(_) | Component::CurDir) || part.as_os_str() == ".git"
    }) {
        return Err("The path must stay inside the project and outside .git.".into());
    }
    Ok(())
}

// Resolve the parent, retaining the leaf so rename/delete operate on a symlink itself.
pub(super) fn entry_path(root: &str, relative: &str) -> Result<PathBuf, String> {
    validate_relative(relative)?;
    if relative.is_empty() {
        return directory(root);
    }
    let relative = Path::new(relative);
    let parent = relative
        .parent()
        .and_then(Path::to_str)
        .ok_or("Invalid parent path.")?;
    let leaf = relative.file_name().ok_or("Choose a file or folder.")?;
    let path = inside(root, parent)?.join(leaf);
    fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    Ok(path)
}

fn unused(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(_) => Err("A file or folder with that name already exists.".into()),
        Err(error) => Err(error.to_string()),
    }
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|error| error.to_string())?;
    if metadata.is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
        return Err("Copying symbolic links and special files is not supported.".into());
    }
    if metadata.is_file() {
        let mut from = fs::File::open(source).map_err(|error| error.to_string())?;
        let mut to = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|error| error.to_string())?;
        std::io::copy(&mut from, &mut to).map_err(|error| error.to_string())?;
        to.set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
    } else {
        fs::create_dir(target).map_err(|error| error.to_string())?;
        for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        fs::set_permissions(target, metadata.permissions()).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn copy(source: &Path, target: &Path) -> Result<(), String> {
    unused(target)?;
    if target.starts_with(source) {
        return Err("A folder cannot be copied into itself.".into());
    }
    // Build the copy beside its destination. Failed copies never leave a partial destination.
    let staging = tempfile::tempdir_in(target.parent().ok_or("Invalid destination.")?)
        .map_err(|error| error.to_string())?;
    let staged = staging.path().join("copy");
    copy_tree(source, &staged)?;
    unused(target)?;
    fs::rename(staged, target).map_err(|error| error.to_string())
}

fn move_entry(source: &Path, target: &Path) -> Result<(), String> {
    unused(target)?;
    if target.starts_with(source) {
        return Err("A folder cannot be moved into itself.".into());
    }
    fs::rename(source, target).map_err(|error| error.to_string())
}

fn perform(root: &str, relative: &str, operation: Operation) -> Result<FileChange, String> {
    let source = entry_path(root, relative)?;
    let mut result = FileChange {
        old_path: None,
        new_path: None,
    };
    let destination = match operation {
        Operation::NewFile { ref name } | Operation::NewFolder { ref name } => {
            self::name(name)?;
            let target = inside(root, relative)?.join(name);
            if matches!(operation, Operation::NewFolder { .. }) {
                fs::create_dir(&target).map_err(|error| error.to_string())?;
            } else {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)
                    .map_err(|error| error.to_string())?;
            }
            target
        }
        Operation::Rename { name: value } => {
            name(&value)?;
            let target = source
                .parent()
                .ok_or("This folder cannot be renamed.")?
                .join(value);
            if target != source {
                move_entry(&source, &target)?;
            }
            result.old_path = Some(source.to_string_lossy().into_owned());
            target
        }
        Operation::Copy {
            ref source_root,
            source: ref relative_source,
        }
        | Operation::Move {
            ref source_root,
            source: ref relative_source,
        } => {
            if relative_source.is_empty() {
                return Err("Copy or move items inside the project.".into());
            }
            let original = entry_path(source_root, relative_source)?;
            let target =
                inside(root, relative)?.join(original.file_name().ok_or("Invalid source.")?);
            if matches!(operation, Operation::Move { .. }) {
                move_entry(&original, &target)?;
                result.old_path = Some(original.to_string_lossy().into_owned());
            } else {
                copy(&original, &target)?;
            }
            target
        }
        Operation::Duplicate => {
            if relative.is_empty() {
                return Err("Duplicate items inside the project.".into());
            }
            let parent = source.parent().ok_or("Invalid parent.")?;
            let stem = if source.is_dir() {
                source.file_name()
            } else {
                source.file_stem()
            }
            .and_then(|value| value.to_str())
            .ok_or("Invalid filename.")?;
            let extension = if source.is_dir() {
                None
            } else {
                source.extension().and_then(|value| value.to_str())
            };
            let mut number = 1;
            let target = loop {
                let suffix = if number == 1 {
                    " copy".to_owned()
                } else {
                    format!(" copy {number}")
                };
                let candidate = parent.join(format!(
                    "{stem}{suffix}{}",
                    extension
                        .map(|extension| format!(".{extension}"))
                        .unwrap_or_default()
                ));
                if !candidate.try_exists().map_err(|error| error.to_string())?
                    && fs::symlink_metadata(&candidate).is_err()
                {
                    break candidate;
                }
                number += 1;
            };
            copy(&source, &target)?;
            target
        }
        Operation::Trash | Operation::Delete => {
            if source.parent().is_none() {
                return Err("The filesystem root cannot be deleted.".into());
            }
            if matches!(operation, Operation::Trash) {
                trash::delete(&source).map_err(|error| error.to_string())?;
            } else {
                let metadata = fs::symlink_metadata(&source).map_err(|error| error.to_string())?;
                if metadata.is_dir() && !metadata.is_symlink() {
                    fs::remove_dir_all(&source)
                } else {
                    fs::remove_file(&source)
                }
                .map_err(|error| error.to_string())?;
            }
            return Ok(FileChange {
                old_path: Some(source.to_string_lossy().into_owned()),
                new_path: None,
            });
        }
    };
    result.new_path = Some(destination.to_string_lossy().into_owned());
    Ok(result)
}

#[tauri::command]
pub async fn resolve_project_entry(
    window: Window,
    root: String,
    relative: String,
) -> Result<String, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        entry_path(&root, &relative).map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn file_operation(
    window: Window,
    root: String,
    relative: String,
    operation: Operation,
    expected_path: Option<String>,
) -> Result<FileChange, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        if let Some(expected) = expected_path {
            if entry_path(&root, &relative)? != Path::new(&expected) {
                return Err("The selected path changed. Please try the operation again.".into());
            }
        }
        perform(&root, &relative, operation)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn open_project_item(
    window: Window,
    root: String,
    relative: String,
    reveal: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let path = inside(&root, &relative)?;
        if reveal {
            window.opener().reveal_item_in_dir(path)
        } else {
            window
                .opener()
                .open_path(path.to_string_lossy(), None::<&str>)
        }
        .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_pull(window: Window, root: String, rebase: bool) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        // Hold the editor write lock through pull to serialize worktree mutations.
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::git::pull(&root, rebase)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_discard(
    window: Window,
    root: String,
    change: crate::git::Change,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        crate::git::discard(&root, &change)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn ignore_project_item(
    window: Window,
    root: String,
    relative: String,
    local: bool,
) -> Result<(), String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = window.state::<super::editor::EditorFiles>();
        let _lock = state
            .writes
            .lock()
            .map_err(|_| "File writes are unavailable.")?;
        ignore_item(&root, &relative, local)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn ignore_item(root: &str, relative: &str, local: bool) -> Result<(), String> {
    let path = entry_path(root, relative)?;
    let _operation = crate::git::mutation_guard(root)?;
    let repository = crate::git::repository(root)?;
    let relative = path
        .strip_prefix(&repository)
        .map_err(|_| "The item is outside the repository.")?
        .to_str()
        .ok_or("Invalid filename.")?;
    if relative.is_empty() || relative.contains(['\r', '\n']) {
        return Err("This path cannot be added to an ignore file.".into());
    }
    let mut pattern = String::from("/");
    for ch in relative.chars() {
        if cfg!(windows) && ch == '\\' {
            pattern.push('/');
            continue;
        }
        if matches!(ch, '\\' | '*' | '?' | '[' | ']' | '!' | '#' | ' ') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    if path.is_dir() {
        pattern.push('/');
    }
    let target = if local {
        let bytes = crate::git::checked_repository(
            &repository,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "info/exclude",
            ],
        )?;
        PathBuf::from(
            String::from_utf8(bytes)
                .map_err(|_| "Invalid Git path.")?
                .trim_end_matches('\n'),
        )
    } else {
        repository.join(".gitignore")
    };
    if fs::symlink_metadata(&target).is_ok_and(|metadata| metadata.is_symlink()) {
        return Err("The ignore file must not be a symbolic link.".into());
    }
    let previous = match fs::read_to_string(&target) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    if previous.lines().any(|line| line == pattern) {
        return Ok(());
    }
    let parent = target.parent().ok_or("Invalid ignore path.")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| error.to_string())?;
    if let Ok(metadata) = fs::metadata(&target) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .map_err(|error| error.to_string())?;
    }
    temporary
        .write_all(previous.as_bytes())
        .map_err(|error| error.to_string())?;
    if !previous.is_empty() && !previous.ends_with('\n') {
        temporary
            .write_all(b"\n")
            .map_err(|error| error.to_string())?;
    }
    writeln!(temporary, "{pattern}").map_err(|error| error.to_string())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| error.to_string())?;
    temporary
        .persist(target)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_rename_and_cut_move_preserve_contents() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("file.txt"), "contents").unwrap();
        let renamed = perform(
            root.to_str().unwrap(),
            "",
            Operation::Rename {
                name: "renamed".into(),
            },
        )
        .unwrap();
        let root = renamed.new_path.unwrap();
        perform(
            &root,
            "",
            Operation::NewFolder {
                name: "nested".into(),
            },
        )
        .unwrap();
        let moved = perform(
            &root,
            "nested",
            Operation::Move {
                source_root: root.clone(),
                source: "file.txt".into(),
            },
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(moved.new_path.unwrap()).unwrap(),
            "contents"
        );
        assert!(!Path::new(&root).join("file.txt").exists());
    }

    #[test]
    fn ignore_rejects_a_missing_nested_repository_without_writing_to_its_parent() {
        let parent = tempfile::tempdir().unwrap();
        crate::git::checked(parent.path(), &["init", "-b", "main"]).unwrap();
        let child = parent.path().join("child");
        fs::create_dir(&child).unwrap();
        crate::git::checked(&child, &["init", "-b", "main"]).unwrap();
        fs::write(child.join("file.txt"), "keep").unwrap();
        fs::remove_dir_all(child.join(".git")).unwrap();
        assert!(ignore_item(child.to_str().unwrap(), "file.txt", false).is_err());
        assert!(!parent.path().join(".gitignore").exists());
        assert!(!child.join(".gitignore").exists());
    }

    #[test]
    fn ignore_rules_are_literal_and_preserve_existing_contents() {
        let root = tempfile::tempdir().unwrap();
        crate::git::checked(root.path(), &["init"]).unwrap();
        let path = root.path().to_str().unwrap();
        #[cfg(not(windows))]
        let filename = "odd [x]* #.txt";
        #[cfg(windows)]
        let filename = "odd [x] #.txt";
        fs::write(root.path().join(filename), "contents").unwrap();
        fs::write(root.path().join(".gitignore"), "# keep this comment").unwrap();
        ignore_item(path, filename, false).unwrap();
        let first = fs::read_to_string(root.path().join(".gitignore")).unwrap();
        assert!(first.starts_with("# keep this comment\n"));
        ignore_item(path, filename, false).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join(".gitignore")).unwrap(),
            first
        );
        crate::git::checked(root.path(), &["check-ignore", "--", filename]).unwrap();
        assert!(
            crate::git::checked(root.path(), &["check-ignore", "--", "odd x123 #.txt"]).is_err()
        );
        assert!(crate::git::checked(root.path(), &["check-ignore", "--", "odd x #.txt"]).is_err());
        fs::create_dir(root.path().join("local")).unwrap();
        ignore_item(path, "local", true).unwrap();
        assert!(fs::read_to_string(root.path().join(".git/info/exclude"))
            .unwrap()
            .contains("/local/"));
        assert_eq!(
            fs::read_to_string(root.path().join(".gitignore")).unwrap(),
            first
        );
    }

    #[test]
    fn creates_renames_copies_and_deletes_without_overwriting() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_str().unwrap();
        perform(path, "", Operation::NewFolder { name: "src".into() }).unwrap();
        perform(
            path,
            "src",
            Operation::NewFile {
                name: "a.txt".into(),
            },
        )
        .unwrap();
        fs::write(root.path().join("src/a.txt"), "keep me").unwrap();
        assert!(perform(
            path,
            "src",
            Operation::NewFile {
                name: "a.txt".into()
            }
        )
        .is_err());
        perform(path, "src/a.txt", Operation::Duplicate).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join("src/a copy.txt")).unwrap(),
            "keep me"
        );
        assert!(perform(
            path,
            "src/a.txt",
            Operation::Rename {
                name: "a copy.txt".into()
            }
        )
        .is_err());
        perform(
            path,
            "src",
            Operation::Rename {
                name: "code".into(),
            },
        )
        .unwrap();
        assert!(root.path().join("code/a.txt").is_file());
        assert!(perform(
            path,
            "code",
            Operation::Copy {
                source_root: path.into(),
                source: "code".into()
            }
        )
        .is_err());
        perform(path, "code", Operation::Delete).unwrap();
        assert!(!root.path().join("code").exists());
        assert!(perform(
            path,
            "",
            Operation::NewFile {
                name: "../escape".into()
            }
        )
        .is_err());
        assert!(entry_path(path, "../escape").is_err());
        assert!(entry_path(path, ".git/config").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deletes_link_itself_and_rejects_escaping_parents_and_partial_copies() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("safe"), "safe").unwrap();
        symlink(outside.path(), root.path().join("link")).unwrap();
        let path = root.path().to_str().unwrap();
        assert!(perform(path, "link/safe", Operation::Delete).is_err());
        assert!(perform(path, "link", Operation::Duplicate).is_err());
        assert!(!root.path().join("link copy").exists());
        perform(path, "link", Operation::Delete).unwrap();
        assert!(outside.path().join("safe").is_file());
    }
}
