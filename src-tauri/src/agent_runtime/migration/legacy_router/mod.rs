use std::collections::HashSet;
// Frozen schema 1–5 data decoder. No executable legacy dependencies.
type TitleCli = String;
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Protocol {
    Anthropic,
    OpenaiChat,
    OpenaiResponses,
    Gemini,
}

use serde::{Deserialize, Serialize};

macro_rules! state {
    ($name:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
        #[serde(rename_all = "snake_case")]
        pub(crate) enum $name { $($variant),+ }
    };
}

state!(AuthState {
    Disconnected,
    Connecting,
    Verifying,
    Unverified,
    Ready,
    Refreshing,
    ReauthRequired,
    Disabled,
    PendingRemove,
    IdentityMismatch,
    Error
});
state!(StorageMode {
    Keyring,
    SessionOnly,
    CliManaged,
    ApiKey
});
state!(QuotaStatus {
    Unknown,
    Fresh,
    Stale,
    Exhausted,
    ReaderThrottled,
    ReaderError,
    Unsupported
});
state!(RunState {
    Idle,
    Starting,
    Running,
    Switching,
    WaitingForCapacity,
    Paused,
    Completed,
    Stopped,
    Failed,
    RecoveryRequired
});
state!(AttemptState {
    Selected,
    DispatchIntent,
    Running,
    Completed,
    Rejected,
    Failed,
    Stopped,
    RecoveryRequired
});

impl AttemptState {
    pub(crate) fn is_active(self) -> bool {
        matches!(self, Self::Selected | Self::DispatchIntent | Self::Running)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Profile {
    pub(crate) id: String,
    pub(crate) cli: TitleCli,
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) revision: u64,
    pub(crate) auth_state: AuthState,
    pub(crate) storage_mode: StorageMode,
    #[serde(default)]
    pub(crate) credential_ref: Option<String>,
    #[serde(default)]
    pub(crate) quota_group_key: Option<String>,
    /// Explicit native API destination used only by gateway terminals. Existing
    /// subscription and managed-text bindings keep their original contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gateway_provider: Option<ApiDestination>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ApiDestination {
    pub(crate) protocol: Protocol,
    pub(crate) base_url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Router {
    pub(crate) id: String,
    pub(crate) cli: TitleCli,
    pub(crate) label: String,
    pub(crate) enabled: bool,
    pub(crate) ordered_profile_ids: Vec<String>,
    pub(crate) balance_remaining_quota: bool,
    pub(crate) revision: u64,
}

/// Windows must already be qualified by the native reader as blocking the run's
/// model and execution mode. Unknown or incomplete scope is not a fresh report.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct QuotaWindow {
    pub(crate) id: String,
    pub(crate) remaining_percent: Option<f64>,
    pub(crate) reset_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Quota {
    pub(crate) profile_id: String,
    pub(crate) status: QuotaStatus,
    pub(crate) windows: Vec<QuotaWindow>,
    pub(crate) observed_at: i64,
    pub(crate) expires_at: i64,
    pub(crate) epoch: u64,
    pub(crate) block_revision: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunInput {
    pub(crate) id: String,
    pub(crate) text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunAttempt {
    pub(crate) id: String,
    pub(crate) input_id: String,
    pub(crate) profile_id: String,
    pub(crate) generation: u64,
    pub(crate) state: AttemptState,
    pub(crate) reason: String,
}

/// Text observed from one exact native attempt. Completed is authoritative only
/// after the provider terminal event, clean EOF and successful process drain.
/// Other terminal states preserve uncertain partial output without replay.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct RunTurn {
    pub(crate) input_id: String,
    pub(crate) attempt_id: String,
    pub(crate) profile_id: String,
    pub(crate) generation: u64,
    pub(crate) state: AttemptState,
    pub(crate) text: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RunExecutionMode {
    #[default]
    Text,
    Coding,
    Gateway,
    Native,
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Run {
    pub(crate) id: String,
    pub(crate) router_id: String,
    pub(crate) cwd: String,
    #[serde(default)]
    pub(crate) shell_profile_id: Option<String>,
    pub(crate) title: String,
    pub(crate) state: RunState,
    pub(crate) model: Option<String>,
    #[serde(default)]
    pub(crate) reasoning_effort: Option<String>,
    #[serde(default)]
    pub(crate) execution_mode: RunExecutionMode,
    /// Main explicitly requested continuation of a fully checkpointed native
    /// interrupted turn. Consumed by dispatch; never inferred from status text.
    #[serde(default, skip_serializing_if = "is_false")]
    pub(crate) continuation_requested: bool,
    pub(crate) pinned_profile_id: Option<String>,
    pub(crate) allowed_profile_ids: Vec<String>,
    pub(crate) active_profile_id: Option<String>,
    pub(crate) generation: u64,
    pub(crate) revision: u64,
    pub(crate) inputs: Vec<RunInput>,
    pub(crate) attempts: Vec<RunAttempt>,
    pub(crate) output: String,
    #[serde(default)]
    pub(crate) turns: Vec<RunTurn>,
    /// Pre-ledger output cannot be associated with an input or completion.
    /// Preserve it verbatim once, separate from newly recorded native turns.
    #[serde(default)]
    pub(crate) legacy_output: Option<String>,
    pub(crate) status_message: String,
    /// Recovery ledger for the current logical input, cleared only after a
    /// completed turn or an authoritative capacity/auth repair.
    pub(crate) attempted_profile_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Snapshot {
    pub(crate) revision: u64,
    pub(crate) profiles: Vec<Profile>,
    pub(crate) routers: Vec<Router>,
    pub(crate) quota: Vec<Quota>,
    pub(crate) runs: Vec<Run>,
}

fn unique<'a>(ids: impl IntoIterator<Item = &'a str>) -> bool {
    let mut seen = HashSet::new();
    ids.into_iter()
        .all(|id| !id.is_empty() && id.len() <= 256 && seen.insert(id))
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if !unique(snapshot.profiles.iter().map(|p| p.id.as_str()))
        || !unique(snapshot.routers.iter().map(|r| r.id.as_str()))
        || !unique(snapshot.runs.iter().map(|r| r.id.as_str()))
        || !unique(snapshot.quota.iter().map(|q| q.profile_id.as_str()))
    {
        return Err("Invalid or duplicate router record identifier".into());
    }
    for router in &snapshot.routers {
        if !unique(router.ordered_profile_ids.iter().map(String::as_str))
            || router.ordered_profile_ids.iter().any(|id| {
                !snapshot
                    .profiles
                    .iter()
                    .any(|p| p.id == *id && p.cli == router.cli)
            })
        {
            return Err("Router profiles must exist and use the same CLI".into());
        }
    }
    for quota in &snapshot.quota {
        if !snapshot.profiles.iter().any(|p| p.id == quota.profile_id)
            || !unique(quota.windows.iter().map(|w| w.id.as_str()))
            || quota.windows.iter().any(|w| {
                w.remaining_percent
                    .is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v))
            })
        {
            return Err("Invalid sanitized quota observation".into());
        }
    }
    for run in &snapshot.runs {
        // History survives removal of its router/profile records.
        if !unique(run.inputs.iter().map(|input| input.id.as_str()))
            || !unique(run.attempts.iter().map(|attempt| attempt.id.as_str()))
            || !unique(run.allowed_profile_ids.iter().map(String::as_str))
            || !unique(run.attempted_profile_ids.iter().map(String::as_str))
            || run.attempts.iter().filter(|a| a.state.is_active()).count() > 1
            || run.attempts.iter().any(|attempt| {
                !run.inputs.iter().any(|input| input.id == attempt.input_id)
                    || attempt.generation > run.generation
                    || (attempt.state.is_active()
                        && !run.allowed_profile_ids.contains(&attempt.profile_id))
            })
            || !unique(run.turns.iter().map(|turn| turn.attempt_id.as_str()))
            || run.turns.iter().any(|turn| {
                !run.attempts.iter().any(|attempt| {
                    attempt.id == turn.attempt_id
                        && attempt.input_id == turn.input_id
                        && attempt.profile_id == turn.profile_id
                        && attempt.generation == turn.generation
                        && attempt.state == turn.state
                }) || turn.text.len() > (1024 * 1024)
            })
            || run
                .legacy_output
                .as_ref()
                .is_some_and(|text| text.len() > (1024 * 1024))
        {
            return Err("Invalid logical input or run attempt ledger".into());
        }
        if run.output.len() > (1024 * 1024)
            || run
                .turns
                .windows(2)
                .any(|turns| turns[0].generation >= turns[1].generation)
        {
            return Err("Invalid managed-turn history size or order".into());
        }
        if !run.turns.is_empty() || run.legacy_output.is_some() {
            let mut output = run.legacy_output.clone().unwrap_or_default();
            for turn in &run.turns {
                if output.len().saturating_add(turn.text.len()) > (1024 * 1024) {
                    return Err("Managed-turn history exceeds its size limit".into());
                }
                output.push_str(&turn.text);
            }
            if output != run.output {
                return Err("Managed-turn records do not match retained output".into());
            }
        }
    }
    Ok(())
}

pub(super) fn decode(value: &serde_json::Value) -> Result<(), String> {
    let snapshot: Snapshot =
        serde_json::from_value(value.clone()).map_err(|_| "Invalid legacy snapshot")?;
    for cli in snapshot
        .profiles
        .iter()
        .map(|p| p.cli.as_str())
        .chain(snapshot.routers.iter().map(|r| r.cli.as_str()))
    {
        if !matches!(
            cli,
            "codex"
                | "agy"
                | "cursor"
                | "claude"
                | "gemini"
                | "copilot"
                | "opencode"
                | "openclaw"
                | "hermes"
                | "pi"
                | "kilo"
                | "qwen"
                | "kiro"
                | "vibe"
                | "kimi"
                | "grok"
                | "aider"
                | "cline"
                | "goose"
                | "droid"
                | "openhands"
                | "continue"
                | "amp"
                | "auggie"
                | "crush"
                | "interpreter"
                | "junie"
                | "freebuff"
                | "sweagent"
                | "deepagents"
                | "trae"
        ) {
            return Err("Unknown legacy CLI identifier".into());
        }
    }
    validate(&snapshot)
}
