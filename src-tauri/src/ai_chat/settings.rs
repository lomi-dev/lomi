//! Chat provider registry: types, trust-boundary validation and the settings file.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub(crate) const SETTINGS_LIMIT: u64 = 256 * 1024;
pub(crate) const KEY_LIMIT: usize = 4096;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ApiKind {
    Openai,
    Anthropic,
    /// Google AI Studio through its OpenAI-compatible endpoint.
    Google,
    OpenaiCompatible,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModelSource {
    Fetched,
    Manual,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SendKey {
    Enter,
    ModEnter,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelEntry {
    pub id: String,
    pub source: ModelSource,
    /// Hidden from model pickers when false; list order is the picker order.
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub favorite: bool,
}

fn enabled_default() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApiProvider {
    pub instance_id: String,
    pub kind: ApiKind,
    pub label: String,
    pub accent_color: Option<String>,
    pub enabled: bool,
    pub base_url: String,
    pub headers: Vec<Header>,
    pub models: Vec<ModelEntry>,
    pub api_key_set: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CliProvider {
    /// Equals `cli`: one provider per supported system CLI.
    pub instance_id: String,
    pub cli: String,
    pub label: String,
    pub enabled: bool,
    /// Model preferences; CLI-reported models missing here stay enabled.
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Defaults {
    pub instance_id: Option<String>,
    pub model: Option<String>,
    /// Reasoning effort for the default model.
    #[serde(default)]
    pub effort: Option<String>,
    /// Used when the default provider is unavailable.
    #[serde(default)]
    pub fallback: Option<Fallback>,
    #[serde(default)]
    pub permissions: Permissions,
    #[serde(default)]
    pub workspace: Workspace,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Fallback {
    pub instance_id: String,
    pub model: String,
    /// Reasoning effort for the fallback model.
    #[serde(default)]
    pub effort: Option<String>,
}

/// What CLI agents may do in a thread; fixed when the thread is created.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Permissions {
    #[default]
    ReadOnly,
    Edit,
    Full,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Workspace {
    #[default]
    Project,
    Private,
}

/// Reasoning effort names become CLI argv values.
pub(crate) fn valid_effort(effort: &str) -> Result<(), String> {
    if (1..=16).contains(&effort.len()) && effort.bytes().all(|b| b.is_ascii_lowercase()) {
        Ok(())
    } else {
        Err("Invalid reasoning effort.".into())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Behavior {
    pub send_key: SendKey,
    pub system_instructions: String,
    pub max_output_tokens: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ChatSettings {
    pub version: u32,
    pub revision: u64,
    pub api_providers: Vec<ApiProvider>,
    pub cli_providers: Vec<CliProvider>,
    pub defaults: Defaults,
    pub behavior: Behavior,
    /// CLI ids already auto-added or dismissed; never re-added by detection.
    #[serde(default)]
    pub auto_detected: Vec<String>,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            version: 2,
            revision: 0,
            api_providers: vec![],
            cli_providers: vec![],
            defaults: Defaults::default(),
            behavior: Behavior {
                send_key: SendKey::Enter,
                system_instructions: String::new(),
                max_output_tokens: None,
            },
            auto_detected: vec![],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ProviderInput {
    Api {
        instance_id: String,
        kind: ApiKind,
        label: String,
        #[serde(default)]
        accent_color: Option<String>,
        base_url: String,
        headers: Vec<Header>,
        models: Vec<ModelEntry>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum ProviderPatch {
    Api {
        label: String,
        #[serde(default)]
        accent_color: Option<String>,
        enabled: bool,
        base_url: String,
        headers: Vec<Header>,
        models: Vec<ModelEntry>,
    },
    Cli {
        label: String,
        enabled: bool,
        /// `None` keeps the stored model preferences.
        #[serde(default)]
        models: Option<Vec<ModelEntry>>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderCreate {
    pub expected_revision: u64,
    pub provider: ProviderInput,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderUpdate {
    pub expected_revision: u64,
    pub instance_id: String,
    pub patch: ProviderPatch,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderRemove {
    pub expected_revision: u64,
    pub instance_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderKeySet {
    pub instance_id: String,
    pub api_key: Option<Zeroizing<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ProviderProbe {
    pub instance_id: Option<String>,
    pub kind: ApiKind,
    pub base_url: String,
    pub headers: Vec<Header>,
    pub api_key: Option<Zeroizing<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeResult {
    pub models: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PreferencesUpdate {
    pub expected_revision: u64,
    pub defaults: Defaults,
    pub behavior: Behavior,
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

pub(crate) fn valid_instance_id(id: &str) -> Result<(), String> {
    let bytes = id.as_bytes();
    let ok = !bytes.is_empty()
        && bytes.len() <= 48
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err("Provider ID must be 1-48 lowercase letters, digits, hyphens or underscores, starting with a letter or digit.".into())
    }
}

pub(crate) fn normalize_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    let count = label.chars().count();
    if count == 0 || count > 64 || has_control(label) {
        return Err("Provider name must be 1-64 characters without control characters.".into());
    }
    Ok(label.to_string())
}

pub(crate) fn valid_accent(color: &Option<String>) -> Result<(), String> {
    match color {
        None => Ok(()),
        Some(c)
            if c.len() == 7
                && c.starts_with('#')
                && c[1..]
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
        {
            Ok(())
        }
        _ => Err("Accent color must look like #7c5cff.".into()),
    }
}

pub(crate) fn normalize_base_url(raw: &str) -> Result<String, String> {
    let invalid = || {
        "Base URL must be an https URL, or http for localhost, without credentials, query or fragment.".to_string()
    };
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty()
        || trimmed.len() > 2048
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(invalid());
    }
    let url = reqwest::Url::parse(trimmed).map_err(|_| invalid())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    let host = url.host_str().unwrap_or("").trim_matches(['[', ']']);
    let ok = match url.scheme() {
        "https" => !host.is_empty(),
        "http" => {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        _ => false,
    };
    if ok {
        Ok(trimmed.to_string())
    } else {
        Err(invalid())
    }
}

const BLOCKED_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "cookie",
    "host",
    "content-length",
    "content-type",
    "transfer-encoding",
    "connection",
    "anthropic-version",
];

pub(crate) fn validate_headers(headers: &[Header]) -> Result<(), String> {
    if headers.len() > 16 {
        return Err("At most 16 custom headers are allowed.".into());
    }
    let mut seen = Vec::new();
    for h in headers {
        let name_ok = !h.name.is_empty()
            && h.name.len() <= 64
            && h.name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+.^_`|~-".contains(&b));
        if !name_ok {
            return Err(format!("Invalid header name \"{}\".", safe_echo(&h.name)));
        }
        let lower = h.name.to_ascii_lowercase();
        if BLOCKED_HEADERS.contains(&lower.as_str()) {
            return Err(format!("The {} header is managed by Lomi.", h.name));
        }
        if seen.contains(&lower) {
            return Err(format!("Duplicate header \"{}\".", h.name));
        }
        seen.push(lower);
        if h.value.len() > 1024 || has_control(&h.value) {
            return Err(format!(
                "Header \"{}\" has an invalid value (max 1024 bytes, no control characters).",
                h.name
            ));
        }
    }
    Ok(())
}

fn safe_echo(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).take(32).collect()
}

pub(crate) fn valid_model_id(id: &str) -> Result<(), String> {
    let count = id.chars().count();
    if count == 0 || count > 256 || has_control(id) {
        return Err("Model IDs must be 1-256 characters without control characters.".into());
    }
    Ok(())
}

/// Validates and drops duplicate model IDs (first wins).
pub(crate) fn normalize_models(models: &[ModelEntry]) -> Result<Vec<ModelEntry>, String> {
    let mut out: Vec<ModelEntry> = Vec::new();
    for m in models {
        valid_model_id(&m.id)?;
        if !out.iter().any(|o| o.id == m.id) {
            out.push(m.clone());
        }
    }
    if out.len() > 500 {
        return Err("At most 500 models are allowed per provider.".into());
    }
    Ok(out)
}

pub(crate) fn validate_behavior(b: &Behavior) -> Result<(), String> {
    if b.system_instructions.len() > 32 * 1024 {
        return Err("System instructions are limited to 32 KiB.".into());
    }
    if b.max_output_tokens.is_some_and(|n| n == 0 || n > 1_000_000) {
        return Err("Max output tokens must be between 1 and 1,000,000.".into());
    }
    Ok(())
}

pub(crate) fn normalize_api(p: &ApiProvider) -> Result<ApiProvider, String> {
    valid_instance_id(&p.instance_id)?;
    valid_accent(&p.accent_color)?;
    validate_headers(&p.headers)?;
    Ok(ApiProvider {
        label: normalize_label(&p.label)?,
        base_url: normalize_base_url(&p.base_url)?,
        models: normalize_models(&p.models)?,
        ..p.clone()
    })
}

pub(crate) fn normalize_cli(p: &CliProvider) -> Result<CliProvider, String> {
    if !crate::ai_chat::cli::supported(&p.cli) || p.instance_id != p.cli {
        return Err("Unsupported CLI provider.".into());
    }
    Ok(CliProvider {
        label: normalize_label(&p.label)?,
        models: normalize_models(&p.models)?,
        ..p.clone()
    })
}

impl ChatSettings {
    pub(crate) fn has_instance(&self, id: &str) -> bool {
        self.api_providers.iter().any(|p| p.instance_id == id)
            || self.cli_providers.iter().any(|p| p.instance_id == id)
    }

    /// Full-document validation; returns the normalized document.
    pub(crate) fn validated(&self) -> Result<ChatSettings, String> {
        if self.version != 2 {
            return Err("Unsupported chat settings version.".into());
        }
        if self.auto_detected.len() > 64
            || self
                .auto_detected
                .iter()
                .any(|id| valid_instance_id(id).is_err())
        {
            return Err("Invalid auto-detected CLI list.".into());
        }
        let mut out = self.clone();
        out.api_providers = self
            .api_providers
            .iter()
            .map(normalize_api)
            .collect::<Result<_, _>>()?;
        out.cli_providers = self
            .cli_providers
            .iter()
            .map(normalize_cli)
            .collect::<Result<_, _>>()?;
        let mut ids: Vec<&str> = out
            .api_providers
            .iter()
            .map(|p| p.instance_id.as_str())
            .chain(out.cli_providers.iter().map(|p| p.instance_id.as_str()))
            .collect();
        ids.sort_unstable();
        if ids.windows(2).any(|w| w[0] == w[1]) {
            return Err("Provider IDs must be unique.".into());
        }
        validate_behavior(&out.behavior)?;
        self.validate_defaults(&out.defaults)?;
        Ok(out)
    }

    pub(crate) fn validate_defaults(&self, d: &Defaults) -> Result<(), String> {
        d.effort.as_deref().map_or(Ok(()), valid_effort)?;
        if let Some(f) = &d.fallback {
            if !self.has_instance(&f.instance_id) {
                return Err("The fallback provider does not exist.".into());
            }
            valid_model_id(&f.model)?;
            f.effort.as_deref().map_or(Ok(()), valid_effort)?;
        }
        match (&d.instance_id, &d.model) {
            (None, None) => Ok(()),
            (None, Some(_)) => Err("A default model needs a default provider.".into()),
            (Some(id), model) => {
                if !self.has_instance(id) {
                    return Err("The default provider does not exist.".into());
                }
                model.as_deref().map_or(Ok(()), valid_model_id)
            }
        }
    }

    /// Applies `f` to a copy, validates, bumps the revision and persists atomically.
    pub(crate) fn mutate(
        &mut self,
        dir: &Path,
        expected_revision: Option<u64>,
        f: impl FnOnce(&mut ChatSettings) -> Result<(), String>,
    ) -> Result<(), String> {
        if expected_revision.is_some_and(|r| r != self.revision) {
            return Err("Chat settings changed in another view. Reload and try again.".into());
        }
        let mut next = self.clone();
        f(&mut next)?;
        let mut next = next.validated()?;
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or("Chat settings revision overflow.")?;
        let bytes = serde_json::to_vec_pretty(&next).map_err(|_| "Cannot encode chat settings.")?;
        if bytes.len() as u64 > SETTINGS_LIMIT {
            return Err("Chat settings are too large.".into());
        }
        crate::storage_guard::atomic(&dir.join("settings.json"), &bytes)?;
        *self = next;
        Ok(())
    }
}

pub(crate) enum Loaded {
    Valid(ChatSettings),
    Invalid(String),
}

/// Parses a settings document. Version 1 predates system-CLI providers: its CLI
/// entries (account overlays) and the auto-detection memory are dropped so
/// installed CLIs are detected again; the file is rewritten as v2 on the next change.
fn parse(bytes: &[u8]) -> Result<ChatSettings, ()> {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| ())?;
    let v1 = value["version"] == 1;
    if v1 {
        value["version"] = 2.into();
        value["cliProviders"] = serde_json::json!([]);
        value["autoDetected"] = serde_json::json!([]);
    }
    let mut s: ChatSettings = serde_json::from_value(value).map_err(|_| ())?;
    if v1
        && s.defaults
            .instance_id
            .as_deref()
            .is_some_and(|id| !s.has_instance(id))
    {
        s.defaults = Defaults::default();
    }
    s.validated().map_err(|_| ())
}

/// Reads settings.json. A missing file yields defaults; anything unreadable or
/// invalid is preserved untouched and reported.
pub(crate) fn load(dir: &Path) -> Loaded {
    let path = dir.join("settings.json");
    let invalid = |path: &PathBuf| {
        Loaded::Invalid(format!(
            "Chat settings are invalid and were preserved at {}.",
            path.display()
        ))
    };
    if crate::storage_guard::reject_link(&path).is_err() {
        return invalid(&path);
    }
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded::Valid(ChatSettings::default())
        }
        Err(_) => return invalid(&path),
    };
    let mut bytes = Vec::new();
    if file
        .take(SETTINGS_LIMIT + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > SETTINGS_LIMIT
    {
        return invalid(&path);
    }
    match parse(&bytes) {
        Ok(s) => Loaded::Valid(s),
        Err(()) => invalid(&path),
    }
}

pub(crate) fn valid_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > KEY_LIMIT || has_control(key) {
        return Err("The API key is empty, too long or contains control characters.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(id: &str) -> ApiProvider {
        ApiProvider {
            instance_id: id.into(),
            kind: ApiKind::Openai,
            label: " OpenAI ".into(),
            accent_color: Some("#7c5cff".into()),
            enabled: true,
            base_url: "https://api.openai.com/v1/".into(),
            headers: vec![],
            models: vec![],
            api_key_set: false,
        }
    }

    #[test]
    fn base_url_rules() {
        assert_eq!(
            normalize_base_url(" https://api.openai.com/v1/ ").unwrap(),
            "https://api.openai.com/v1"
        );
        for ok in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8080",
            "http://127.9.9.9/v1",
            "http://[::1]:1234/v1",
            "https://example.com",
        ] {
            assert!(normalize_base_url(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://example.com/v1",
            "http://192.168.1.2",
            "https://user:pw@example.com",
            "https://user@example.com",
            "https://example.com/v1?x=1",
            "https://example.com/v1?",
            "https://example.com/v1#frag",
            "ftp://example.com",
            "",
            "https://exa mple.com",
            "not a url",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad}");
        }
        let long = format!("https://example.com/{}", "a".repeat(2048));
        assert!(normalize_base_url(&long).is_err());
    }

    #[test]
    fn header_rules() {
        let h = |n: &str, v: &str| Header {
            name: n.into(),
            value: v.into(),
        };
        assert!(validate_headers(&[h("OpenAI-Organization", "org_x")]).is_ok());
        for blocked in [
            "Authorization",
            "PROXY-AUTHORIZATION",
            "X-Api-Key",
            "api-key",
            "Cookie",
            "Host",
            "Content-Length",
            "Content-Type",
            "Transfer-Encoding",
            "Connection",
            "Anthropic-Version",
        ] {
            assert!(validate_headers(&[h(blocked, "x")]).is_err(), "{blocked}");
        }
        assert!(validate_headers(&[h("X-A", "a\r\nB: c")]).is_err());
        assert!(validate_headers(&[h("X-A", "a\nb")]).is_err());
        assert!(validate_headers(&[h("X A", "a")]).is_err());
        assert!(validate_headers(&[h("", "a")]).is_err());
        assert!(validate_headers(&[h("X-A", &"v".repeat(1025))]).is_err());
        assert!(validate_headers(&[h("X-A", "1"), h("x-a", "2")]).is_err());
        let many: Vec<Header> = (0..17).map(|i| h(&format!("X-{i}"), "v")).collect();
        assert!(validate_headers(&many).is_err());
    }

    #[test]
    fn identity_and_labels() {
        for ok in ["a", "openai-work", "0x_y", &"a".repeat(48)] {
            assert!(valid_instance_id(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-a", "_a", "A", "a b", "a.b", &"a".repeat(49)] {
            assert!(valid_instance_id(bad).is_err(), "{bad}");
        }
        assert!(valid_accent(&Some("#7c5cff".into())).is_ok());
        assert!(valid_accent(&None).is_ok());
        for bad in ["#7C5CFF", "7c5cff", "#7c5cf", "#7c5cfff", "#7c5cfg"] {
            assert!(valid_accent(&Some(bad.into())).is_err(), "{bad}");
        }
        assert_eq!(normalize_label("  x ").unwrap(), "x");
        assert!(normalize_label("   ").is_err());
        assert!(normalize_label(&"x".repeat(65)).is_err());
        assert!(normalize_label("a\nb").is_err());
        let normalized = normalize_api(&api("openai")).unwrap();
        assert_eq!(normalized.label, "OpenAI");
        assert_eq!(normalized.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn models_and_behavior() {
        let m = |id: &str| ModelEntry {
            id: id.into(),
            source: ModelSource::Manual,
            enabled: true,
            favorite: false,
        };
        let old: ModelEntry = serde_json::from_str(r#"{"id":"x","source":"fetched"}"#).unwrap();
        assert!(old.enabled && !old.favorite);
        assert_eq!(
            normalize_models(&[m("a"), m("a"), m("b")]).unwrap().len(),
            2
        );
        assert!(normalize_models(&[m("")]).is_err());
        assert!(normalize_models(&[m("a\0")]).is_err());
        let many: Vec<ModelEntry> = (0..501).map(|i| m(&format!("m{i}"))).collect();
        assert!(normalize_models(&many).is_err());
        let b = |t: Option<u32>| Behavior {
            send_key: SendKey::Enter,
            system_instructions: String::new(),
            max_output_tokens: t,
        };
        assert!(validate_behavior(&b(Some(0))).is_err());
        assert!(validate_behavior(&b(Some(1_000_001))).is_err());
        assert!(validate_behavior(&b(Some(1_000_000))).is_ok());
    }

    #[test]
    fn serde_shapes() {
        let input: ProviderCreate = serde_json::from_str(
            r##"{"expectedRevision":0,"provider":{"type":"api","instanceId":"a","kind":"openai-compatible","label":"L","accentColor":null,"baseUrl":"http://localhost:1/v1","headers":[],"models":[{"id":"m","source":"fetched"}]}}"##,
        )
        .unwrap();
        assert!(matches!(
            input.provider,
            ProviderInput::Api {
                kind: ApiKind::OpenaiCompatible,
                ..
            }
        ));
        assert!(serde_json::from_str::<ProviderCreate>(
            r##"{"expectedRevision":1,"provider":{"type":"cli","instanceId":"codex"}}"##
        )
        .is_err());
        let patch: ProviderUpdate = serde_json::from_str(
            r##"{"expectedRevision":1,"instanceId":"a","patch":{"type":"cli","label":"L","enabled":false}}"##,
        )
        .unwrap();
        assert!(matches!(
            patch.patch,
            ProviderPatch::Cli {
                enabled: false,
                models: None,
                ..
            }
        ));
        assert!(serde_json::from_str::<ProviderUpdate>(
            r##"{"expectedRevision":1,"instanceId":"a","patch":{"type":"cli","label":"L","enabled":true,"x":1}}"##
        )
        .is_err());
        let json = serde_json::to_value(ChatSettings::default()).unwrap();
        assert_eq!(json["behavior"]["sendKey"], "enter");
        assert_eq!(json["apiProviders"], serde_json::json!([]));
    }

    fn cli(id: &str) -> CliProvider {
        CliProvider {
            instance_id: id.into(),
            cli: id.into(),
            label: " Codex ".into(),
            enabled: true,
            models: vec![],
        }
    }

    #[test]
    fn defaults_serde_and_validation() {
        let old: Defaults = serde_json::from_str(r#"{"instanceId":null,"model":null}"#).unwrap();
        assert_eq!(old, Defaults::default());
        let new: Defaults = serde_json::from_str(
            r#"{"instanceId":"codex","model":"m","effort":"high","fallback":{"instanceId":"claude","model":"opus"},
                "permissions":"full","workspace":"private"}"#,
        )
        .unwrap();
        assert_eq!(
            (new.permissions, new.workspace),
            (Permissions::Full, Workspace::Private)
        );
        assert!(serde_json::from_str::<Defaults>(
            r#"{"instanceId":null,"model":null,"permissions":"x"}"#
        )
        .is_err());
        let mut s = ChatSettings::default();
        s.cli_providers.push(cli("codex"));
        s.cli_providers.push(cli("claude"));
        assert!(s.validate_defaults(&new).is_ok());
        let mut bad = new.clone();
        bad.effort = Some("High".into());
        assert!(s.validate_defaults(&bad).is_err());
        bad.effort = None;
        bad.fallback = Some(Fallback {
            instance_id: "nope".into(),
            model: "m".into(),
            effort: None,
        });
        assert!(s.validate_defaults(&bad).is_err());
        // Old files have no fallback effort; a present one is validated.
        assert_eq!(new.fallback.as_ref().unwrap().effort, None);
        let with: Defaults = serde_json::from_str(
            r#"{"instanceId":null,"model":null,"fallback":{"instanceId":"claude","model":"opus","effort":"high"}}"#,
        )
        .unwrap();
        assert_eq!(
            with.fallback.as_ref().unwrap().effort.as_deref(),
            Some("high")
        );
        assert!(s.validate_defaults(&with).is_ok());
        let mut bad = with.clone();
        bad.fallback.as_mut().unwrap().effort = Some("High".into());
        assert!(s.validate_defaults(&bad).is_err());
        assert!(valid_effort("xhigh").is_ok());
        assert!(["", "-x", "a b", "abcdefghijklmnopq"]
            .iter()
            .all(|e| valid_effort(e).is_err()));
    }

    #[test]
    fn cli_providers_are_validated() {
        let mut s = ChatSettings::default();
        s.cli_providers.push(cli("codex"));
        assert_eq!(s.validated().unwrap().cli_providers[0].label, "Codex");
        s.cli_providers[0].instance_id = "claude".into();
        assert!(s.validated().is_err());
        s.cli_providers = vec![cli("pi")];
        assert!(s.validated().is_err());
        s.cli_providers = vec![cli("codex"), cli("codex")];
        assert!(s.validated().is_err());
    }

    #[test]
    fn v1_file_loads_as_v2_keeping_api_providers() {
        let dir = tempfile::tempdir().unwrap();
        let mut api = serde_json::to_value(api("openai")).unwrap();
        api["label"] = "OpenAI".into();
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "revision": 3, "apiProviders": [api],
                "cliProviders": [{"instanceId": "cli-341e041adc14c3266723c71e7d836e2d",
                    "accountId": "341e041adc14c3266723c71e7d836e2d", "accentColor": null}],
                "defaults": {"instanceId": "cli-341e041adc14c3266723c71e7d836e2d", "model": null},
                "behavior": {"sendKey": "mod-enter", "systemInstructions": "be brief", "maxOutputTokens": null},
                "autoDetected": ["codex", "claude"]
            }))
            .unwrap(),
        )
        .unwrap();
        let Loaded::Valid(s) = load(dir.path()) else {
            panic!("expected valid");
        };
        assert_eq!((s.version, s.revision), (2, 3));
        assert_eq!(s.api_providers.len(), 1);
        assert!(s.cli_providers.is_empty() && s.auto_detected.is_empty());
        assert_eq!(s.defaults.instance_id, None);
        assert_eq!(
            (s.defaults.permissions, s.defaults.workspace),
            (Permissions::ReadOnly, Workspace::Project)
        );
        assert_eq!(s.behavior.system_instructions, "be brief");
        // The next change writes v2.
        let mut s = s;
        s.mutate(dir.path(), None, |_| Ok(())).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(saved["version"], 2);
    }

    #[test]
    fn revision_conflict_and_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = ChatSettings::default();
        s.mutate(dir.path(), Some(0), |s| {
            s.api_providers.push(api("openai"));
            Ok(())
        })
        .unwrap();
        assert_eq!(s.revision, 1);
        let err = s.mutate(dir.path(), Some(0), |_| Ok(())).unwrap_err();
        assert!(err.contains("changed in another view"));
        // A failing mutation leaves memory and disk untouched.
        assert!(s
            .mutate(dir.path(), Some(1), |s| {
                s.api_providers.push(api("openai"));
                Ok(())
            })
            .is_err());
        assert_eq!(s.revision, 1);
        match load(dir.path()) {
            Loaded::Valid(l) => assert_eq!(l, s),
            Loaded::Invalid(_) => panic!("expected valid"),
        }
    }

    #[test]
    fn invalid_file_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        for content in [
            r#"{"version":2,"revision":0}"#.to_string(),
            "not json".to_string(),
            serde_json::to_string(&serde_json::json!({
                "version":1,"revision":0,"apiProviders":[],"cliProviders":[],
                "defaults":{"instanceId":null,"model":null},
                "behavior":{"sendKey":"enter","systemInstructions":"","maxOutputTokens":0}
            }))
            .unwrap(),
        ] {
            std::fs::write(&path, &content).unwrap();
            match load(dir.path()) {
                Loaded::Invalid(m) => assert!(m.contains("preserved at")),
                Loaded::Valid(_) => panic!("expected invalid: {content}"),
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(load(dir.path()), Loaded::Valid(_)));
    }
}
