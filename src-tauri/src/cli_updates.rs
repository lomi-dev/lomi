//! Detects outdated agent CLIs. Only package names (npm, PyPI) leave the
//! machine; Homebrew freshness comes from brew's local state. Updating is
//! always a user-initiated command in a visible terminal.
use crate::{cli_catalog::TitleCli, cli_launch, files::main_window, terminal::Shells};
use futures_util::future::join_all;
use serde::Serialize;
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::{State, Window};
use tokio::{io::AsyncReadExt, process::Command};

const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const TIMEOUT: Duration = Duration::from_secs(5);
const BREW_TIMEOUT: Duration = Duration::from_secs(20);
const RESPONSE_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Source {
    Npm(&'static str),
    Pypi(&'static str),
    Cursor,
}

/// Where each CLI's latest version is published. agy and kiro have no known
/// version feed, so they are not checked.
const SOURCES: [(TitleCli, Source); 14] = [
    (TitleCli::Claude, Source::Npm("@anthropic-ai/claude-code")),
    (TitleCli::Codex, Source::Npm("@openai/codex")),
    (TitleCli::Gemini, Source::Npm("@google/gemini-cli")),
    (TitleCli::Copilot, Source::Npm("@github/copilot")),
    (TitleCli::Qwen, Source::Npm("@qwen-code/qwen-code")),
    (TitleCli::Opencode, Source::Npm("opencode-ai")),
    (TitleCli::Kilo, Source::Npm("@kilocode/cli")),
    (TitleCli::Pi, Source::Npm("@mariozechner/pi-coding-agent")),
    (TitleCli::Grok, Source::Npm("@vibe-kit/grok-cli")),
    (TitleCli::Openclaw, Source::Npm("openclaw")),
    (TitleCli::Kimi, Source::Npm("@moonshot-ai/kimi-code")),
    (TitleCli::Vibe, Source::Pypi("mistral-vibe")),
    (TitleCli::Hermes, Source::Pypi("hermes-agent")),
    (TitleCli::Cursor, Source::Cursor),
];

static CACHE: Mutex<Option<(Instant, Vec<CliUpdate>)>> = Mutex::new(None);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliUpdate {
    cli: TitleCli,
    name: String,
    installed: String,
    latest: String,
    /// None when the installer is unknown: the user updates it themselves.
    command: Option<String>,
}

#[tauri::command]
pub async fn check_cli_updates(
    window: Window,
    shells: State<'_, Shells>,
    force: Option<bool>,
) -> Result<Vec<CliUpdate>, String> {
    main_window(&window)?;
    if !force.unwrap_or(false) {
        if let Some((at, found)) = &*CACHE.lock().unwrap() {
            if at.elapsed() < CACHE_TTL {
                return Ok(found.clone());
            }
        }
    }
    let shells = shells.inner().clone();
    let paths = tauri::async_runtime::spawn_blocking(move || {
        cli_launch::installed_local_cli_paths(&shells)
    })
    .await
    .map_err(|error| error.to_string())??;
    let (found, complete) = collect(&client()?, paths).await;
    // A failed lookup must not hide updates for the whole cache period.
    if complete {
        *CACHE.lock().unwrap() = Some((Instant::now(), found.clone()));
    }
    Ok(found)
}

fn client() -> Result<reqwest::Client, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .user_agent("Lomi")
        .build()
        .map_err(|error| error.to_string())
}

/// Checks every supported installed CLI; the flag is false after a transient failure.
async fn collect(
    client: &reqwest::Client,
    paths: HashMap<TitleCli, PathBuf>,
) -> (Vec<CliUpdate>, bool) {
    // At most one task per table entry, so concurrency is bounded by the table.
    let targets = join_all(SOURCES.iter().filter_map(|(cli, source)| {
        let path = paths.get(cli)?.clone();
        Some(async move {
            let canonical = tokio::fs::canonicalize(&path).await.ok()?;
            Some((
                *cli,
                *source,
                path,
                install_method(*cli, &canonical, *source),
            ))
        })
    }))
    .await
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    // One `brew outdated` per Homebrew prefix covers all of its CLIs.
    let brews = targets
        .iter()
        .filter_map(|target| match &target.3 {
            Method::Brew { brew, .. } if brew.is_file() => Some(brew.clone()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let outdated = join_all(brews.into_iter().map(|brew| async move {
        let result = brew_outdated(&brew).await;
        (brew, result)
    }))
    .await
    .into_iter()
    .collect::<HashMap<_, _>>();
    let results = join_all(targets.iter().map(|(cli, source, path, method)| {
        check_one(client, &outdated, *cli, *source, path, method)
    }))
    .await;
    let mut found = Vec::new();
    let mut complete = true;
    for result in results {
        match result {
            Ok(update) => found.extend(update),
            Err(_) => complete = false,
        }
    }
    (found, complete)
}

/// Err means a transient failure worth retrying; Ok(None) means nothing to offer.
async fn check_one(
    client: &reqwest::Client,
    outdated: &HashMap<PathBuf, Result<Outdated, String>>,
    cli: TitleCli,
    source: Source,
    path: &Path,
    method: &Method,
) -> Result<Option<CliUpdate>, String> {
    let version = installed_version(cli, path).await;
    let update = |installed: String, latest: String, command: Option<String>| CliUpdate {
        cli,
        name: cli.name().to_owned(),
        installed,
        latest,
        command,
    };
    if let Method::Brew { cask, name, brew } = method {
        let Some(result) = outdated.get(brew) else {
            return Ok(None);
        };
        let list = result.as_ref().map_err(Clone::clone)?;
        let entries = if *cask { &list.casks } else { &list.formulae };
        let Some((installed, current)) = entries.get(name) else {
            return Ok(None);
        };
        let command = if *cask {
            format!("brew upgrade --cask {name}")
        } else {
            format!("brew upgrade {name}")
        };
        return Ok(Some(update(
            version.unwrap_or_else(|| installed.clone()),
            current.clone(),
            Some(command),
        )));
    }
    let Method::Run(command) = method else {
        return Ok(None);
    };
    let Some(installed) = version else {
        return Ok(None);
    };
    let latest = latest_version(client, source).await?;
    Ok(is_newer(cli, &installed, &latest).then(|| update(installed, latest, command.clone())))
}

/// Runs a command with bounded output and time; the child is killed on drop.
async fn output_of(mut command: Command, limit: u64, timeout: Duration) -> Option<Vec<u8>> {
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command.spawn().ok()?;
    let mut stdout = child.stdout.take()?.take(limit);
    let mut output = Vec::new();
    tokio::time::timeout(timeout, stdout.read_to_end(&mut output))
        .await
        .ok()?
        .ok()?;
    Some(output)
}

async fn installed_version(cli: TitleCli, path: &Path) -> Option<String> {
    let mut command = Command::new(path);
    command.arg("--version");
    // Node-based shims find `node` next to themselves in nvm/Homebrew layouts.
    if let Some(dir) = path.parent() {
        let mut dirs = vec![dir.to_path_buf()];
        dirs.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        if let Ok(joined) = std::env::join_paths(dirs) {
            command.env("PATH", joined);
        }
    }
    let output = output_of(command, 4096, TIMEOUT).await?;
    let text = String::from_utf8_lossy(&output);
    if cli == TitleCli::Cursor {
        let (nums, hash) = split_cursor(&text)?;
        return Some(format!("{}-{hash}", join(&nums)));
    }
    parse_version(&text).map(|nums| join(&nums))
}

async fn fetch(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let mut response = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{url}: {}", response.status()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        body.extend_from_slice(&chunk);
        if body.len() > RESPONSE_LIMIT {
            return Err("The version feed response is too large.".into());
        }
    }
    Ok(body)
}

async fn latest_version(client: &reqwest::Client, source: Source) -> Result<String, String> {
    let json = |body: &[u8]| serde_json::from_slice::<serde_json::Value>(body);
    let missing = || "The version feed has no version.".to_owned();
    match source {
        Source::Npm(package) => {
            let body = fetch(
                client,
                &format!("https://registry.npmjs.org/{package}/latest"),
            )
            .await?;
            let value = json(&body).map_err(|e| e.to_string())?;
            value["version"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(missing)
        }
        Source::Pypi(package) => {
            let body = fetch(client, &format!("https://pypi.org/pypi/{package}/json")).await?;
            let value = json(&body).map_err(|e| e.to_string())?;
            value["info"]["version"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(missing)
        }
        Source::Cursor => {
            let body = fetch(client, "https://cursor.com/install").await?;
            cursor_latest(&String::from_utf8_lossy(&body)).ok_or_else(missing)
        }
    }
}

/// Version dir in `downloads.cursor.com/lab/<date>-<hash>/` of the install script.
fn cursor_latest(script: &str) -> Option<String> {
    let rest = script.split_once("downloads.cursor.com/lab/")?.1;
    let version = rest.split('/').next()?;
    split_cursor(version).map(|_| version.to_owned())
}

/// First dotted-numeric version with at least three parts, covering semver
/// ("0.172.0") and calendar ("2026.11.10") forms. Suffixes are ignored.
fn parse_version(text: &str) -> Option<Vec<u64>> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .find_map(|token| {
            let parts = token
                .trim_matches('.')
                .split('.')
                .map(|part| part.parse().ok())
                .collect::<Option<Vec<u64>>>()?;
            (parts.len() >= 3).then_some(parts)
        })
}

fn join(nums: &[u64]) -> String {
    nums.iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn compare(a: &[u64], b: &[u64]) -> Ordering {
    (0..a.len().max(b.len()))
        .map(|i| {
            a.get(i)
                .copied()
                .unwrap_or(0)
                .cmp(&b.get(i).copied().unwrap_or(0))
        })
        .find(|ordering| ordering.is_ne())
        .unwrap_or(Ordering::Equal)
}

/// Cursor's `<year>.<month>.<day>-<hash>` build id as (date, hash).
fn split_cursor(text: &str) -> Option<(Vec<u64>, &str)> {
    text.split_whitespace().find_map(|token| {
        let (date, hash) = token.split_once('-')?;
        let hex = !hash.is_empty() && hash.bytes().all(|b| b.is_ascii_hexdigit());
        hex.then_some(())?;
        Some((parse_version(date)?, hash))
    })
}

fn is_newer(cli: TitleCli, installed: &str, latest: &str) -> bool {
    if cli == TitleCli::Cursor {
        let (Some((old, old_hash)), Some((new, new_hash))) =
            (split_cursor(installed), split_cursor(latest))
        else {
            return false;
        };
        // Same-day builds differ only by hash; the feed's build is the reference.
        return match compare(&new, &old) {
            Ordering::Greater => true,
            Ordering::Equal => !(new_hash.starts_with(old_hash) || old_hash.starts_with(new_hash)),
            Ordering::Less => false,
        };
    }
    matches!(
        (parse_version(installed), parse_version(latest)),
        (Some(old), Some(new)) if compare(&new, &old) == Ordering::Greater
    )
}

enum Method {
    Brew {
        cask: bool,
        name: String,
        brew: PathBuf,
    },
    /// Update command, or None when the installer is unknown.
    Run(Option<String>),
}

/// Names that reach a command line come from install paths, so keep them tame.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-@+".contains(c))
}

fn self_update(cli: TitleCli) -> Option<&'static str> {
    match cli {
        TitleCli::Claude => Some("claude update"),
        TitleCli::Codex => Some("codex update"),
        TitleCli::Kimi => Some("kimi upgrade"),
        TitleCli::Cursor => Some("cursor-agent update"),
        TitleCli::Opencode => Some("opencode upgrade"),
        _ => None,
    }
}

/// Maps the canonical install location to its update mechanism.
fn install_method(cli: TitleCli, canonical: &Path, source: Source) -> Method {
    let path = canonical.to_string_lossy();
    for (marker, cask) in [("/Cellar/", false), ("/Caskroom/", true)] {
        if let Some((prefix, rest)) = path.split_once(marker) {
            let name = rest.split('/').next().unwrap_or_default();
            if safe_name(name) {
                return Method::Brew {
                    cask,
                    name: name.to_owned(),
                    brew: Path::new(prefix).join("bin/brew"),
                };
            }
        }
    }
    let segment = |marker: &str| {
        path.split_once(marker)
            .and_then(|(_, rest)| rest.split('/').next())
            .filter(|name| safe_name(name))
    };
    let native = [
        (TitleCli::Claude, "/.local/share/claude/"),
        (TitleCli::Codex, "/.codex/packages/"),
        (TitleCli::Kimi, "/.kimi-code/"),
        (TitleCli::Cursor, "/.local/share/cursor-agent/"),
    ];
    if native
        .iter()
        .any(|(native_cli, marker)| *native_cli == cli && path.contains(marker))
    {
        return Method::Run(self_update(cli).map(str::to_owned));
    }
    if let Some(name) = segment("/pipx/venvs/") {
        return Method::Run(Some(format!("pipx upgrade {name}")));
    }
    if let Some(name) = segment("/uv/tools/") {
        return Method::Run(Some(format!("uv tool upgrade {name}")));
    }
    if let Source::Npm(package) = source {
        // pnpm and bun globals also live under node_modules, so test them first.
        if path.contains("/pnpm/global/") {
            return Method::Run(Some(format!("pnpm add -g {package}@latest")));
        }
        if path.contains("/.bun/install/global/") {
            return Method::Run(Some(format!("bun add -g {package}@latest")));
        }
        if path.contains("/node_modules/") {
            return Method::Run(Some(format!("npm install -g {package}@latest")));
        }
    }
    Method::Run(self_update(cli).map(str::to_owned))
}

/// Outdated Homebrew packages by name: (installed, current) versions.
#[derive(Clone, Default)]
struct Outdated {
    formulae: HashMap<String, (String, String)>,
    casks: HashMap<String, (String, String)>,
}

async fn brew_outdated(brew: &Path) -> Result<Outdated, String> {
    let mut command = Command::new(brew);
    command
        .args(["outdated", "--json=v2"])
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("HOMEBREW_NO_ANALYTICS", "1");
    // The exit status is not trusted: only parseable output counts.
    let output = output_of(command, 4 * 1024 * 1024, BREW_TIMEOUT)
        .await
        .ok_or("Cannot run brew outdated.")?;
    parse_outdated(&output)
}

fn parse_outdated(json: &[u8]) -> Result<Outdated, String> {
    let value: serde_json::Value = serde_json::from_slice(json).map_err(|e| e.to_string())?;
    let section = |key: &str| {
        value[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                let name = entry["name"].as_str()?.to_owned();
                let current = entry["current_version"].as_str()?.to_owned();
                let installed = match &entry["installed_versions"] {
                    serde_json::Value::String(version) => version.clone(),
                    serde_json::Value::Array(list) => list.first()?.as_str()?.to_owned(),
                    _ => String::new(),
                };
                Some((name, (installed, current)))
            })
            .collect()
    };
    Ok(Outdated {
        formulae: section("formulae"),
        casks: section("casks"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cli: TitleCli, path: &str, source: Source) -> Option<String> {
        match install_method(cli, Path::new(path), source) {
            Method::Run(command) => command,
            Method::Brew { .. } => panic!("unexpected Homebrew install"),
        }
    }

    #[test]
    fn parses_real_version_output() {
        assert_eq!(
            parse_version("2.1.332 (Claude Code)"),
            Some(vec![2, 1, 332])
        );
        assert_eq!(parse_version("codex-cli 0.172.0"), Some(vec![0, 172, 0]));
        assert_eq!(parse_version("0.71.1\n"), Some(vec![0, 71, 1]));
        assert_eq!(parse_version("v1.2.3-beta.1"), Some(vec![1, 2, 3]));
        assert_eq!(parse_version("2026.11.10"), Some(vec![2026, 11, 10]));
        assert_eq!(parse_version("no version"), None);
        assert_eq!(parse_version("1.2"), None);
    }

    #[test]
    fn compares_semver_and_calendar_versions() {
        let newer = |a, b| is_newer(TitleCli::Claude, a, b);
        assert!(newer("0.9.9", "0.10.0"));
        assert!(newer("2.1.33", "2.1.332"));
        assert!(newer("2026.11.10", "2026.12.1"));
        assert!(newer("1.2.3", "1.2.3.1"));
        assert!(newer("2.7.2", "2.8.1"));
        assert!(!newer("1.0.0", "1.0.0"));
        assert!(!newer("1.0.0", "1.0.0.0"));
        assert!(!newer("2.0.0", "1.9.9"));
        assert!(newer("1.0.0", "1.0.1-rc.1"));
    }

    #[test]
    fn handles_cursor_build_ids() {
        let script = "curl -fsSL https://downloads.cursor.com/lab/2026.11.25-abc1234/darwin/arm64/agent.tar.gz";
        assert_eq!(cursor_latest(script).as_deref(), Some("2026.11.25-abc1234"));
        assert_eq!(cursor_latest("nothing"), None);
        let newer = |a, b| is_newer(TitleCli::Cursor, a, b);
        assert!(newer("2026.11.22-e9042b2", "2026.11.25-abc1234"));
        assert!(newer("2026.11.25-e9042b2", "2026.11.25-abc1234"));
        assert!(!newer("2026.11.25-abc1234", "2026.11.25-abc1234567"));
        assert!(!newer("2026.12.1-abc1234", "2026.11.25-e9042b2"));
    }

    #[test]
    fn maps_homebrew_installs() {
        let Method::Brew { cask, name, brew } = install_method(
            TitleCli::Gemini,
            Path::new("/opt/homebrew/Cellar/gemini-cli/0.9.0/libexec/lib/node_modules/@google/gemini-cli/a.js"),
            Source::Npm("@google/gemini-cli"),
        ) else {
            panic!("expected Homebrew");
        };
        assert_eq!((cask, name.as_str()), (false, "gemini-cli"));
        assert_eq!(brew, Path::new("/opt/homebrew/bin/brew"));
        let Method::Brew { cask, name, brew } = install_method(
            TitleCli::Claude,
            Path::new("/usr/local/Caskroom/claude-code/2.1.0/claude"),
            Source::Npm("x"),
        ) else {
            panic!("expected Homebrew");
        };
        assert_eq!((cask, name.as_str()), (true, "claude-code"));
        assert_eq!(brew, Path::new("/usr/local/bin/brew"));
        // npm with Homebrew's node is not a Homebrew-managed package.
        assert_eq!(
            run(
                TitleCli::Gemini,
                "/opt/homebrew/lib/node_modules/@google/gemini-cli/a.js",
                Source::Npm("@google/gemini-cli")
            )
            .as_deref(),
            Some("npm install -g @google/gemini-cli@latest")
        );
    }

    #[test]
    fn maps_install_location_to_update_command() {
        let npm = Source::Npm("pkg");
        let cases = [
            (
                TitleCli::Claude,
                "/Users/a/.local/share/claude/versions/2.1.332",
                "claude update",
            ),
            (
                TitleCli::Codex,
                "/Users/a/.codex/packages/standalone/releases/0.1/codex",
                "codex update",
            ),
            (
                TitleCli::Kimi,
                "/Users/a/.kimi-code/bin/kimi",
                "kimi upgrade",
            ),
            (
                TitleCli::Cursor,
                "/Users/a/.local/share/cursor-agent/versions/1/cursor-agent",
                "cursor-agent update",
            ),
            (
                TitleCli::Opencode,
                "/Users/a/.opencode/bin/opencode",
                "opencode upgrade",
            ),
            (
                TitleCli::Gemini,
                "/usr/local/lib/node_modules/@google/gemini-cli/dist/index.js",
                "npm install -g pkg@latest",
            ),
            (
                TitleCli::Gemini,
                "/Users/a/Library/pnpm/global/5/node_modules/@google/gemini-cli/a.js",
                "pnpm add -g pkg@latest",
            ),
            (
                TitleCli::Gemini,
                "/Users/a/.bun/install/global/node_modules/@google/gemini-cli/a.js",
                "bun add -g pkg@latest",
            ),
        ];
        for (cli, path, command) in cases {
            assert_eq!(run(cli, path, npm).as_deref(), Some(command), "{path}");
        }
        let pypi = Source::Pypi("mistral-vibe");
        assert_eq!(
            run(
                TitleCli::Vibe,
                "/Users/a/.local/pipx/venvs/mistral-vibe/bin/vibe",
                pypi
            )
            .as_deref(),
            Some("pipx upgrade mistral-vibe")
        );
        assert_eq!(
            run(
                TitleCli::Vibe,
                "/Users/a/.local/share/uv/tools/mistral-vibe/bin/vibe",
                pypi
            )
            .as_deref(),
            Some("uv tool upgrade mistral-vibe")
        );
        assert_eq!(run(TitleCli::Pi, "/Users/a/bin/pi", npm), None);
        assert_eq!(run(TitleCli::Vibe, "/Users/a/bin/vibe", pypi), None);
        assert_eq!(
            run(TitleCli::Vibe, "/x/pipx/venvs/a;rm/bin/vibe", pypi),
            None
        );
    }

    #[test]
    fn parses_brew_outdated_json() {
        let json = br#"{"formulae":[{"name":"gemini-cli","installed_versions":["0.9.0"],"current_version":"0.10.0"}],
            "casks":[{"name":"claude-code","installed_versions":"2.1.0","current_version":"2.2.0"}]}"#;
        let outdated = parse_outdated(json).unwrap();
        assert_eq!(
            outdated.formulae["gemini-cli"],
            ("0.9.0".to_owned(), "0.10.0".to_owned())
        );
        assert_eq!(
            outdated.casks["claude-code"],
            ("2.1.0".to_owned(), "2.2.0".to_owned())
        );
        assert!(parse_outdated(b"not json").is_err());
        assert!(parse_outdated(b"{}").unwrap().formulae.is_empty());
    }

    /// Real run against this machine: `cargo test cli_updates -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn real_check_of_installed_clis() {
        let paths = SOURCES
            .iter()
            .filter_map(|(cli, _)| {
                let name = match cli {
                    TitleCli::Cursor => "cursor-agent".to_owned(),
                    other => format!("{other:?}").to_lowercase(),
                };
                Some((*cli, lomi_chat::cli_bin::resolve(&name)?))
            })
            .collect::<HashMap<_, _>>();
        for (cli, path) in &paths {
            println!("found {cli:?} at {}", path.display());
        }
        let (found, complete) = collect(&client().unwrap(), paths).await;
        println!("complete: {complete}");
        for update in found {
            println!(
                "{:?}: {} -> {} via {:?}",
                update.cli, update.installed, update.latest, update.command
            );
        }
    }
}
