use super::*;

fn initialize(root: &Path) {
    fs::create_dir_all(root).unwrap();
    checked(root, &["init", "-b", "main"]).unwrap();
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
