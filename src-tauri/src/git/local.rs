use super::Change;
use crate::files::directory;
#[cfg(unix)]
use std::collections::HashMap;
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

const IGNORE_FILE_LIMIT: u64 = 1024 * 1024;
const REBASE_PATH_LIMIT: usize = 16 * 1024 * 1024;
const REBASE_COMMIT_LIMIT: usize = 10_000;
const REBASE_PREFLIGHT_LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

fn checked(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    super::checked_repository(root, args)
}

fn remotes(root: &Path) -> Result<Vec<String>, String> {
    Ok(String::from_utf8_lossy(&super::checked(root, &["remote"])?)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn validate_remote(root: &Path, remote: &str) -> Result<(), String> {
    if !remotes(root)?.iter().any(|name| name == remote) {
        return Err("This Git remote is no longer configured. Choose a remote again.".into());
    }
    Ok(())
}

fn one_config_value(bytes: &[u8]) -> Result<String, String> {
    let Some(value) = bytes.strip_suffix(b"\0") else {
        return Err("The configured Git upstream is invalid.".into());
    };
    if value.is_empty() || value.contains(&0) {
        return Err("The configured Git upstream is invalid.".into());
    }
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| "The configured Git upstream is not valid UTF-8.".into())
}

fn valid_head_reference(reference: &str) -> bool {
    reference.starts_with("refs/heads/")
        && reference.len() <= 1024
        && !reference.ends_with('/')
        && !reference.contains("..")
        && !reference.contains("@{")
        && !reference
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || "~^:?*[\\".contains(ch))
        && reference.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !part.ends_with(".lock")
        })
}

fn tracking_target(root: &Path) -> Result<(String, String), String> {
    let branch = String::from_utf8_lossy(&checked(root, &["symbolic-ref", "--quiet", "HEAD"])?)
        .trim()
        .to_owned();
    let short = branch
        .strip_prefix("refs/heads/")
        .ok_or("Check out a branch with a configured upstream before pulling.")?;
    let remote_key = format!("branch.{short}.remote");
    let merge_key = format!("branch.{short}.merge");
    let remote = one_config_value(&checked(
        root,
        &["config", "--null", "--get-all", &remote_key],
    )?)?;
    let reference = one_config_value(&checked(
        root,
        &["config", "--null", "--get-all", &merge_key],
    )?)?;
    if remote != "." {
        validate_remote(root, &remote)?;
    }
    if !valid_head_reference(&reference) {
        return Err("The configured Git upstream is not a single branch.".into());
    }
    Ok((remote, reference))
}

fn validate_object_id(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Git did not identify the fetched upstream commit.".into());
    }
    Ok(())
}

fn append_nul_paths(
    paths: &mut std::collections::BTreeSet<Vec<u8>>,
    bytes: &[u8],
    path_limit: usize,
) -> Result<(), String> {
    for path in bytes
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        paths.insert(path.to_vec());
        if paths.len() > path_limit {
            return Err("Safe Git pull exceeded its changed-path limit.".into());
        }
    }
    Ok(())
}

fn path_sets_overlap(
    changed: &std::collections::BTreeSet<Vec<u8>>,
    untracked: &std::collections::BTreeSet<Vec<u8>>,
) -> bool {
    changed.iter().any(|path| {
        if untracked.contains(path) {
            return true;
        }
        for (index, byte) in path.iter().enumerate() {
            if *byte == b'/' && untracked.contains(&path[..index]) {
                return true;
            }
        }
        let mut descendant_prefix = path.clone();
        descendant_prefix.push(b'/');
        untracked
            .range(descendant_prefix.clone()..)
            .next()
            .is_some_and(|candidate| candidate.starts_with(&descendant_prefix))
    })
}

#[cfg(not(unix))]
fn require_verified_rebase_paths(
    untracked: &std::collections::BTreeSet<Vec<u8>>,
) -> Result<(), String> {
    if !untracked.is_empty() {
        return Err("Rebase pull on this platform requires a clean untracked and ignored worktree because safe filesystem path-equivalence checks are unavailable. Move or remove those files, commit files you want to keep, or use merge pull.".into());
    }
    Ok(())
}

#[cfg(unix)]
#[derive(Default)]
struct FilesystemPathTrie {
    children: HashMap<(u64, u64), FilesystemPathTrie>,
    terminal: bool,
}

#[cfg(unix)]
impl FilesystemPathTrie {
    fn insert(&mut self, identities: &[(u64, u64)]) {
        let mut node = self;
        for identity in identities {
            node = node.children.entry(*identity).or_default();
        }
        node.terminal = true;
    }

    fn collides(&self, identities: &[(u64, u64)], component_count: usize) -> bool {
        let mut node = self;
        for (index, identity) in identities.iter().enumerate() {
            let Some(next) = node.children.get(identity) else {
                return false;
            };
            node = next;
            if node.terminal || (index + 1 == component_count && !node.children.is_empty()) {
                return true;
            }
        }
        false
    }
}

#[cfg(unix)]
fn identity_components(root: &fs::File, path: &[u8]) -> Result<(Vec<(u64, u64)>, usize), String> {
    use std::{
        ffi::CString,
        os::fd::{AsRawFd, FromRawFd},
    };

    let components: Vec<_> = path
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .collect();
    if path.starts_with(b"/")
        || path.ends_with(b"/")
        || components.is_empty()
        || components.len() != path.split(|byte| *byte == b'/').count()
        || components
            .iter()
            .any(|part| *part == b"." || *part == b".." || part.contains(&0))
    {
        return Err("Git returned an unsafe path during pull collision checking.".into());
    }
    let mut directory = root.try_clone().map_err(|error| error.to_string())?;
    let mut identities = Vec::with_capacity(components.len());
    for (index, component) in components.iter().enumerate() {
        let name = CString::new(*component)
            .map_err(|_| "Git returned an invalid path during pull collision checking.")?;
        let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name.as_ptr(),
                &mut metadata,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(libc::ENOENT) | Some(libc::ENOTDIR)
            ) {
                break;
            }
            return Err(format!("Cannot safely inspect Git pull paths: {error}"));
        }
        identities.push((metadata.st_dev as u64, metadata.st_ino as u64));
        let is_directory = metadata.st_mode & libc::S_IFMT == libc::S_IFDIR;
        if index + 1 == components.len() || !is_directory {
            break;
        }
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "Cannot safely pin a Git pull path directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        directory = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok((identities, components.len()))
}

#[cfg(unix)]
fn filesystem_path_collides(
    root: &Path,
    changed: &std::collections::BTreeSet<Vec<u8>>,
    untracked: &std::collections::BTreeSet<Vec<u8>>,
    deadline: std::time::Instant,
) -> Result<bool, String> {
    use std::{
        ffi::CString,
        os::{fd::FromRawFd, unix::ffi::OsStrExt},
    };

    let path = CString::new(root.as_os_str().as_bytes())
        .map_err(|_| "The Git repository path is invalid.")?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!(
            "Cannot safely pin the Git repository for pull: {}",
            std::io::Error::last_os_error()
        ));
    }
    let root = unsafe { fs::File::from_raw_fd(fd) };
    // Let the filesystem resolve each incoming component so APFS case and
    // Unicode equivalence is checked by identity, including directory prefixes.
    let mut paths = FilesystemPathTrie::default();
    for path in untracked {
        if std::time::Instant::now() >= deadline {
            return Err("Safe Git pull exceeded its collision-check deadline.".into());
        }
        let (identities, component_count) = identity_components(&root, path)?;
        if identities.len() != component_count {
            return Err("An untracked or ignored file changed during pull preparation.".into());
        }
        paths.insert(&identities);
    }
    for path in changed {
        if std::time::Instant::now() >= deadline {
            return Err("Safe Git pull exceeded its collision-check deadline.".into());
        }
        let (identities, component_count) = identity_components(&root, path)?;
        if paths.collides(&identities, component_count) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn git_paths(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let bytes = super::history::bounded_bytes(root, args, REBASE_PATH_LIMIT)?;
    if bytes.len() > REBASE_PATH_LIMIT {
        return Err("Safe Git pull exceeded its path observation limit.".into());
    }
    Ok(bytes)
}

fn reject_rebase_collisions(root: &Path, head: &str, target: &str) -> Result<(), String> {
    use std::collections::BTreeSet;

    let deadline = std::time::Instant::now() + REBASE_PREFLIGHT_LIMIT;
    let mut changed = BTreeSet::new();
    append_nul_paths(
        &mut changed,
        &git_paths(
            root,
            &[
                "diff",
                "--no-renames",
                "--name-only",
                "-z",
                head,
                target,
                "--",
            ],
        )?,
        100_000,
    )?;

    let range = format!("{target}..{head}");
    let commits = git_paths(
        root,
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            "--max-count=10001",
            &range,
        ],
    )?;
    let commits: Vec<_> = commits
        .split(|byte| *byte == b'\n')
        .filter(|commit| !commit.is_empty())
        .collect();
    if commits.len() > REBASE_COMMIT_LIMIT {
        return Err("Safe Git pull exceeded its replay-commit limit.".into());
    }
    for commit in commits {
        if std::time::Instant::now() >= deadline {
            return Err("Safe Git pull exceeded its collision-check deadline.".into());
        }
        let commit =
            std::str::from_utf8(commit).map_err(|_| "Git returned an invalid replay commit.")?;
        validate_object_id(commit)?;
        let commit_paths = git_paths(
            root,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-only",
                "--no-renames",
                "-r",
                "-m",
                "-z",
                commit,
                "--",
            ],
        )?;
        append_nul_paths(&mut changed, &commit_paths, 100_000)?;
    }

    let mut untracked = BTreeSet::new();
    for args in [
        &(["ls-files", "--others", "--exclude-standard", "-z"] as [&str; 4])[..],
        &([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ] as [&str; 5])[..],
    ] {
        append_nul_paths(&mut untracked, &git_paths(root, args)?, 100_000)?;
    }
    #[cfg(unix)]
    let collides = path_sets_overlap(&changed, &untracked)
        || filesystem_path_collides(root, &changed, &untracked, deadline)?;
    #[cfg(not(unix))]
    let collides = {
        require_verified_rebase_paths(&untracked)?;
        path_sets_overlap(&changed, &untracked)
    };
    if collides {
        return Err(
            "Pull would overwrite an untracked or ignored file. Move it and try again.".into(),
        );
    }
    Ok(())
}

fn selected_git_path(path: &str) -> Vec<u8> {
    let mut selected = Vec::new();
    for component in Path::new(path).components() {
        let Component::Normal(part) = component else {
            continue;
        };
        if !selected.is_empty() {
            selected.push(b'/');
        }
        #[cfg(windows)]
        selected.extend(part.to_string_lossy().replace('\\', "/").bytes());
        #[cfg(not(windows))]
        selected.extend_from_slice(part.as_encoded_bytes());
    }
    selected
}

fn path_is_selected_by(selected: &[u8], candidate: &[u8]) -> bool {
    selected.is_empty()
        || candidate == selected
        || (candidate.starts_with(selected)
            && candidate
                .get(selected.len())
                .is_some_and(|byte| *byte == b'/'))
}

fn private_repo_paths(root: &Path) -> Result<Vec<Vec<u8>>, String> {
    let mut private = Vec::new();
    for args in [
        &(["ls-files", "--cached", "-z"] as [&str; 3])[..],
        &(["ls-files", "--others", "--exclude-standard", "-z"] as [&str; 4])[..],
        &([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ] as [&str; 5])[..],
    ] {
        let bytes = git_paths(root, args)?;
        for path in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            if path
                .split(|byte| *byte == b'/')
                .any(lomi_files::is_private_staging_name)
            {
                private.push(path.to_vec());
                if private.len() > 100_000 {
                    return Err("Safe Git staging exceeded its private-path limit.".into());
                }
            }
        }
    }
    Ok(private)
}

fn reject_private_stage_selection(root: &Path, selections: &[String]) -> Result<(), String> {
    let private = private_repo_paths(root)?;
    for selection in selections {
        let selection = selected_git_path(selection);
        if private
            .iter()
            .any(|path| path_is_selected_by(&selection, path))
        {
            return Err(
                "This selection includes a path reserved for private file recovery.".into(),
            );
        }
    }
    Ok(())
}

fn reject_private_commit_paths(root: &Path) -> Result<(), String> {
    let staged = git_paths(
        root,
        &["diff", "--cached", "--no-renames", "--name-only", "-z"],
    )?;
    if staged
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .any(|path| {
            path.split(|byte| *byte == b'/')
                .any(lomi_files::is_private_staging_name)
        })
    {
        return Err("The index contains a path reserved for private file recovery.".into());
    }
    Ok(())
}

pub(crate) fn stage(root: &str, paths: &[String], stage: bool) -> Result<(), String> {
    let _operation = super::mutation_guard(root)?;
    let root = super::repository(root)?;
    if paths.is_empty() {
        return Ok(());
    }
    if paths.len() > 200 {
        return Err("Select at most 200 files for a Git index change.".into());
    }
    for path in paths {
        super::relative(path)?;
        if private_staging_path(Path::new(path)) {
            return Err("This path is reserved for private file recovery.".into());
        }
    }
    reject_private_stage_selection(&root, paths)?;
    let has_head = super::command(&root, &["rev-parse", "--verify", "HEAD"])?
        .status
        .success();
    let mut args = if stage {
        vec!["--literal-pathspecs", "add", "--"]
    } else if has_head {
        vec!["--literal-pathspecs", "restore", "--staged", "--"]
    } else {
        vec!["--literal-pathspecs", "rm", "--cached", "--"]
    };
    args.extend(paths.iter().map(String::as_str));
    checked(&root, &args)?;
    Ok(())
}

pub(crate) fn commit(root: &str, message: &str) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("Enter a commit message.".into());
    }
    if message.len() > 16_384 {
        return Err("The commit message exceeds the supported limit.".into());
    }
    let _operation = super::mutation_guard(root)?;
    let root = super::repository(root)?;
    reject_private_commit_paths(&root)?;
    checked(&root, &["commit", "--cleanup=verbatim", "-m", message])?;
    Ok(())
}

pub(crate) fn fetch(root: &str, remote: Option<&str>) -> Result<(), String> {
    let _operation = super::mutation_guard(root)?;
    let root = super::repository(root)?;
    if let Some(remote) = remote {
        validate_remote(&root, remote)?;
        checked(&root, &["fetch", "--", remote])?;
    } else if remotes(&root)?.is_empty() {
        return Err(
            "No Git remote is configured. Add a remote in a terminal, then try again.".into(),
        );
    } else {
        checked(&root, &["fetch", "--all"])?;
    }
    Ok(())
}

pub(crate) fn pull(root: &str, rebase: bool) -> Result<(), String> {
    let _operation = super::mutation_guard(root)?;
    let root = super::repository(root)?;
    if rebase {
        let dirty = super::command(&root, &["diff", "--quiet", "HEAD", "--"])?;
        if !dirty.status.success() {
            return Err("Commit or discard tracked changes before rebasing.".into());
        }
    }

    let (remote, reference) = tracking_target(&root)?;
    checked(
        &root,
        &[
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-auto-maintenance",
            "--",
            &remote,
            &reference,
        ],
    )?;
    let target = String::from_utf8_lossy(&checked(
        &root,
        &["rev-parse", "--verify", "FETCH_HEAD^{commit}"],
    )?)
    .trim()
    .to_owned();
    validate_object_id(&target)?;

    if rebase {
        let head = String::from_utf8_lossy(&checked(
            &root,
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )?)
        .trim()
        .to_owned();
        validate_object_id(&head)?;
        let dirty = super::command(&root, &["diff", "--quiet", "HEAD", "--"])?;
        if !dirty.status.success() {
            return Err("Tracked files changed during the pull. Refresh and try again.".into());
        }
        reject_rebase_collisions(&root, &head, &target)?;
        checked(&root, &["rebase", "--no-autostash", &target])?;
    } else {
        checked(
            &root,
            &[
                "merge",
                "--ff-only",
                "--no-edit",
                "--no-autostash",
                "--no-overwrite-ignore",
                &target,
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn push(root: &str, remote: Option<&str>, force: bool) -> Result<(), String> {
    let _operation = super::mutation_guard(root)?;
    let root = super::repository(root)?;
    super::checked(&root, &["symbolic-ref", "--quiet", "HEAD"])
        .map_err(|_| "Check out a branch before pushing.".to_string())?;
    let mut args = vec!["push"];
    if force {
        args.push("--force-with-lease");
    }
    if let Some(remote) = remote {
        validate_remote(&root, remote)?;
        args.extend(["--", remote, "HEAD"]);
    }
    checked(&root, &args)?;
    Ok(())
}

pub(crate) fn discard(root: &str, expected: &Change) -> Result<(), String> {
    let _operation = super::mutation_guard(root)?;
    super::relative(&expected.path)?;
    let status = super::exact_status(root)?;
    let change = status
        .changes
        .iter()
        .find(|change| change.path == expected.path);
    if change != Some(expected) {
        return Err("The file status changed. Refresh Source Control and try again.".into());
    }

    let root = Path::new(&status.root);
    let path = root.join(&expected.path);
    for parent in path
        .ancestors()
        .skip(1)
        .take_while(|parent| *parent != root)
    {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Cannot discard changes through a symbolic link.".into())
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string())
            }
            _ => {}
        }
    }

    if expected.index == '?' && expected.worktree == '?' {
        return trash::delete(&path).map_err(|error| error.to_string());
    }
    if !matches!(expected.worktree, 'M' | 'D' | 'T')
        || expected.index == 'U'
        || fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(
            "Only unconflicted working tree files can be discarded. Unstage staged changes first."
                .into(),
        );
    }
    checked(
        root,
        &[
            "--literal-pathspecs",
            "restore",
            "--worktree",
            "--",
            &expected.path,
        ],
    )?;
    Ok(())
}

struct SelectedPath {
    repository: PathBuf,
    relative: String,
    is_directory: bool,
}

fn selected_path(project_root: &str, relative: &str) -> Result<SelectedPath, String> {
    crate::files::operations::validate_relative(relative)?;
    super::relative(relative)?;
    if relative.is_empty() || relative.contains(['\r', '\n']) {
        return Err("Choose a file or folder inside the Git repository.".into());
    }

    let project = directory(project_root)?;
    if private_staging_path(&project) || private_staging_path(Path::new(relative)) {
        return Err("This path is reserved for private file recovery.".into());
    }
    let selected = project.join(relative);
    let metadata = fs::symlink_metadata(&selected).map_err(|error| error.to_string())?;
    let is_directory = metadata.is_dir();
    if !is_directory && !metadata.is_file() && !metadata.file_type().is_symlink() {
        return Err("This Git item cannot be ignored.".into());
    }

    let mut parent = selected.parent().ok_or("Invalid selected path.")?;
    while parent != project {
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Cannot ignore an item through a symbolic link.".into())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("The selected Git item has an invalid parent path.".into())
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
        parent = parent
            .parent()
            .ok_or("The selected item is outside the project.")?;
    }

    let start = if is_directory {
        &selected
    } else {
        selected.parent().ok_or("Invalid selected path.")?
    };
    let output = super::checked(start, &["rev-parse", "--show-toplevel"])?;
    let repository_text = std::str::from_utf8(&strip_git_newline(&output))
        .map_err(|_| "The Git repository path is not valid UTF-8.")?;
    let repository = directory(repository_text)?;
    if !selected.starts_with(&repository) {
        return Err("The selected path is outside its Git repository.".into());
    }
    let path = selected
        .strip_prefix(&repository)
        .map_err(|_| "The selected path is outside its Git repository.")?;
    let path = path
        .to_str()
        .ok_or("The selected Git item path is not valid UTF-8.")?;
    #[cfg(windows)]
    let path = path.replace('\\', "/");
    #[cfg(not(windows))]
    let path = path.to_owned();
    super::relative(&path)?;
    if path.is_empty() || private_staging_path(Path::new(&path)) {
        return Err("This path is reserved for private file recovery.".into());
    }
    Ok(SelectedPath {
        repository,
        relative: path,
        is_directory,
    })
}

pub(super) fn private_staging_path(path: &Path) -> bool {
    path.components().any(|part| {
        matches!(part, Component::Normal(name)
            if lomi_files::is_private_staging_name(name.as_encoded_bytes()))
    })
}

fn strip_git_newline(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

fn ignore_pattern(path: &str, directory: bool) -> Result<Vec<u8>, String> {
    if path.is_empty() || path.contains(['\r', '\n']) {
        return Err("Choose a valid Git path to ignore.".into());
    }
    let mut pattern = Vec::with_capacity(path.len() + 3);
    pattern.push(b'/');
    for byte in path.bytes() {
        if matches!(byte, b' ' | b'\\' | b'*' | b'?' | b'[' | b']') {
            pattern.push(b'\\');
        }
        pattern.push(byte);
    }
    if directory {
        pattern.push(b'/');
    }
    Ok(pattern)
}

fn append_ignore_pattern(mut existing: Vec<u8>, pattern: &[u8]) -> Result<Vec<u8>, String> {
    let already_present = existing
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == pattern);
    if !already_present {
        if !existing.is_empty() && !existing.ends_with(b"\n") {
            existing.push(b'\n');
        }
        existing.extend_from_slice(pattern);
        existing.push(b'\n');
    }
    if existing.len() as u64 > IGNORE_FILE_LIMIT {
        return Err("The Git ignore file exceeds its 1 MiB limit.".into());
    }
    Ok(existing)
}

fn read_ignore_file(path: &Path) -> Result<(Vec<u8>, Option<fs::Permissions>), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), None))
        }
        Err(error) => return Err(error.to_string()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("The Git ignore target must be a regular file.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err("The Git ignore target cannot have multiple hard links.".into());
        }
    }
    if metadata.len() > IGNORE_FILE_LIMIT {
        return Err("The Git ignore file exceeds its 1 MiB limit.".into());
    }
    let mut file = crate::files::resolved::open_resolved_file(path)
        .map_err(|error| format!("Cannot safely read Git ignore settings: {error}"))?;
    let opened = file.metadata().map_err(|error| error.to_string())?;
    if !opened.is_file() || opened.len() > IGNORE_FILE_LIMIT {
        return Err("The Git ignore target changed while it was read.".into());
    }
    let mut bytes = Vec::with_capacity(opened.len() as usize);
    file.take(IGNORE_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > IGNORE_FILE_LIMIT {
        return Err("The Git ignore file exceeds its 1 MiB limit.".into());
    }
    Ok((bytes, Some(metadata.permissions())))
}

fn write_ignore_file(
    path: &Path,
    bytes: &[u8],
    previous_bytes: &[u8],
    permissions: Option<fs::Permissions>,
) -> Result<(), String> {
    let had_permissions = permissions.is_some();
    let parent = path.parent().ok_or("Invalid Git ignore target.")?;
    reject_symlink_components(parent)?;
    let mut replacement = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Cannot prepare Git ignore update: {error}"))?;
    use std::io::Write;
    replacement
        .write_all(bytes)
        .map_err(|error| format!("Cannot write Git ignore settings: {error}"))?;
    replacement
        .as_file()
        .sync_all()
        .map_err(|error| format!("Cannot flush Git ignore settings: {error}"))?;
    if let Some(permissions) = permissions.as_ref() {
        replacement
            .as_file()
            .set_permissions(permissions.clone())
            .map_err(|error| error.to_string())?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            replacement
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o644))
                .map_err(|error| error.to_string())?;
        }
    }
    let (current_bytes, current_permissions) = read_ignore_file(path)?;
    if current_bytes != previous_bytes || current_permissions.is_some() != had_permissions {
        return Err("The Git ignore file changed. Refresh and try again.".into());
    }
    reject_symlink_components(parent)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err("The Git ignore target changed before publication.".into())
        }
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.to_string()),
        _ => {}
    }
    replacement
        .persist(path)
        .map_err(|error| format!("Cannot publish Git ignore settings: {}", error.error))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("Cannot flush the Git ignore directory: {error}"))?;
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("Git ignore settings cannot be written through a symbolic link.".into())
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err("The Git ignore parent is not a directory.".into())
            }
            Ok(_) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn common_git_dir(repository: &Path) -> Result<PathBuf, String> {
    let output = checked(repository, &["rev-parse", "--git-common-dir"])?;
    let text = std::str::from_utf8(&strip_git_newline(&output))
        .map_err(|_| "The Git metadata path is not valid UTF-8.")?;
    let path = Path::new(text);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repository.join(path)
    };
    reject_symlink_components(&path)?;
    let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("The Git metadata directory is not a safe directory.".into());
    }
    fs::canonicalize(path).map_err(|error| error.to_string())
}

pub(crate) fn ignore(project_root: &str, relative: &str, local: bool) -> Result<(), String> {
    let selected = selected_path(project_root, relative)?;
    let _operation = super::mutation_guard(
        selected
            .repository
            .to_str()
            .ok_or("The Git repository path is not valid UTF-8.")?,
    )?;
    let checked_repository = super::repository(
        selected
            .repository
            .to_str()
            .ok_or("The Git repository path is not valid UTF-8.")?,
    )?;
    if checked_repository != selected.repository {
        return Err("The Git repository changed. Refresh Source Control and try again.".into());
    }
    let current = selected_path(project_root, relative)?;
    if current.repository != selected.repository
        || current.relative != selected.relative
        || current.is_directory != selected.is_directory
    {
        return Err("The selected Git item changed. Refresh and try again.".into());
    }

    let target = if local {
        let common = common_git_dir(&selected.repository)?;
        let info = common.join("info");
        match fs::symlink_metadata(&info) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err("The Git info directory is not safe to update.".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&info).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
        info.join("exclude")
    } else {
        selected.repository.join(".gitignore")
    };
    let (existing, permissions) = read_ignore_file(&target)?;
    let pattern = ignore_pattern(&selected.relative, selected.is_directory)?;
    let bytes = append_ignore_pattern(existing.clone(), &pattern)?;
    if bytes == existing {
        return Ok(());
    }
    write_ignore_file(&target, &bytes, &existing, permissions)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checked(root: &Path, args: &[&str]) -> Vec<u8> {
        let mut command = super::super::mutation_command(root, args);
        command
            .env(
                "GIT_CONFIG_GLOBAL",
                super::super::observation::null_device(),
            )
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_COUNT", "0")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("GIT_AUTHOR_NAME")
            .env_remove("GIT_AUTHOR_EMAIL")
            .env_remove("GIT_COMMITTER_NAME")
            .env_remove("GIT_COMMITTER_EMAIL")
            .env_remove("GIT_AUTHOR_DATE")
            .env_remove("GIT_COMMITTER_DATE");
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }

    fn repository() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        checked(root.path(), &["init", "-b", "main"]);
        for (key, value) in [
            ("user.name", "Lomi Git Test"),
            ("user.email", "lomi-git-test@example.invalid"),
            ("user.useConfigOnly", "true"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", ".git/disabled-hooks"),
            ("core.autocrlf", "false"),
        ] {
            checked(root.path(), &["config", key, value]);
        }
        root
    }

    fn root_text(path: &Path) -> &str {
        path.to_str().unwrap()
    }

    fn names(bytes: &[u8]) -> Vec<String> {
        bytes
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).into_owned())
            .collect()
    }

    #[test]
    fn stage_unstage_and_commit_preserve_literal_names_and_message() {
        let root = repository();
        for name in ["-leading file.txt", "space name.txt"] {
            fs::write(root.path().join(name), name).unwrap();
        }
        stage(
            root_text(root.path()),
            &["-leading file.txt".into(), "space name.txt".into()],
            true,
        )
        .unwrap();
        let staged = names(&checked(
            root.path(),
            &["diff", "--cached", "--name-only", "-z"],
        ));
        assert_eq!(staged, ["-leading file.txt", "space name.txt"]);

        stage(root_text(root.path()), &["-leading file.txt".into()], false).unwrap();
        let staged = names(&checked(
            root.path(),
            &["diff", "--cached", "--name-only", "-z"],
        ));
        assert_eq!(staged, ["space name.txt"]);
        stage(root_text(root.path()), &["-leading file.txt".into()], true).unwrap();

        let message = "Keep the exact subject\n\nand body";
        commit(root_text(root.path()), message).unwrap();
        let object = checked(root.path(), &["cat-file", "commit", "HEAD"]);
        let separator = object
            .windows(2)
            .position(|window| window == b"\n\n")
            .unwrap();
        assert_eq!(&object[separator + 2..], format!("{message}\n").as_bytes());
        let identity = checked(root.path(), &["show", "-s", "--format=%an <%ae>", "HEAD"]);
        assert_eq!(
            String::from_utf8_lossy(&identity).trim(),
            "Lomi Git Test <lomi-git-test@example.invalid>"
        );
    }

    #[test]
    fn discard_restores_only_the_confirmed_file_and_checks_paths() {
        let root = repository();
        for name in ["first file.txt", "other-file.txt"] {
            fs::write(root.path().join(name), "committed\n").unwrap();
            stage(root_text(root.path()), &[name.into()], true).unwrap();
        }
        commit(root_text(root.path()), "initial").unwrap();
        fs::write(root.path().join("first file.txt"), "changed\n").unwrap();
        fs::write(root.path().join("other-file.txt"), "keep changed\n").unwrap();
        let first = super::super::status(root_text(root.path()))
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "first file.txt")
            .unwrap();
        discard(root_text(root.path()), &first).unwrap();
        assert_eq!(
            fs::read_to_string(root.path().join("first file.txt")).unwrap(),
            "committed\n"
        );
        assert_eq!(
            fs::read_to_string(root.path().join("other-file.txt")).unwrap(),
            "keep changed\n"
        );

        fs::write(root.path().join("first file.txt"), "changed again\n").unwrap();
        stage(root_text(root.path()), &["first file.txt".into()], true).unwrap();
        assert!(discard(root_text(root.path()), &first)
            .unwrap_err()
            .contains("status changed"));
        assert!(stage(root_text(root.path()), &["../outside".into()], true).is_err());
        let invalid = Change {
            path: "../outside".into(),
            original_path: None,
            index: ' ',
            worktree: 'M',
        };
        assert!(discard(root_text(root.path()), &invalid).is_err());
    }

    #[test]
    fn status_hides_private_recovery_and_stage_directory_cannot_capture_it() {
        let root = repository();
        let private_name = ".lomi-desktop-0123456789abcdef0123456789abcdef.tmp";
        fs::write(root.path().join(private_name), "recoverable user data").unwrap();
        fs::write(root.path().join("ordinary.txt"), "ordinary change").unwrap();

        let status = super::super::status(root_text(root.path()))
            .unwrap()
            .unwrap();
        assert!(status
            .changes
            .iter()
            .all(|change| !private_staging_path(Path::new(&change.path))));
        assert!(stage(root_text(root.path()), &[".".into()], true)
            .unwrap_err()
            .contains("private file recovery"));
        assert!(stage(root_text(root.path()), &[private_name.into()], true).is_err());
        assert!(checked(root.path(), &["diff", "--cached", "--name-only"]).is_empty());

        checked(root.path(), &["add", "--", private_name]);
        assert!(
            commit(root_text(root.path()), "must not publish recovery data")
                .unwrap_err()
                .contains("private file recovery")
        );
        let status = super::super::status(root_text(root.path()))
            .unwrap()
            .unwrap();
        assert!(status
            .changes
            .iter()
            .all(|change| !private_staging_path(Path::new(&change.path))));
    }

    #[cfg(unix)]
    #[test]
    fn discard_refuses_to_restore_through_a_symlink_parent() {
        use std::os::unix::fs::symlink;

        let root = repository();
        let nested = root.path().join("folder");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("tracked.txt"), "tracked\n").unwrap();
        stage(root_text(root.path()), &["folder/tracked.txt".into()], true).unwrap();
        commit(root_text(root.path()), "initial").unwrap();
        fs::remove_file(nested.join("tracked.txt")).unwrap();
        fs::remove_dir(&nested).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), &nested).unwrap();
        let change = super::super::status(root_text(root.path()))
            .unwrap()
            .unwrap()
            .changes
            .into_iter()
            .find(|change| change.path == "folder/tracked.txt")
            .unwrap();
        assert!(discard(root_text(root.path()), &change)
            .unwrap_err()
            .contains("symbolic link"));
        assert!(!outside.path().join("tracked.txt").exists());
    }

    #[test]
    fn ignore_writes_escaped_rules_to_the_selected_target() {
        let root = repository();
        let selected = root.path().join("assets");
        fs::create_dir(&selected).unwrap();
        fs::write(selected.join("[draft] file?.txt"), "draft").unwrap();

        ignore(root_text(root.path()), "assets/[draft] file?.txt", false).unwrap();
        assert_eq!(
            fs::read(root.path().join(".gitignore")).unwrap(),
            b"/assets/\\[draft\\]\\ file\\?.txt\n"
        );
        ignore(root_text(root.path()), "assets/[draft] file?.txt", false).unwrap();
        assert_eq!(
            fs::read(root.path().join(".gitignore")).unwrap(),
            b"/assets/\\[draft\\]\\ file\\?.txt\n"
        );

        let exclude = root.path().join(".git/info/exclude");
        let previous_exclude = fs::read(&exclude).unwrap_or_default();
        let expected_exclude =
            append_ignore_pattern(previous_exclude.clone(), b"/assets/").unwrap();
        ignore(root_text(root.path()), "assets", true).unwrap();
        let actual_exclude = fs::read(&exclude).unwrap();
        assert_eq!(actual_exclude, expected_exclude);
        assert!(actual_exclude.starts_with(&previous_exclude));
        assert!(ignore(root_text(root.path()), "../outside", false).is_err());
    }

    #[test]
    fn fetch_pull_and_push_use_only_a_temporary_bare_remote() {
        let remote = tempfile::tempdir().unwrap();
        let first = repository();
        let second = repository();
        checked(remote.path(), &["init", "--bare", "-b", "main"]);

        fs::write(first.path().join("initial.txt"), "initial\n").unwrap();
        stage(root_text(first.path()), &["initial.txt".into()], true).unwrap();
        commit(root_text(first.path()), "initial").unwrap();
        checked(
            first.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        push(root_text(first.path()), Some("origin"), false).unwrap();
        checked(first.path(), &["branch", "--set-upstream-to=origin/main"]);

        checked(
            second.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        fetch(root_text(second.path()), Some("origin")).unwrap();
        checked(second.path(), &["checkout", "-B", "main", "origin/main"]);
        checked(second.path(), &["branch", "--set-upstream-to=origin/main"]);
        fs::write(second.path().join("remote.txt"), "remote\n").unwrap();
        stage(root_text(second.path()), &["remote.txt".into()], true).unwrap();
        commit(root_text(second.path()), "remote update").unwrap();
        push(root_text(second.path()), None, false).unwrap();

        fetch(root_text(first.path()), None).unwrap();
        assert!(fs::read(first.path().join("remote.txt")).is_err());
        pull(root_text(first.path()), false).unwrap();
        assert_eq!(
            fs::read_to_string(first.path().join("remote.txt")).unwrap(),
            "remote\n"
        );
        let remote_head = checked(remote.path(), &["rev-parse", "main"]);
        let local_head = checked(first.path(), &["rev-parse", "HEAD"]);
        assert_eq!(
            String::from_utf8_lossy(&remote_head).trim(),
            String::from_utf8_lossy(&local_head).trim()
        );
    }

    fn pull_pair() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        let remote = tempfile::tempdir().unwrap();
        let source = repository();
        let client = repository();
        checked(remote.path(), &["init", "--bare", "-b", "main"]);
        fs::write(
            source.path().join(".gitignore"),
            ".ignored-collision\n*.secret\n",
        )
        .unwrap();
        fs::write(source.path().join("tracked.txt"), "base\n").unwrap();
        stage(
            root_text(source.path()),
            &[".gitignore".into(), "tracked.txt".into()],
            true,
        )
        .unwrap();
        commit(root_text(source.path()), "base").unwrap();
        checked(
            source.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        push(root_text(source.path()), Some("origin"), false).unwrap();
        checked(source.path(), &["branch", "--set-upstream-to=origin/main"]);

        checked(
            client.path(),
            &["remote", "add", "origin", remote.path().to_str().unwrap()],
        );
        fetch(root_text(client.path()), Some("origin")).unwrap();
        checked(client.path(), &["checkout", "-B", "main", "origin/main"]);
        checked(client.path(), &["branch", "--set-upstream-to=origin/main"]);
        (remote, source, client)
    }

    #[test]
    fn pull_merge_and_rebase_preserve_ignored_and_untracked_collisions() {
        for rebase in [false, true] {
            let (_remote, source, client) = pull_pair();
            if rebase {
                fs::write(client.path().join("local.txt"), "local commit\n").unwrap();
                stage(root_text(client.path()), &["local.txt".into()], true).unwrap();
                commit(root_text(client.path()), "local change").unwrap();
            }

            fs::write(source.path().join(".ignored-collision"), "remote ignored\n").unwrap();
            fs::write(source.path().join("plain-collision"), "remote untracked\n").unwrap();
            checked(
                source.path(),
                &["add", "--force", "--", ".ignored-collision"],
            );
            stage(root_text(source.path()), &["plain-collision".into()], true).unwrap();
            commit(root_text(source.path()), "add collision paths").unwrap();
            push(root_text(source.path()), None, false).unwrap();

            let head = checked(client.path(), &["rev-parse", "HEAD"]);
            fs::write(
                client.path().join(".ignored-collision"),
                "keep ignored secret\n",
            )
            .unwrap();
            assert!(pull(root_text(client.path()), rebase).is_err());
            assert_eq!(
                fs::read_to_string(client.path().join(".ignored-collision")).unwrap(),
                "keep ignored secret\n"
            );
            assert_eq!(checked(client.path(), &["rev-parse", "HEAD"]), head);

            fs::remove_file(client.path().join(".ignored-collision")).unwrap();
            fs::write(
                client.path().join("plain-collision"),
                "keep untracked data\n",
            )
            .unwrap();
            assert!(pull(root_text(client.path()), rebase).is_err());
            assert_eq!(
                fs::read_to_string(client.path().join("plain-collision")).unwrap(),
                "keep untracked data\n"
            );
            assert_eq!(checked(client.path(), &["rev-parse", "HEAD"]), head);
            if rebase {
                assert_eq!(
                    fs::read_to_string(client.path().join("local.txt")).unwrap(),
                    "local commit\n"
                );
            }
        }
    }

    #[cfg(unix)]
    fn same_filesystem_entry(root: &Path, left: &str, right: &str) -> bool {
        use std::os::unix::fs::MetadataExt;

        let Ok(left) = fs::symlink_metadata(root.join(left)) else {
            return false;
        };
        let Ok(right) = fs::symlink_metadata(root.join(right)) else {
            return false;
        };
        left.dev() == right.dev() && left.ino() == right.ino()
    }

    #[cfg(unix)]
    fn assert_rebase_rejects_filesystem_alias(
        incoming_name: &str,
        existing_name: &str,
        directory: bool,
    ) {
        let (_remote, source, client) = pull_pair();
        fs::write(source.path().join(incoming_name), "remote content\n").unwrap();
        checked(source.path(), &["add", "--force", "--", incoming_name]);
        commit(root_text(source.path()), "add aliased path").unwrap();
        push(root_text(source.path()), None, false).unwrap();

        let local = client.path().join(existing_name);
        if directory {
            fs::create_dir(&local).unwrap();
            fs::write(local.join("held.secret"), "keep local secret\n").unwrap();
        } else {
            fs::write(&local, "keep local secret\n").unwrap();
        }
        // Case and Unicode aliases only exist on filesystems that admit them.
        if !same_filesystem_entry(client.path(), incoming_name, existing_name) {
            return;
        }

        let head = checked(client.path(), &["rev-parse", "HEAD"]);
        assert!(pull(root_text(client.path()), true)
            .unwrap_err()
            .contains("untracked or ignored file"));
        assert_eq!(checked(client.path(), &["rev-parse", "HEAD"]), head);
        let kept = if directory {
            local.join("held.secret")
        } else {
            local
        };
        assert_eq!(fs::read_to_string(kept).unwrap(), "keep local secret\n");
    }

    #[cfg(unix)]
    #[test]
    fn rebase_pull_detects_case_unicode_and_directory_prefix_aliases() {
        assert_rebase_rejects_filesystem_alias("Config.Secret", "config.secret", false);
        assert_rebase_rejects_filesystem_alias("Caf\u{00e9}.secret", "Cafe\u{0301}.secret", false);
        assert_rebase_rejects_filesystem_alias("Config.secret", "config.secret", true);
    }

    #[test]
    fn rebase_pull_keeps_tracked_local_edits_without_autostash() {
        let (_remote, source, client) = pull_pair();
        fs::write(source.path().join("remote.txt"), "remote commit\n").unwrap();
        stage(root_text(source.path()), &["remote.txt".into()], true).unwrap();
        commit(root_text(source.path()), "remote change").unwrap();
        push(root_text(source.path()), None, false).unwrap();

        fs::write(
            client.path().join("tracked.txt"),
            "local uncommitted edit\n",
        )
        .unwrap();
        let head = checked(client.path(), &["rev-parse", "HEAD"]);
        assert!(pull(root_text(client.path()), true)
            .unwrap_err()
            .contains("tracked changes"));
        assert_eq!(
            fs::read_to_string(client.path().join("tracked.txt")).unwrap(),
            "local uncommitted edit\n"
        );
        assert_eq!(checked(client.path(), &["rev-parse", "HEAD"]), head);
    }

    #[cfg(unix)]
    const DETACHED_FIXTURE_MODE: &str = "LOMI_GIT_DETACHED_FIXTURE_MODE";
    #[cfg(unix)]
    const DETACHED_FIXTURE_PIDFILE: &str = "LOMI_GIT_DETACHED_FIXTURE_PIDFILE";

    #[cfg(unix)]
    #[test]
    fn mutation_runner_detached_pipe_fixture() {
        use std::{process::Command, time::Instant};

        match std::env::var(DETACHED_FIXTURE_MODE).as_deref() {
            Ok("detached") => {
                assert!(unsafe { libc::setsid() } >= 0);
                let pidfile = std::env::var_os(DETACHED_FIXTURE_PIDFILE).unwrap();
                fs::write(pidfile, std::process::id().to_string()).unwrap();
                let deadline = Instant::now() + std::time::Duration::from_secs(15);
                while Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
            Ok("parent") => {
                let pidfile = std::env::var_os(DETACHED_FIXTURE_PIDFILE).unwrap();
                let executable = std::env::current_exe().unwrap();
                let _child = Command::new(executable)
                    .args(["mutation_runner_detached_pipe_fixture", "--nocapture"])
                    .env(DETACHED_FIXTURE_MODE, "detached")
                    .env(DETACHED_FIXTURE_PIDFILE, &pidfile)
                    .spawn()
                    .unwrap();
                let deadline = Instant::now() + std::time::Duration::from_secs(2);
                while !Path::new(&pidfile).exists() && Instant::now() < deadline {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(Path::new(&pidfile).exists());
            }
            _ => {}
        }
    }

    #[cfg(unix)]
    #[test]
    fn bounded_mutation_runner_marks_timeout_and_output_overflow_unknown() {
        use std::process::Command;

        let mut timeout = Command::new("/bin/sh");
        timeout.args(["-c", "sleep 5"]);
        let error =
            super::super::run_mutation_command(&mut timeout, std::time::Duration::from_millis(50))
                .unwrap_err();
        assert!(error.contains("outcome is unknown"));

        let mut overflow = Command::new("/usr/bin/yes");
        overflow.arg("git output");
        let error =
            super::super::run_mutation_command(&mut overflow, std::time::Duration::from_secs(5))
                .unwrap_err();
        assert!(error.contains("outcome is unknown"));
    }

    #[cfg(unix)]
    #[test]
    fn bounded_mutation_runner_reports_escaped_pipe_holder_without_waiting() {
        use std::{process::Command, time::Instant};

        struct FixtureCleanup(PathBuf);
        impl Drop for FixtureCleanup {
            fn drop(&mut self) {
                if let Ok(text) = fs::read_to_string(&self.0) {
                    if let Ok(pid) = text.parse::<i32>() {
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                }
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let pidfile = directory.path().join("detached-pid");
        let _cleanup = FixtureCleanup(pidfile.clone());
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["mutation_runner_detached_pipe_fixture", "--nocapture"])
            .env(DETACHED_FIXTURE_MODE, "parent")
            .env(DETACHED_FIXTURE_PIDFILE, &pidfile);
        let started = Instant::now();
        let error =
            super::super::run_mutation_command(&mut command, std::time::Duration::from_secs(5))
                .unwrap_err();
        assert!(pidfile.is_file());
        assert!(error.contains("left output open after its process tree exited"));
        assert!(started.elapsed() < std::time::Duration::from_secs(4));
    }

    #[test]
    fn mutation_commands_drop_git_routing_and_config_injection_environment() {
        use std::{ffi::OsString, process::Command};

        let first = repository();
        let second = repository();
        fs::write(first.path().join("first.txt"), "first\n").unwrap();
        fs::write(second.path().join("second.txt"), "second\n").unwrap();
        let environment = vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (OsString::from("HOME"), OsString::from("/tmp")),
            (
                OsString::from("GIT_DIR"),
                second.path().join(".git").into_os_string(),
            ),
            (
                OsString::from("GIT_WORK_TREE"),
                second.path().as_os_str().to_owned(),
            ),
            (
                OsString::from("GIT_INDEX_FILE"),
                second.path().join(".git/index").into_os_string(),
            ),
            (OsString::from("GIT_CONFIG_COUNT"), OsString::from("1")),
            (
                OsString::from("GIT_CONFIG_KEY_0"),
                OsString::from("core.worktree"),
            ),
            (
                OsString::from("GIT_CONFIG_VALUE_0"),
                second.path().to_str().unwrap().into(),
            ),
            (
                OsString::from("GIT_CONFIG_PARAMETERS"),
                OsString::from("'core.worktree=/tmp/poisoned'"),
            ),
            (
                OsString::from("GIT_AUTHOR_NAME"),
                OsString::from("Configured Test Identity"),
            ),
        ];
        let mut command = super::super::mutation_command_with_environment(
            first.path(),
            &["--literal-pathspecs", "add", "--", "first.txt"],
            environment.into_iter(),
        );
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            names(&checked(first.path(), &["ls-files", "-z"])),
            ["first.txt"]
        );
        assert!(checked(second.path(), &["ls-files", "-z"]).is_empty());
        assert!(super::super::safe_mutation_environment_name(
            std::ffi::OsStr::new("GIT_AUTHOR_NAME")
        ));
        assert!(!super::super::safe_mutation_environment_name(
            std::ffi::OsStr::new("GIT_INDEX_FILE")
        ));
        assert!(!super::super::safe_mutation_environment_name(
            std::ffi::OsStr::new("GIT_CONFIG_COUNT")
        ));
        #[cfg(windows)]
        {
            assert!(!super::super::safe_mutation_environment_name(
                std::ffi::OsStr::new("gIt_DiR")
            ));
            assert!(!super::super::safe_mutation_environment_name(
                std::ffi::OsStr::new("Git_Config_Count")
            ));
            assert!(super::super::safe_mutation_environment_name(
                std::ffi::OsStr::new("git_author_name")
            ));
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn rebase_preflight_requires_untracked_paths_to_be_clean_without_identity_lookup() {
        use std::collections::BTreeSet;

        let mut untracked = BTreeSet::new();
        assert!(require_verified_rebase_paths(&untracked).is_ok());
        untracked.insert(b"unrelated-file.txt".to_vec());
        assert!(require_verified_rebase_paths(&untracked)
            .unwrap_err()
            .contains("use merge pull"));
    }
}
