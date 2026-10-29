use super::{checked, relative, spawn_observation, spawn_observation_with_stdin};
use crate::files::{directory, main_window};
use serde::Serialize;
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    path::Path,
    thread,
};
use tauri::Window;

const PAGE_SIZE: usize = 50;
pub(super) const PATCH_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitSummary {
    id: String,
    short_id: String,
    subject: String,
    author_name: String,
    authored_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    commits: Vec<CommitSummary>,
    tips: Vec<String>,
    has_more: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitFile {
    path: String,
    original_path: Option<String>,
    status: String,
    additions: Option<u64>,
    deletions: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDetails {
    commit: CommitSummary,
    author_email: String,
    committer_name: String,
    committer_email: String,
    committed_at: String,
    parents: Vec<String>,
    message: String,
    files: Vec<CommitFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitDiff {
    pub(super) patch: String,
    pub(super) truncated: bool,
}

fn object_id(id: &str) -> Result<(), String> {
    if !matches!(id.len(), 40 | 64) || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("A full commit hash is required.".into());
    }
    Ok(())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn history(
    root: &Path,
    tips: Option<Vec<String>>,
    skip: u32,
    path: Option<&str>,
) -> Result<HistoryPage, String> {
    // Freeze the starting revisions so new commits cannot shift subsequent pages.
    let tips = match tips {
        Some(tips) => tips,
        None => text(&checked(root, &["rev-list", "--all", "--no-walk"])?)
            .lines()
            .map(String::from)
            .collect(),
    };
    for tip in &tips {
        object_id(tip)?;
    }
    if tips.is_empty() {
        return Ok(HistoryPage {
            commits: vec![],
            tips,
            has_more: false,
        });
    }
    let count = format!("--max-count={}", PAGE_SIZE + 1);
    let skip = format!("--skip={skip}");
    let mut args = vec![
        "--literal-pathspecs",
        "log",
        "--date-order",
        "--no-decorate",
        "--no-notes",
        "--no-color",
        "--no-show-signature",
        "--encoding=UTF-8",
        "-z",
        "--format=%H%x00%h%x00%s%x00%an%x00%aI",
        &count,
        &skip,
    ];
    args.extend(tips.iter().map(String::as_str));
    args.push("--");
    if let Some(path) = path {
        if !path.is_empty() {
            relative(path)?;
            args.push(path);
        }
    }
    let bytes = checked(root, &args)?;
    let mut fields = bytes.split(|byte| *byte == 0);
    let mut commits = Vec::new();
    while let Some(id) = fields.next().filter(|id| !id.is_empty()) {
        let mut next = || {
            fields
                .next()
                .map(text)
                .ok_or("Cannot read the commit history.")
        };
        commits.push(CommitSummary {
            id: text(id),
            short_id: next()?,
            subject: next()?,
            author_name: next()?,
            authored_at: next()?,
        });
    }
    let has_more = commits.len() > PAGE_SIZE;
    commits.truncate(PAGE_SIZE);
    Ok(HistoryPage {
        commits,
        tips,
        has_more,
    })
}

fn metadata(root: &Path, id: &str) -> Result<CommitDetails, String> {
    object_id(id)?;
    let revision = format!("{id}^{{commit}}");
    let bytes = checked(
        root,
        &[
            "show",
            "--no-patch",
            "--no-notes",
            "--no-show-signature",
            "--no-color",
            "--encoding=UTF-8",
            "--format=format:%H%x00%h%x00%s%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%P%x00%B",
            &revision,
            "--",
        ],
    )?;
    let fields: Vec<_> = bytes.splitn(11, |byte| *byte == 0).map(text).collect();
    if fields.len() != 11 {
        return Err("Cannot read the commit details.".into());
    }
    Ok(CommitDetails {
        commit: CommitSummary {
            id: fields[0].clone(),
            short_id: fields[1].clone(),
            subject: fields[2].clone(),
            author_name: fields[3].clone(),
            authored_at: fields[5].clone(),
        },
        author_email: fields[4].clone(),
        committer_name: fields[6].clone(),
        committer_email: fields[7].clone(),
        committed_at: fields[8].clone(),
        parents: fields[9].split_whitespace().map(String::from).collect(),
        message: fields[10].clone(),
        files: vec![],
    })
}

fn diff_args<'a>(details: &'a CommitDetails, format: &'a str) -> Vec<&'a str> {
    let mut args = vec![
        "--literal-pathspecs",
        "diff-tree",
        "--root",
        "--no-commit-id",
        "-r",
        "-M",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-relative",
        "--ignore-submodules=none",
        format,
    ];
    if let Some(parent) = details.parents.first() {
        args.push(parent);
    }
    args.push(&details.commit.id);
    args
}

fn files(root: &Path, details: &CommitDetails) -> Result<Vec<CommitFile>, String> {
    let mut args = diff_args(details, "--name-status");
    args.extend(["-z", "--"]);
    let bytes = checked(root, &args)?;
    let mut entries = bytes.split(|byte| *byte == 0);
    let mut files = Vec::new();
    while let Some(status) = entries.next().filter(|entry| !entry.is_empty()) {
        let first = entries.next().ok_or("Cannot read the changed files.")?;
        let (path, original_path) = if matches!(status[0], b'R' | b'C') {
            (
                entries.next().ok_or("Cannot read a renamed file.")?,
                Some(text(first)),
            )
        } else {
            (first, None)
        };
        files.push(CommitFile {
            path: text(path),
            original_path,
            status: text(&status[..1]),
            additions: None,
            deletions: None,
        });
    }
    let mut args = diff_args(details, "--numstat");
    args.extend(["-z", "--"]);
    let bytes = checked(root, &args)?;
    let mut entries = bytes.split(|byte| *byte == 0);
    let mut stats = HashMap::new();
    while let Some(entry) = entries.next().filter(|entry| !entry.is_empty()) {
        let mut fields = entry.splitn(3, |byte| *byte == b'\t');
        let additions = text(fields.next().ok_or("Cannot read file statistics.")?)
            .parse::<u64>()
            .ok();
        let deletions = text(fields.next().ok_or("Cannot read file statistics.")?)
            .parse::<u64>()
            .ok();
        let mut path = fields.next().ok_or("Cannot read file statistics.")?;
        if path.is_empty() {
            entries
                .next()
                .ok_or("Cannot read renamed file statistics.")?;
            path = entries
                .next()
                .ok_or("Cannot read renamed file statistics.")?;
        }
        stats.insert(text(path), (additions, deletions));
    }
    for file in &mut files {
        (file.additions, file.deletions) =
            stats.remove(&file.path).ok_or("Missing file statistics.")?;
    }
    Ok(files)
}

fn details(root: &Path, id: &str) -> Result<CommitDetails, String> {
    let mut details = metadata(root, id)?;
    details.files = files(root, &details)?;
    Ok(details)
}

fn patch(
    root: &Path,
    id: &str,
    path: &str,
    original_path: Option<&str>,
) -> Result<CommitDiff, String> {
    relative(path)?;
    if let Some(original) = original_path {
        relative(original)?;
    }
    let details = metadata(root, id)?;
    let mut args = diff_args(&details, "--patch");
    args.extend([
        "--unified=3",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--output-indicator-new=+",
        "--output-indicator-old=-",
        "--output-indicator-context= ",
        "--",
        path,
    ]);
    if let Some(original) = original_path {
        args.push(original);
    }
    let bytes = patch_bytes(root, &args)?;
    Ok(limit_patch(&bytes))
}

pub(super) fn limit_patch(bytes: &[u8]) -> CommitDiff {
    let mut end = bytes.len().min(PATCH_LIMIT);
    let mut lines = 0;
    for (index, byte) in bytes[..end].iter().enumerate() {
        if *byte == b'\n' {
            lines += 1;
        }
        if lines == 20_000 {
            end = index + 1;
            break;
        }
    }
    CommitDiff {
        patch: text(&bytes[..end]),
        truncated: end < bytes.len(),
    }
}

pub(super) fn patch_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    bounded_bytes(root, args, PATCH_LIMIT)
}

pub(super) fn bounded_bytes(root: &Path, args: &[&str], limit: usize) -> Result<Vec<u8>, String> {
    bounded_bytes_inner(root, args, limit, None)
}

pub(super) fn bounded_bytes_with_input(
    root: &Path,
    args: &[&str],
    limit: usize,
    input: Vec<u8>,
) -> Result<Vec<u8>, String> {
    bounded_bytes_inner(root, args, limit, Some(input))
}

fn bounded_bytes_inner(
    root: &Path,
    args: &[&str],
    limit: usize,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let (mut child, _guard) = if input.is_some() {
        spawn_observation_with_stdin(root, args)?
    } else {
        spawn_observation(root, args)?
    };
    let input = input.map(|bytes| {
        let mut stdin = child.stdin.take().expect("piped Git stdin");
        thread::spawn(move || stdin.write_all(&bytes))
    });
    let mut stderr = child.stderr.take().expect("piped Git stderr");
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stderr).take(8192).read_to_end(&mut bytes);
        let _ = io::copy(&mut stderr, &mut io::sink());
        bytes
    });
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("piped Git stdout")
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes);
    let truncated = bytes.len() > limit;
    if truncated || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    let stderr = errors.join().unwrap_or_default();
    let input = input.map(|writer| {
        writer
            .join()
            .map_err(|_| "Cannot finish Git input.".to_string())
            .and_then(|result| result.map_err(|error| format!("Cannot write Git input: {error}")))
    });
    read.map_err(|error| format!("Cannot read Git content: {error}"))?;
    let status = status.map_err(|error| error.to_string())?;
    if !truncated && !status.success() {
        return Err(format!("Cannot read Git content: {}", text(&stderr).trim()));
    }
    if !truncated {
        input.transpose()?;
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn git_history(
    window: Window,
    root: String,
    tips: Option<Vec<String>>,
    skip: u32,
    path: Option<String>,
) -> Result<HistoryPage, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        history(&directory(&root)?, tips, skip, path.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_commit_details(
    window: Window,
    root: String,
    id: String,
) -> Result<CommitDetails, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || details(&directory(&root)?, &id))
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn git_commit_diff(
    window: Window,
    root: String,
    id: String,
    path: String,
    original_path: Option<String>,
) -> Result<CommitDiff, String> {
    main_window(&window)?;
    tauri::async_runtime::spawn_blocking(move || {
        patch(&directory(&root)?, &id, &path, original_path.as_deref())
    })
    .await
    .map_err(|error| error.to_string())?
}
