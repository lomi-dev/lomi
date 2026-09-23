//! Scoped search shares the UI's matching/filter/encoding semantics, but all
//! traversal and bytes come from broker-pinned NOFOLLOW descriptors.
use super::{decode, file_filter, line_match, query_pattern, SearchOptions};
use lomi_control_core::{broker::FileSearchBatch, project_files::ProjectDirectory};
use lomi_control_protocol::{files::*, ErrorCode};
use sha2::{Digest, Sha256};

pub(crate) fn search(
    directory: &ProjectDirectory,
    input: &FilesSearchInput,
    check: &dyn Fn() -> Result<(), ErrorCode>,
) -> Result<FileSearchBatch, ErrorCode> {
    check()?;
    let options = SearchOptions {
        case_sensitive: input.query.case_sensitive,
        whole_word: input.query.whole_word,
        regex: input.query.regex,
        include: input.query.include.clone(),
        exclude: input.query.exclude.clone(),
        ..Default::default()
    };
    let pattern =
        query_pattern(&input.query.text, &options).map_err(|_| ErrorCode::UnsupportedCapability)?;
    // The synthetic root only anchors glob matching; it is never opened.
    let root = std::path::Path::new("/");
    let include =
        file_filter(root, &options.include).map_err(|_| ErrorCode::UnsupportedCapability)?;
    let exclude =
        file_filter(root, &options.exclude).map_err(|_| ErrorCode::UnsupportedCapability)?;
    let mut result = FileSearchBatch::default();
    let mut directories = vec![(input.relative_directory.clone(), 0_u16)];
    let (mut visited, mut read_bytes, mut output_bytes, mut queue_bytes) = (0, 0_u64, 0, 0_usize);
    'scan: while let Some((relative, depth)) = directories.pop() {
        check()?;
        queue_bytes = queue_bytes.saturating_sub(relative.len());
        let entries = match directory.list(&relative, check) {
            Ok(d) => d.entries,
            Err(e)
                if matches!(
                    e,
                    ErrorCode::TargetNotFound
                        | ErrorCode::RevisionConflict
                        | ErrorCode::ResourceExhausted
                ) && depth > 0 =>
            {
                result.skipped += 1;
                continue;
            }
            Err(e) => return Err(e),
        };
        for entry in entries {
            check()?;
            visited += 1;
            if visited > 10000 {
                result.limited = true;
                break 'scan;
            }
            let path = root.join(&entry.relative_path);
            let is_dir = entry.kind == FileEntryKind::Directory;
            if exclude
                .matched_path_or_any_parents(&path, is_dir)
                .is_ignore()
            {
                continue;
            }
            if is_dir {
                if depth >= 32
                    || directories.len() >= 2048
                    || queue_bytes + entry.relative_path.len() > 1024 * 1024
                {
                    result.limited = true;
                    continue;
                }
                queue_bytes += entry.relative_path.len();
                directories.push((entry.relative_path, depth + 1));
                continue;
            }
            if !include.is_empty()
                && !include
                    .matched_path_or_any_parents(&path, false)
                    .is_ignore()
            {
                continue;
            }
            let remaining = 32 * 1024 * 1024 - read_bytes;
            if entry
                .byte_length
                .as_deref()
                .and_then(|n| n.parse::<u64>().ok())
                .is_some_and(|n| n <= 4 * 1024 * 1024 && n > remaining)
            {
                result.limited = true;
                break 'scan;
            }
            let bytes = match directory
                .open_file(&entry.relative_path, 4 * 1024 * 1024)
                .and_then(|f| f.read_bytes(remaining.min(4 * 1024 * 1024), check))
            {
                Ok(b) => b,
                Err(e @ (ErrorCode::ControlRevoked | ErrorCode::DeadlineExceeded)) => {
                    return Err(e)
                }
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            read_bytes += bytes.len() as u64;
            if read_bytes > 32 * 1024 * 1024 {
                result.limited = true;
                break 'scan;
            }
            check()?;
            let (text, _) = match decode(&bytes) {
                Ok(t) => t,
                Err(_) => {
                    result.skipped += 1;
                    continue;
                }
            };
            result.files_scanned += 1;
            let revision = format!("{:x}", Sha256::digest(&bytes));
            let text = text.replace("\r\n", "\n").replace('\r', "\n");
            for (line_index, line) in text.split('\n').enumerate() {
                check()?;
                // Bounds each regex invocation and UTF-16 prefix scan independently.
                if line.len() > 32768 {
                    result.skipped += 1;
                    continue;
                }
                for found in pattern.find_iter(line) {
                    check()?;
                    let m = line_match(&entry.relative_path, line_index, line, found);
                    let m = FileSearchMatch {
                        relative_path: m.relative,
                        disk_revision: revision.clone(),
                        line: m.line as u32,
                        column: m.column as u32,
                        length: m.length as u32,
                        preview: m.preview,
                        preview_start_utf16: m.preview_start as u32,
                    };
                    let size = serde_json::to_vec(&m)
                        .map_err(|_| ErrorCode::ResourceExhausted)?
                        .len()
                        + 1;
                    if result.matches.len() >= 256 || output_bytes + size > 131072 {
                        result.limited = true;
                        break 'scan;
                    }
                    output_bytes += size;
                    result.matches.push(m);
                }
            }
        }
    }
    check()?;
    directory.check()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};
    fn input() -> FilesSearchInput {
        FilesSearchInput {
            workspace_id: "w".into(),
            relative_directory: String::new(),
            query: FileSearchQuery {
                text: "zażółć".into(),
                ..Default::default()
            },
            limit: 100,
            cursor: None,
        }
    }
    #[test]
    fn scoped_search_reuses_unicode_positions_and_never_follows_links_or_secret_paths() {
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        let source = "🙂 ZAŻÓŁĆ\r\nZażółć\rZażółć";
        fs::write(root.path().join("src/a.txt"), source).unwrap();
        fs::write(root.path().join(".env.fixture"), "zażółć PRIVATE").unwrap();
        fs::write(root.path().join("binary"), b"za\0\0").unwrap();
        fs::write(external.path().join("private"), "zażółć PRIVATE").unwrap();
        symlink(external.path(), root.path().join("outside")).unwrap();
        symlink(root.path().join("src/a.txt"), root.path().join("link")).unwrap();
        let dir = ProjectDirectory::open(&root.path().canonicalize().unwrap()).unwrap();
        let result = search(&dir, &input(), &|| Ok(())).unwrap();
        assert_eq!(result.matches.len(), 3);
        assert_eq!(result.skipped, 1);
        assert_eq!(result.matches[0].column, 4);
        assert_eq!(result.matches[0].length, 6);
        assert_eq!(result.matches[2].line, 3);
        assert_eq!(
            result.matches[0].disk_revision,
            format!("{:x}", Sha256::digest(source.as_bytes()))
        );
        assert!(result
            .matches
            .iter()
            .all(|m| m.relative_path == "src/a.txt"));
        let mut q = input();
        q.query.exclude = "src/**".into();
        assert!(search(&dir, &q, &|| Ok(())).unwrap().matches.is_empty());
        q.query.exclude.clear();
        q.query.include = "*.rs".into();
        assert!(search(&dir, &q, &|| Ok(())).unwrap().matches.is_empty());
        q.query.include = "*.txt".into();
        q.query.case_sensitive = true;
        assert!(search(&dir, &q, &|| Ok(())).unwrap().matches.is_empty());
        q.query.text = "[".into();
        q.query.regex = true;
        assert!(matches!(
            search(&dir, &q, &|| Ok(())),
            Err(ErrorCode::UnsupportedCapability)
        ));
        assert!(matches!(
            search(&dir, &input(), &|| Err(ErrorCode::ControlRevoked)),
            Err(ErrorCode::ControlRevoked)
        ));
    }
    #[test]
    fn regex_match_count_long_lines_and_encoded_result_size_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), "x ".repeat(1000)).unwrap();
        let dir = ProjectDirectory::open(&root.path().canonicalize().unwrap()).unwrap();
        let mut q = input();
        q.query.text = "x".into();
        q.query.whole_word = true;
        let found = search(&dir, &q, &|| Ok(())).unwrap();
        assert!(found.limited);
        assert!(found.matches.len() <= 256);
        fs::write(root.path().join("a.txt"), "x".repeat(32769)).unwrap();
        let found = search(&dir, &q, &|| Ok(())).unwrap();
        assert!(found.matches.is_empty());
        assert_eq!(found.skipped, 1);
        let mut bytes = vec![0xff, 0xfe];
        for unit in "🙂 x".encode_utf16() {
            bytes.extend(unit.to_le_bytes());
        }
        fs::write(root.path().join("a.txt"), bytes).unwrap();
        let found = search(&dir, &q, &|| Ok(())).unwrap();
        assert_eq!(found.matches[0].column, 4);
    }
}
