use super::{directory, editor::decode, inside, main_window};
use ignore::{gitignore::GitignoreBuilder, WalkBuilder};
use regex::RegexBuilder;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::Read,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tauri::{State, Window};

const FILE_LIMIT: u64 = 16 * 1024 * 1024;
const MATCH_LIMIT: usize = 1000;

#[derive(Default)]
pub struct ProjectSearch(pub Arc<AtomicU64>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    relative: String,
    line: usize,
    column: usize,
    length: usize,
    preview: String,
    preview_start: usize,
}

#[derive(Default, Serialize)]
pub struct SearchResults {
    matches: Vec<SearchMatch>,
    limited: bool,
    skipped: usize,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchOptions {
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    include_ignored: bool,
    include: String,
    exclude: String,
}

fn file_filter(
    root: &std::path::Path,
    patterns: &str,
) -> Result<ignore::gitignore::Gitignore, String> {
    if patterns.len() > 4096 || patterns.contains(['\r', '\n']) {
        return Err("File filters must be a single line of at most 4096 bytes.".into());
    }
    let mut builder = GitignoreBuilder::new(root);
    let mut depth = 0usize;
    let mut in_class = false;
    let mut escaped = false;
    for pattern in patterns.split(|character| {
        if escaped {
            escaped = false;
            return false;
        }
        match character {
            '\\' => escaped = true,
            '[' => in_class = true,
            ']' => in_class = false,
            '{' if !in_class => depth += 1,
            '}' if !in_class => depth = depth.saturating_sub(1),
            ',' if depth == 0 && !in_class => return true,
            _ => {}
        }
        false
    }) {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            continue;
        }
        if pattern.starts_with('!') {
            return Err("Use the include or exclude field instead of a ! prefix.".into());
        }
        let normalized = if let Some(path) = pattern.strip_prefix("./") {
            Some(format!("/{path}"))
        } else {
            pattern.starts_with('#').then(|| format!(r"\{pattern}"))
        };
        builder
            .add_line(None, normalized.as_deref().unwrap_or(pattern))
            .map_err(|error| format!("Invalid file filter: {error}"))?;
    }
    builder
        .build()
        .map_err(|error| format!("Invalid file filter: {error}"))
}

fn search(
    root: &str,
    relative: &str,
    query: &str,
    options: &SearchOptions,
    cancelled: impl Fn() -> bool,
) -> Result<SearchResults, String> {
    if query.is_empty() || query.len() > 1024 || query.contains(['\r', '\n']) {
        return Err("Enter a single-line search of 1–1024 bytes.".into());
    }
    let folder = inside(root, relative)?;
    if !folder.is_dir() {
        return Err("Choose a folder to search.".into());
    }
    let root = directory(root)?;
    let expression = if options.regex {
        query.to_owned()
    } else {
        regex::escape(query)
    };
    let expression = if options.whole_word {
        format!(r"\b{{start-half}}(?:{expression})\b{{end-half}}")
    } else {
        expression
    };
    let pattern = RegexBuilder::new(&expression)
        .case_insensitive(!options.case_sensitive)
        .build()
        .map_err(|error| error.to_string())?;
    let include = file_filter(&root, &options.include)?;
    let exclude = file_filter(&root, &options.exclude)?;
    let mut walk = WalkBuilder::new(folder);
    walk.hidden(false)
        .follow_links(false)
        .git_ignore(!options.include_ignored)
        .git_global(!options.include_ignored)
        .git_exclude(!options.include_ignored)
        .ignore(!options.include_ignored)
        .filter_entry(move |entry| {
            entry.file_name() != ".git"
                && !exclude
                    .matched_path_or_any_parents(
                        entry.path(),
                        entry.file_type().is_some_and(|kind| kind.is_dir()),
                    )
                    .is_ignore()
        });
    let started = Instant::now();
    let mut result = SearchResults::default();
    'files: for (visited, entry) in walk.build().enumerate() {
        if cancelled() {
            break;
        }
        if visited >= 100_000 || started.elapsed() > Duration::from_secs(15) {
            result.limited = true;
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                result.skipped += 1;
                continue;
            }
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        if !include.is_empty()
            && !include
                .matched_path_or_any_parents(entry.path(), false)
                .is_ignore()
        {
            continue;
        }
        // Re-resolve each entry so symlinks introduced during the walk cannot escape the project.
        let Some(relative) = entry
            .path()
            .strip_prefix(&root)
            .ok()
            .and_then(|path| path.to_str())
        else {
            result.skipped += 1;
            continue;
        };
        let read = || -> Result<Vec<u8>, String> {
            let path = inside(root.to_str().ok_or("Invalid project path")?, relative)?;
            let file = File::open(path).map_err(|error| error.to_string())?;
            let meta = file.metadata().map_err(|error| error.to_string())?;
            if !meta.is_file() || meta.len() > FILE_LIMIT {
                return Err("Skipped file".into());
            }
            let mut bytes = Vec::new();
            file.take(FILE_LIMIT + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() as u64 > FILE_LIMIT {
                return Err("Skipped file".into());
            }
            Ok(bytes)
        };
        let text = match read().ok().and_then(|bytes| decode(&bytes).ok()) {
            Some((text, _)) => text,
            None => {
                result.skipped += 1;
                continue;
            }
        };
        // Match the editor's normalization of CRLF and standalone CR line endings.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for (line_index, line) in text.split('\n').enumerate() {
            if cancelled() {
                break 'files;
            }
            if started.elapsed() > Duration::from_secs(15) {
                result.limited = true;
                break 'files;
            }
            for found in pattern.find_iter(line) {
                if cancelled() {
                    break 'files;
                }
                if result.matches.len() == MATCH_LIMIT
                    || started.elapsed() > Duration::from_secs(15)
                {
                    result.limited = true;
                    break 'files;
                }
                let start = line[..found.start()]
                    .char_indices()
                    .rev()
                    .nth(60)
                    .map_or(0, |(index, _)| index);
                let end = line[found.start()..]
                    .char_indices()
                    .nth(240)
                    .map_or(line.len(), |(index, _)| found.start() + index);
                result.matches.push(SearchMatch {
                    relative: relative.replace(std::path::MAIN_SEPARATOR, "/"),
                    line: line_index + 1,
                    column: line[..found.start()].encode_utf16().count() + 1,
                    length: found.as_str().encode_utf16().count(),
                    preview: line[start..end].to_owned(),
                    preview_start: line[..start].encode_utf16().count(),
                });
            }
        }
    }
    Ok(result)
}

#[tauri::command]
pub async fn search_project(
    window: Window,
    state: State<'_, ProjectSearch>,
    root: String,
    relative: String,
    query: String,
    options: SearchOptions,
) -> Result<SearchResults, String> {
    main_window(&window)?;
    let generation = state.0.clone();
    let current = generation.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn_blocking(move || {
        search(&root, &relative, &query, &options, || {
            generation.load(Ordering::SeqCst) != current
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn cancel_project_search(
    window: Window,
    state: State<'_, ProjectSearch>,
) -> Result<(), String> {
    main_window(&window)?;
    state.0.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn searches_scoped_text_with_unicode_editor_positions_and_ignores() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("src")).unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();
        fs::write(root.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(root.path().join("ignored.txt"), "Zażółć").unwrap();
        fs::write(root.path().join(".git/secret"), "Zażółć").unwrap();
        fs::write(
            root.path().join("src/main.txt"),
            "🦀 ZAŻÓŁĆ\r\nZażółć\rZażółć",
        )
        .unwrap();
        fs::write(root.path().join("src/binary"), b"Za\0\0").unwrap();
        let path = root.path().to_str().unwrap();
        let found = search(path, "src", "zażółć", &SearchOptions::default(), || {
            false
        })
        .unwrap();
        assert_eq!(found.matches.len(), 3);
        assert_eq!(found.matches[0].column, 4);
        assert_eq!(found.matches[0].length, 6);
        assert_eq!(found.matches[2].line, 3);
        assert_eq!(found.skipped, 1);
        assert_eq!(
            search(
                path,
                "",
                "Zażółć",
                &SearchOptions {
                    case_sensitive: true,
                    ..Default::default()
                },
                || false
            )
            .unwrap()
            .matches
            .len(),
            2
        );
        assert_eq!(
            search(
                path,
                "",
                "Zażółć",
                &SearchOptions {
                    case_sensitive: true,
                    include_ignored: true,
                    ..Default::default()
                },
                || false
            )
            .unwrap()
            .matches
            .len(),
            3
        );
        assert!(search(path, "../", "hello", &SearchOptions::default(), || false).is_err());
        assert!(search(
            path,
            "",
            "Zażółć",
            &SearchOptions {
                include_ignored: true,
                ..Default::default()
            },
            || true
        )
        .unwrap()
        .matches
        .is_empty());
    }

    #[test]
    fn bounds_results_and_reads_utf16() {
        let root = tempfile::tempdir().unwrap();
        let bytes: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain("hit".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        fs::write(root.path().join("wide.txt"), bytes).unwrap();
        let path = root.path().to_str().unwrap();
        assert_eq!(
            search(
                path,
                "",
                "hit",
                &SearchOptions {
                    case_sensitive: true,
                    include_ignored: true,
                    ..Default::default()
                },
                || false
            )
            .unwrap()
            .matches
            .len(),
            1
        );
        fs::write(root.path().join("many.txt"), "hit\n".repeat(1100)).unwrap();
        let found = search(
            path,
            "",
            "hit",
            &SearchOptions {
                case_sensitive: true,
                include_ignored: true,
                ..Default::default()
            },
            || false,
        )
        .unwrap();
        assert_eq!(found.matches.len(), MATCH_LIMIT);
        assert!(found.limited);
    }

    #[test]
    fn matches_words_regex_and_filters_without_bypassing_ignores() {
        let root = tempfile::tempdir().unwrap();
        for directory in ["src", "dist", ".git"] {
            fs::create_dir(root.path().join(directory)).unwrap();
        }
        fs::write(root.path().join(".gitignore"), "ignored.ts\n").unwrap();
        for file in [
            "src/main.ts",
            "src/main.test.ts",
            "src/view.tsx",
            "dist/main.ts",
            "ignored.ts",
            ".git/private.ts",
        ] {
            fs::write(
                root.path().join(file),
                "🦀 needle needles Needle needle_ zażółć zażółćmy\n",
            )
            .unwrap();
        }
        let path = root.path().to_str().unwrap();
        let mut options = SearchOptions {
            include: "src/*.{ts,tsx}".into(),
            exclude: "**/*.test.ts, dist".into(),
            whole_word: true,
            ..Default::default()
        };
        let found = search(path, "", "needle", &options, || false).unwrap();
        assert_eq!(found.matches.len(), 4);
        assert!(found
            .matches
            .iter()
            .all(|found| matches!(found.relative.as_str(), "src/main.ts" | "src/view.tsx")));
        assert_eq!(found.matches[0].column, 4);
        options.case_sensitive = true;
        assert_eq!(
            search(path, "", "needle", &options, || false)
                .unwrap()
                .matches
                .len(),
            2
        );
        assert_eq!(
            search(path, "", "zażółć", &options, || false)
                .unwrap()
                .matches
                .len(),
            2
        );
        assert_eq!(
            search(path, "", "🦀", &options, || false)
                .unwrap()
                .matches
                .len(),
            2
        );
        assert!(search(path, "", "ne{2}dle", &options, || false)
            .unwrap()
            .matches
            .is_empty());
        options.regex = true;
        assert_eq!(
            search(path, "", "ne{2}dle", &options, || false)
                .unwrap()
                .matches
                .len(),
            2
        );
        assert!(search(path, "", "[", &options, || false).is_err());
        options.whole_word = false;
        options.include = "./src".into();
        assert_eq!(
            search(path, "src", "^🦀", &options, || false)
                .unwrap()
                .matches[0]
                .length,
            2
        );
        options.include = "*.ts".into();
        let found = search(path, "", "needle", &options, || false).unwrap();
        assert_eq!(found.matches.len(), 3);
        options.include_ignored = true;
        assert_eq!(
            search(path, "", "needle", &options, || false)
                .unwrap()
                .matches
                .len(),
            6
        );
        options.include = "[z-a]".into();
        assert!(search(path, "", "needle", &options, || false).is_err());
        options.include = "a".repeat(4097);
        assert!(search(path, "", "needle", &options, || false).is_err());
        for (pattern, file) in [
            ("#notes", "#notes"),
            (r"file\,name.ts", "file,name.ts"),
            ("[{].ts, *.md", "{.ts"),
        ] {
            let filter = file_filter(root.path(), pattern).unwrap();
            assert!(filter
                .matched_path_or_any_parents(root.path().join(file), false)
                .is_ignore());
        }
    }
}
