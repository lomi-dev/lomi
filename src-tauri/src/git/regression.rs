use super::test_checked as checked;
use super::*;

fn initialize(root: &Path) {
    fs::create_dir_all(root).unwrap();
    checked(root, &["init", "-b", "main"]).unwrap();
    checked(root, &["config", "core.autocrlf", "false"]).unwrap();
    checked(root, &["config", "user.name", "Review Test"]).unwrap();
    checked(root, &["config", "user.email", "review@example.test"]).unwrap();
    checked(root, &["config", "commit.gpgsign", "false"]).unwrap();
    checked(root, &["config", "core.hooksPath", ".git/disabled-hooks"]).unwrap();
    fs::write(root.join("file.txt"), "original\n").unwrap();
    checked(root, &["add", "file.txt"]).unwrap();
    checked(root, &["commit", "-m", "initial"]).unwrap();
}

#[test]
fn discard_must_not_fall_back_to_parent_after_nested_repository_disappears() {
    let parent = tempfile::tempdir().unwrap();
    initialize(parent.path());
    let nested = parent.path().join("nested");
    initialize(&nested);
    fs::write(parent.path().join("file.txt"), "keep parent changes\n").unwrap();
    fs::write(nested.join("file.txt"), "nested changes\n").unwrap();
    let expected = status(nested.to_str().unwrap())
        .unwrap()
        .unwrap()
        .changes
        .remove(0);
    fs::remove_dir_all(nested.join(".git")).unwrap();
    let result = discard(nested.to_str().unwrap(), &expected);
    assert_eq!(
        fs::read_to_string(parent.path().join("file.txt")).unwrap(),
        "keep parent changes\n"
    );
    assert!(result.is_err());
}

#[test]
fn one_corrupt_repository_must_not_hide_healthy_siblings() {
    let project = tempfile::tempdir().unwrap();
    initialize(&project.path().join("healthy"));
    let broken = project.path().join("broken");
    fs::create_dir(&broken).unwrap();
    fs::write(broken.join(".git"), "invalid gitfile\n").unwrap();
    let result = repositories(project.path().to_str().unwrap(), None).unwrap();
    assert_eq!(result.repositories.len(), 1);
    assert!(result.repositories[0].root.ends_with("healthy"));
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].root.ends_with("broken"));
}

#[test]
fn repository_scan_respects_its_declared_repository_budget() {
    let project = tempfile::tempdir().unwrap();
    for index in 0..66 {
        let path = project.path().join(format!("repo-{index}"));
        fs::create_dir(&path).unwrap();
        checked(&path, &["init", "-b", "main"]).unwrap();
        checked(&path, &["config", "core.autocrlf", "false"]).unwrap();
    }
    let found = repositories(project.path().to_str().unwrap(), None).unwrap();
    assert_eq!(found.repositories.len(), 64);
    assert!(found.limited);
}

#[test]
fn mutations_reject_a_missing_repository_and_git_cannot_discover_the_parent() {
    let parent = tempfile::tempdir().unwrap();
    initialize(parent.path());
    let nested = parent.path().join("nested");
    initialize(&nested);
    fs::remove_dir_all(nested.join(".git")).unwrap();
    let path = nested.to_str().unwrap();
    assert!(repository(path).is_err());
    assert!(change_index(path, &["file.txt".into()], true).is_err());
    assert!(fetch(path, None).is_err());
    assert!(pull(path, false).is_err());
    assert!(push(path, None, false).is_err());
    assert!(commit(path, "Wrong repository").is_err());
    assert!(checked_repository(&nested, &["rev-parse", "--show-toplevel"]).is_err());
}

#[test]
fn linked_worktrees_remain_valid_exact_repositories() {
    let parent = tempfile::tempdir().unwrap();
    initialize(parent.path());
    let other = tempfile::tempdir().unwrap();
    let worktree = other.path().join("linked");
    checked(
        parent.path(),
        &[
            "worktree",
            "add",
            "-b",
            "linked",
            worktree.to_str().unwrap(),
        ],
    )
    .unwrap();
    fs::write(worktree.join("file.txt"), "worktree changes").unwrap();
    change_index(worktree.to_str().unwrap(), &["file.txt".into()], true).unwrap();
    let scan = repositories(other.path().to_str().unwrap(), None).unwrap();
    assert_eq!(scan.repositories.len(), 1);
    assert_eq!(scan.repositories[0].changes[0].index, 'M');
    assert!(scan.errors.is_empty());
}

#[test]
fn concurrent_mutations_are_rejected_per_repository_and_released_on_drop() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    initialize(first.path());
    initialize(second.path());
    let path = first.path().to_str().unwrap();
    let lock = mutation_guard(path).unwrap();
    assert!(change_index(path, &["file.txt".into()], true).is_err());
    change_index(second.path().to_str().unwrap(), &["file.txt".into()], true).unwrap();
    drop(lock);
    change_index(path, &["file.txt".into()], true).unwrap();
}

#[test]
fn status_refresh_does_not_rescan_the_project_or_retarget_missing_roots() {
    let project = tempfile::tempdir().unwrap();
    initialize(project.path());
    let first = project.path().join("first");
    let second = project.path().join("second");
    initialize(&first);
    initialize(&second);
    let roots = vec![first.to_string_lossy().into_owned()];
    let scan = repositories(project.path().to_str().unwrap(), Some(&roots)).unwrap();
    assert_eq!(scan.repositories.len(), 1);
    assert!(scan.repositories[0].root.ends_with("first"));
    fs::remove_dir_all(first.join(".git")).unwrap();
    let scan = repositories(project.path().to_str().unwrap(), Some(&roots)).unwrap();
    assert!(scan.repositories.is_empty());
    assert_eq!(scan.errors.len(), 1);
}

#[test]
fn commits_preserve_the_exact_message_including_whitespace() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path());
    fs::write(root.path().join("file.txt"), "updated").unwrap();
    change_index(root.path().to_str().unwrap(), &["file.txt".into()], true).unwrap();
    let message = "  Subject 🦀  \n\n  Body with trailing spaces  \n\n";
    commit(root.path().to_str().unwrap(), message).unwrap();
    let object = checked(root.path(), &["cat-file", "commit", "HEAD"]).unwrap();
    let body = object
        .windows(2)
        .position(|bytes| bytes == b"\n\n")
        .unwrap()
        + 2;
    assert_eq!(&object[body..], message.as_bytes());
}

#[test]
fn automatic_status_cannot_execute_repository_helpers_but_explicit_stage_can() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    initialize(root.path());
    let marker = root.path().join("helper-executed");
    let helper = root.path().join(".git/marker-helper");
    // The fixture only records invocation and preserves the filter input.
    fs::write(&helper, "#!/bin/sh\ntouch helper-executed\ncat\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    #[cfg(unix)]
    let helper_command = helper.to_str().unwrap().to_owned();
    #[cfg(windows)]
    let helper_command = format!("sh \"{}\"", helper.to_str().unwrap().replace('\\', "/"));
    checked(root.path(), &["config", "core.fsmonitor", &helper_command]).unwrap();
    fs::write(root.path().join("file.txt"), "changed\n").unwrap();
    let observed = status(root.path().to_str().unwrap()).unwrap().unwrap();
    assert_eq!(observed.changes.len(), 1);
    assert!(!marker.exists());
    let scan = repositories(root.path().to_str().unwrap(), None).unwrap();
    assert_eq!(scan.repositories.len(), 1);
    assert!(!marker.exists());
    checked(root.path(), &["config", "core.fsmonitor", "false"]).unwrap();
    fs::write(
        root.path().join(".gitattributes"),
        "file.txt filter=marker\n",
    )
    .unwrap();
    for key in ["filter.marker.clean", "filter.marker.process"] {
        checked(root.path(), &["config", key, &helper_command]).unwrap();
        let _ = status(root.path().to_str().unwrap());
        assert!(!marker.exists(), "automatic status ran {key}");
        let scan = repositories(root.path().to_str().unwrap(), None).unwrap();
        assert!(!marker.exists(), "repository discovery ran {key}");
        assert!(!scan.repositories.is_empty() || !scan.errors.is_empty());
        checked(root.path(), &["config", "--unset", key]).unwrap();
    }
    checked(
        root.path(),
        &["config", "filter.marker.clean", &helper_command],
    )
    .unwrap();
    change_index(root.path().to_str().unwrap(), &["file.txt".into()], true).unwrap();
    assert!(
        marker.exists(),
        "explicit stage did not retain its configured clean filter"
    );
}

#[test]
fn ordinary_observation_handles_large_indexes_without_worker_processes() {
    let root = tempfile::tempdir().unwrap();
    checked(root.path(), &["init", "-b", "main"]).unwrap();
    checked(root.path(), &["config", "core.autocrlf", "false"]).unwrap();
    checked(root.path(), &["config", "core.preloadIndex", "true"]).unwrap();
    for index in 0..2000 {
        fs::write(root.path().join(format!("file-{index}.txt")), "tracked\n").unwrap();
    }
    checked(root.path(), &["add", "--all"]).unwrap();
    let observed = status(root.path().to_str().unwrap()).unwrap().unwrap();
    assert_eq!(observed.changes.len(), 2000);
}

#[test]
fn guarded_submodules_preserve_clean_dirty_untracked_and_changed_head_status() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let nested_source = tempfile::tempdir().unwrap();
    initialize(root.path());
    initialize(source.path());
    initialize(nested_source.path());
    checked(
        root.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "-c",
            "core.autocrlf=false",
            "submodule",
            "add",
            "--",
            source.path().to_str().unwrap(),
            "sub",
        ],
    )
    .unwrap();
    let child = root.path().join("sub");
    for (key, value) in [
        ("user.name", "Submodule Test"),
        ("user.email", "submodule@example.test"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", ".git/disabled-hooks"),
        ("core.autocrlf", "false"),
    ] {
        checked(&child, &["config", key, value]).unwrap();
    }
    checked(
        &child,
        &[
            "-c",
            "protocol.file.allow=always",
            "-c",
            "core.autocrlf=false",
            "submodule",
            "add",
            "--",
            nested_source.path().to_str().unwrap(),
            "nested",
        ],
    )
    .unwrap();
    checked(&child.join("nested"), &["config", "core.autocrlf", "false"]).unwrap();
    checked(&child, &["commit", "-am", "add nested submodule"]).unwrap();
    checked(root.path(), &["add", "--all"]).unwrap();
    checked(root.path(), &["commit", "-m", "add submodule"]).unwrap();
    checked(root.path(), &["config", "diff.submodule", "diff"]).unwrap();
    let commit_patch = history::patch_bytes(
        root.path(),
        &["diff-tree", "--root", "--patch", "-r", "HEAD"],
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&commit_patch).contains("Subproject commit"));
    let observed = || status(root.path().to_str().unwrap()).unwrap().unwrap();
    assert!(observed().changes.is_empty());
    fs::write(child.join("file.txt"), "dirty\n").unwrap();
    assert_eq!(observed().changes[0].path, "sub");
    assert_eq!(observed().changes[0].worktree, 'M');
    assert!(diff::read(root.path().to_str().unwrap(), "file.txt", false).is_ok());
    let submodule_diff =
        serde_json::to_value(diff::read(root.path().to_str().unwrap(), "sub", false).unwrap())
            .unwrap();
    assert!(submodule_diff["notice"]
        .as_str()
        .unwrap()
        .contains("Submodule"));
    checked(&child, &["checkout", "--", "file.txt"]).unwrap();
    fs::write(child.join("untracked.txt"), "untracked\n").unwrap();
    assert_eq!(observed().changes[0].worktree, 'M');
    fs::remove_file(child.join("untracked.txt")).unwrap();
    let nested = child.join("nested");
    fs::write(nested.join("file.txt"), "nested dirty\n").unwrap();
    assert_eq!(observed().changes[0].worktree, 'M');
    checked(&nested, &["checkout", "--", "file.txt"]).unwrap();
    assert!(observed().changes.is_empty());
    fs::write(child.join("file.txt"), "new child commit\n").unwrap();
    checked(&child, &["commit", "-am", "advance child HEAD"]).unwrap();
    assert_eq!(observed().changes[0].worktree, 'M');
    let changed_head_diff =
        serde_json::to_value(diff::read(root.path().to_str().unwrap(), "sub", false).unwrap())
            .unwrap();
    assert!(changed_head_diff["patch"]
        .as_str()
        .unwrap()
        .contains("Subproject commit"));
    checked(root.path(), &["add", "--", "sub"]).unwrap();
    assert_eq!(observed().changes[0].index, 'M');
    assert_eq!(observed().changes[0].worktree, ' ');
    let helper = root.path().join(".git/submodule-marker-helper");
    fs::write(&helper, "#!/bin/sh\ntouch helper-executed\ncat\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(unix)]
    let helper_command = helper.to_str().unwrap().to_owned();
    #[cfg(windows)]
    let helper_command = format!("sh \"{}\"", helper.to_str().unwrap().replace('\\', "/"));
    checked(&child, &["config", "core.fsmonitor", &helper_command]).unwrap();
    observed();
    assert!(!child.join("helper-executed").exists());
    checked(&child, &["config", "core.fsmonitor", "false"]).unwrap();
    fs::write(child.join(".gitattributes"), "file.txt filter=marker\n").unwrap();
    fs::write(child.join("file.txt"), "child filter input\n").unwrap();
    for key in ["filter.marker.clean", "filter.marker.process"] {
        checked(&child, &["config", key, &helper_command]).unwrap();
        let _ = status(root.path().to_str().unwrap());
        assert!(
            !child.join("helper-executed").exists(),
            "submodule observation executed {key}"
        );
        checked(&child, &["config", "--unset", key]).unwrap();
    }
}

#[test]
fn submodule_observation_limits_are_reported_instead_of_clean_status() {
    let root = tempfile::tempdir().unwrap();
    initialize(root.path());
    let mut budget = StatusBudget::default();
    assert!(
        observed_status(root.path().to_str().unwrap(), &mut budget, 9, true)
            .err()
            .unwrap()
            .contains("limit")
    );
}

#[test]
fn recursive_submodule_status_matches_effective_ignore_modes() {
    let root = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    initialize(root.path());
    initialize(source.path());
    checked(
        root.path(),
        &[
            "-c",
            "protocol.file.allow=always",
            "-c",
            "core.autocrlf=false",
            "submodule",
            "add",
            "--name",
            "custom.name",
            "--",
            source.path().to_str().unwrap(),
            "sub folder",
        ],
    )
    .unwrap();
    let child = root.path().join("sub folder");
    for (key, value) in [
        ("user.name", "Ignore Test"),
        ("user.email", "ignore@example.test"),
        ("commit.gpgsign", "false"),
        ("core.hooksPath", ".git/disabled-hooks"),
        ("core.autocrlf", "false"),
    ] {
        checked(&child, &["config", key, value]).unwrap();
    }
    let initial = String::from_utf8(checked(&child, &["rev-parse", "HEAD"]).unwrap()).unwrap();
    fs::write(child.join("file.txt"), "advanced\n").unwrap();
    checked(&child, &["commit", "-am", "advance submodule"]).unwrap();
    let advanced = String::from_utf8(checked(&child, &["rev-parse", "HEAD"]).unwrap()).unwrap();
    checked(&child, &["checkout", initial.trim()]).unwrap();
    checked(root.path(), &["add", "--all"]).unwrap();
    checked(root.path(), &["commit", "-m", "add named submodule"]).unwrap();
    let assert_status = |mode: &str, state: &str| {
        let expected = parse_status(
            &checked(
                root.path(),
                &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
            )
            .unwrap(),
        )
        .into_iter()
        .filter(|change| change.path == "sub folder")
        .collect::<Vec<_>>();
        let actual = status(root.path().to_str().unwrap())
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .filter(|change| change.path == "sub folder")
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{mode}: {state}");
    };
    for mode in ["none", "untracked", "dirty", "all"] {
        checked(
            root.path(),
            &[
                "config",
                "--file",
                ".gitmodules",
                "submodule.custom.name.ignore",
                mode,
            ],
        )
        .unwrap();
        assert_status(mode, "clean");
        fs::write(child.join("file.txt"), "tracked dirtiness\n").unwrap();
        assert_status(mode, "tracked");
        // Local repository configuration must override the .gitmodules default.
        let override_mode = if mode == "all" { "none" } else { "all" };
        checked(
            root.path(),
            &["config", "submodule.custom.name.ignore", override_mode],
        )
        .unwrap();
        assert_status(override_mode, "local override tracked");
        checked(
            root.path(),
            &["config", "--unset", "submodule.custom.name.ignore"],
        )
        .unwrap();
        checked(&child, &["checkout", "--", "file.txt"]).unwrap();
        fs::write(child.join("untracked.txt"), "untracked\n").unwrap();
        assert_status(mode, "untracked");
        fs::remove_file(child.join("untracked.txt")).unwrap();
        checked(&child, &["checkout", advanced.trim()]).unwrap();
        assert_status(mode, "changed HEAD");
        checked(root.path(), &["add", "--", "sub folder"]).unwrap();
        assert_status(mode, "staged gitlink");
        checked(root.path(), &["reset", "--", "sub folder"]).unwrap();
        checked(&child, &["checkout", initial.trim()]).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn fifo_gitmodules_do_not_block_status_and_are_rejected_when_gitlinks_need_config() {
    use std::{
        ffi::CString,
        io::Write,
        os::unix::{ffi::OsStrExt, fs::OpenOptionsExt},
        sync::mpsc,
        time::Duration,
    };
    let root = tempfile::tempdir().unwrap();
    initialize(root.path());
    let path = root.path().join(".gitmodules");
    let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let bounded_status = || {
        let (sender, receiver) = mpsc::channel();
        let directory = root.path().to_str().unwrap().to_owned();
        let task = std::thread::spawn(move || {
            sender.send(status(&directory).map(|_| ())).unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(5));
        if result.is_err() {
            // Unblock a regressed pathname reader before failing the test.
            if let Ok(mut writer) = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                let _ = writer.write_all(b"[submodule \"sub\"]\npath = sub\n");
            }
        }
        assert!(result.is_ok(), "status blocked on .gitmodules FIFO");
        task.join().unwrap();
        result.unwrap()
    };
    assert!(bounded_status().is_ok());
    let head = String::from_utf8(checked(root.path(), &["rev-parse", "HEAD"]).unwrap()).unwrap();
    checked(
        root.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},sub", head.trim()),
        ],
    )
    .unwrap();
    assert!(bounded_status().unwrap_err().contains("regular file"));
}

#[cfg(unix)]
#[test]
fn gitmodules_descriptor_snapshot_rejects_replacements_and_is_not_reopened_by_git() {
    use std::{
        ffi::CString,
        os::unix::{ffi::OsStrExt, fs::symlink},
    };
    let directory = tempfile::tempdir().unwrap();
    initialize(directory.path());
    let root = directory.path().canonicalize().unwrap();
    let path = root.join(".gitmodules");
    fs::write(&path, b"[submodule \"named\"]\npath = sub\nignore = all\n").unwrap();
    let bytes = submodule_configuration_file(&path).unwrap().unwrap();
    fs::remove_file(&path).unwrap();
    let fifo = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    assert!(submodule_configuration_file(&path)
        .unwrap_err()
        .contains("regular file"));
    // The parser must consume the captured regular-file bytes, not reopen FIFO.
    let (sender, receiver) = std::sync::mpsc::channel();
    let parser_root = root.clone();
    let parser = std::thread::spawn(move || {
        sender
            .send(submodule_ignore_modes_from_bytes(&parser_root, bytes))
            .unwrap();
    });
    let result = receiver.recv_timeout(std::time::Duration::from_secs(5));
    if result.is_err() {
        use std::{io::Write, os::unix::fs::OpenOptionsExt};
        if let Ok(mut writer) = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
        {
            let _ = writer.write_all(b"[submodule \"named\"]\npath = sub\nignore = all\n");
        }
    }
    assert!(
        result.is_ok(),
        "configuration parser reopened .gitmodules FIFO"
    );
    parser.join().unwrap();
    assert_eq!(result.unwrap().unwrap().get("sub").unwrap(), "all");
    fs::remove_file(&path).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join(".gitmodules"), "outside-private").unwrap();
    symlink(outside.path().join(".gitmodules"), &path).unwrap();
    assert!(submodule_configuration_file(&path).is_err());
    fs::remove_file(&path).unwrap();
    fs::File::create(&path)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    assert!(submodule_configuration_file(&path)
        .unwrap_err()
        .contains("1 MiB"));
    let ancestor = root.join("nested");
    fs::create_dir(&ancestor).unwrap();
    fs::write(ancestor.join(".gitmodules"), "approved").unwrap();
    let resolved = ancestor.join(".gitmodules");
    fs::rename(&ancestor, root.join("old-nested")).unwrap();
    symlink(outside.path(), &ancestor).unwrap();
    assert!(submodule_configuration_file(&resolved).is_err());
}
