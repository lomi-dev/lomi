//! Pure, version-qualified native control-plane codecs. This module neither
//! launches children nor reads credentials. The owner must fence the executable,
//! workspace, profile, session and generation before sending any request.
//! Local server authentication below is control-plane authentication, NEVER
//! provider OAuth. Protocol completion still requires the owner's clean drain
//! and effect/history checks. A successful exit alone is not effect evidence.
use crate::cli_catalog::TitleCli;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

pub(crate) const MAX_FRAME: usize = 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_DEPTH: usize = 64;
const MAX_LEDGER_BYTES: usize = 16 * MAX_FRAME;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeKind {
    Claude,
    Codex,
    Grok,
    Kimi,
    Kilo,
    OpenCode,
    Pi,
    Agy,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeTransport {
    Stdio,
    HttpSse,
    HttpWebsocket,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ApprovalSurface {
    NativeControl,
    ExtensionDialogs,
    NativePtyOnly,
}
#[derive(Clone, Debug)]
pub(crate) struct NativeLaunch {
    pub(crate) arguments: Vec<String>,
    pub(crate) transport: NativeTransport,
    pub(crate) approvals: ApprovalSurface,
    /// Caller supplies a random secret in the named private environment variable.
    pub(crate) local_password_variable: Option<&'static str>,
    pub(crate) local_username: Option<&'static str>,
    /// Kimi creates this private local server token; never copy provider tokens.
    pub(crate) native_server_token: bool,
}
impl NativeKind {
    pub(crate) fn from_version(cli: TitleCli, output: &str) -> Result<Self, String> {
        let kind = Self::from_cli(cli).ok_or("No qualified native wire adapter.")?;
        if output.len() > 4096
            || !output.split_whitespace().any(|word| {
                let version = word
                    .trim_matches(|c: char| matches!(c, '(' | ')' | ',' | ';'))
                    .trim_start_matches('v');
                kind.versions().contains(&version)
            })
        {
            return Err("Native CLI version does not match the admitted wire contract.".into());
        }
        Ok(kind)
    }
    pub(crate) fn from_cli(cli: TitleCli) -> Option<Self> {
        Some(match cli {
            TitleCli::Claude => Self::Claude,
            TitleCli::Codex => Self::Codex,
            TitleCli::Grok => Self::Grok,
            TitleCli::Kimi => Self::Kimi,
            TitleCli::Kilo => Self::Kilo,
            TitleCli::Opencode => Self::OpenCode,
            TitleCli::Pi => Self::Pi,
            TitleCli::Agy => Self::Agy,
            _ => return None,
        })
    }
    pub(crate) fn versions(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["2.1.287"],
            Self::Codex => &["0.160.0"],
            Self::Grok => &["1.0.45"],
            Self::Kimi => &["2.1.1"],
            Self::Kilo => &["7.8.3"],
            Self::OpenCode => &["1.18.33", "1.18.34"],
            Self::Pi => &["1.0.1"],
            Self::Agy => &["1.2.16"],
        }
    }
    pub(crate) fn launch(self) -> NativeLaunch {
        let (args, transport, approvals, password, username, token) = match self {
            Self::Claude => (
                vec![
                    "--print",
                    "--verbose",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--include-partial-messages",
                    "--include-hook-events",
                    "--permission-prompt-tool",
                    "stdio",
                ],
                NativeTransport::Stdio,
                ApprovalSurface::NativeControl,
                None,
                None,
                false,
            ),
            Self::Codex => (
                vec!["app-server", "--listen", "stdio://"],
                NativeTransport::Stdio,
                ApprovalSurface::NativeControl,
                None,
                None,
                false,
            ),
            Self::Grok => (
                vec!["agent", "--no-leader", "stdio"],
                NativeTransport::Stdio,
                ApprovalSurface::NativeControl,
                None,
                None,
                false,
            ),
            Self::Kimi => (
                vec!["web", "--host", "127.0.0.1", "--port", "0", "--no-open"],
                NativeTransport::HttpWebsocket,
                ApprovalSurface::NativeControl,
                None,
                None,
                true,
            ),
            Self::Kilo => (
                vec![
                    "serve",
                    "--hostname",
                    "127.0.0.1",
                    "--port",
                    "0",
                    "--no-mdns",
                ],
                NativeTransport::HttpSse,
                ApprovalSurface::NativeControl,
                Some("KILO_SERVER_PASSWORD"),
                Some("kilo"),
                false,
            ),
            Self::OpenCode => (
                vec![
                    "serve",
                    "--hostname",
                    "127.0.0.1",
                    "--port",
                    "0",
                    "--no-mdns",
                ],
                NativeTransport::HttpSse,
                ApprovalSurface::NativeControl,
                Some("OPENCODE_SERVER_PASSWORD"),
                Some("opencode"),
                false,
            ),
            // Hooks fail open and are not an authorization broker. This descriptor
            // is observation-only; coding approval remains in the native PTY.
            Self::Agy => (
                vec![
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                ],
                NativeTransport::Stdio,
                ApprovalSurface::NativePtyOnly,
                None,
                None,
                false,
            ),
            Self::Pi => (
                vec!["--mode", "rpc"],
                NativeTransport::Stdio,
                ApprovalSurface::ExtensionDialogs,
                None,
                None,
                false,
            ),
        };
        NativeLaunch {
            arguments: args.into_iter().map(str::to_owned).collect(),
            transport,
            approvals,
            local_password_variable: password,
            local_username: username,
            native_server_token: token,
        }
    }
    pub(crate) fn launch_for_model(
        self,
        model: &str,
        provider: Option<&str>,
    ) -> Result<NativeLaunch, String> {
        model_name(model)?;
        let mut launch = self.launch();
        // Grok's model option belongs before its `stdio` subcommand. The
        // supervisor selects it through the acknowledged ACP model request.
        if matches!(self, Self::Claude | Self::Pi) {
            if self == Self::Pi {
                let provider = provider.ok_or("Pi requires an exact provider binding.")?;
                model_name(provider)?;
                launch
                    .arguments
                    .extend(["--provider".into(), provider.into()]);
            }
            launch.arguments.extend(["--model".into(), model.into()]);
        }
        Ok(launch)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum WireRequest {
    Stdio(Value),
    Http {
        method: &'static str,
        path: String,
        body: Option<Value>,
    },
    Websocket {
        path: String,
    },
}

fn rpc(id: &str, method: &str, params: Value) -> WireRequest {
    WireRequest::Stdio(json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
}
fn http(method: &'static str, path: String, body: Option<Value>) -> WireRequest {
    WireRequest::Http { method, path, body }
}
fn segment(id: &str) -> Result<&str, String> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Err("Invalid native identifier.".into())
    } else {
        Ok(id)
    }
}
fn model_name(model: &str) -> Result<(), String> {
    if model.is_empty() || model.len() > 512 || model.chars().any(char::is_control) {
        return Err("Invalid exact native model binding.".into());
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ErrorClass {
    Quota,
    Throttle,
    Authentication,
    Permission,
    Cancelled,
    Protocol,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ToolState {
    Started,
    Pending,
    Completed,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeTool {
    pub(crate) session_id: String,
    pub(crate) turn_id: String,
    pub(crate) tool_id: String,
    pub(crate) state: ToolState,
    pub(crate) raw: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativePermission {
    /// Native RPC/control/dialog ID, kept verbatim for an exact reply.
    pub(crate) request_id: Value,
    pub(crate) session_id: String,
    pub(crate) turn_id: String,
    pub(crate) tool_id: Option<String>,
    pub(crate) choices: Vec<Value>,
    pub(crate) raw: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NativeOutcome {
    Completed,
    Cancelled,
    Failed,
    Blocked,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeTerminal {
    pub(crate) session_id: String,
    pub(crate) turn_id: String,
    pub(crate) outcome: NativeOutcome,
    pub(crate) error_class: Option<ErrorClass>,
    pub(crate) raw: Value,
}
#[derive(Clone, Debug, Serialize)]
pub(crate) enum NativeEvent {
    Accepted {
        request_id: Value,
        raw: Value,
    },
    Session {
        session_id: String,
        raw: Value,
    },
    TurnStarted {
        session_id: String,
        turn_id: String,
        raw: Value,
    },
    Text {
        session_id: String,
        turn_id: String,
        text: String,
    },
    Tool(NativeTool),
    Permission(NativePermission),
    Terminal(NativeTerminal),
    PermissionWithdrawn {
        request_id: Value,
    },
    Error {
        class: ErrorClass,
        raw: Value,
    },
    /// Structured account/billing and unknown events remain available to callers.
    Observed(Value),
}

/// One outstanding logical prompt per wire. Pi/Claude events lacking turn IDs
/// are correlated only through this owned single-prompt binding. Reusing a wire
/// for a new turn requires a new instance after terminal/drain verification.
pub(crate) struct NativeWire {
    kind: NativeKind,
    session: String,
    host_turn: String,
    native_turn: Option<String>,
    prompt_request: Option<String>,
    requests: BTreeMap<String, String>,
    permissions: BTreeMap<String, NativePermission>,
    tools: BTreeMap<String, NativeTool>,
    terminal: Option<NativeTerminal>,
    assistant_complete: bool,
    idle: bool,
    started: bool,
    failure: Option<ErrorClass>,
    model: Option<String>,
    provider: Option<String>,
    reasoning: Option<String>,
    assistant_ids: BTreeSet<String>,
    native_agent: Option<String>,
    ledger_bytes: usize,
    ws_hello_id: Option<String>,
    ws_subscribe_id: Option<String>,
    subscription_acknowledged: bool,
    journal_seq: Option<u64>,
    journal_epoch: Option<String>,
    grok_cached_token_advertised: bool,
    grok_authenticated: bool,
}
impl NativeWire {
    pub(crate) fn new(
        kind: NativeKind,
        owned_session: &str,
        host_turn: &str,
    ) -> Result<Self, String> {
        segment(host_turn)?;
        if !owned_session.is_empty() {
            segment(owned_session)?;
        }
        Ok(Self {
            kind,
            session: owned_session.into(),
            host_turn: host_turn.into(),
            native_turn: None,
            prompt_request: None,
            requests: BTreeMap::new(),
            permissions: BTreeMap::new(),
            tools: BTreeMap::new(),
            terminal: None,
            assistant_complete: false,
            idle: false,
            started: false,
            failure: None,
            model: None,
            provider: None,
            reasoning: None,
            assistant_ids: BTreeSet::new(),
            native_agent: None,
            ledger_bytes: 0,
            ws_hello_id: None,
            ws_subscribe_id: None,
            subscription_acknowledged: false,
            journal_seq: None,
            journal_epoch: None,
            grok_cached_token_advertised: false,
            grok_authenticated: false,
        })
    }
    pub(crate) fn kind(&self) -> NativeKind {
        self.kind
    }
    pub(crate) fn session_id(&self) -> &str {
        &self.session
    }
    pub(crate) fn turn_id(&self) -> &str {
        self.native_turn.as_deref().unwrap_or(&self.host_turn)
    }
    pub(crate) fn tools(&self) -> Vec<NativeTool> {
        self.tools.values().cloned().collect()
    }
    pub(crate) fn terminal(&self) -> Option<&NativeTerminal> {
        self.terminal.as_ref()
    }
    pub(crate) fn permission_pending(&self, request_id: &Value) -> bool {
        self.permissions.contains_key(&request_id.to_string())
    }
    pub(crate) fn pending_requests(&self) -> Vec<String> {
        self.requests.keys().cloned().collect()
    }
    pub(crate) fn ready_for_prompt(&self) -> bool {
        self.model.is_some()
            && self.failure.is_none()
            && self.prompt_request.is_none()
            && self.subscription_ready()
            && (self.kind != NativeKind::Grok || self.grok_authenticated)
            && !self.requests.values().any(|operation| {
                matches!(
                    operation.as_str(),
                    "initialize" | "authenticate" | "session" | "resume" | "model" | "thinking"
                )
            })
    }
    pub(crate) fn subscription_ready(&self) -> bool {
        self.kind != NativeKind::Kimi
            || (self.subscription_acknowledged
                && self.failure.is_none()
                && self.journal_seq.is_some()
                && self.journal_epoch.is_some())
    }
    fn remember(&mut self, id: &str, method: &str) -> Result<(), String> {
        segment(id)?;
        if self.requests.len() >= MAX_ITEMS || self.requests.contains_key(id) {
            return Err("Native request ID reused or bound exceeded.".into());
        }
        self.requests.insert(id.into(), method.into());
        Ok(())
    }
    pub(crate) fn initialize(&mut self, id: &str) -> Result<Vec<WireRequest>, String> {
        self.remember(id, "initialize")?;
        Ok(match self.kind {
            NativeKind::Claude => vec![WireRequest::Stdio(
                json!({"type":"control_request","request_id":id,"request":{"subtype":"initialize"}}),
            )],
            NativeKind::Codex => vec![rpc(
                id,
                "initialize",
                json!({"clientInfo":{"name":"lomi","title":"Lomi","version":"1"},"capabilities":{"experimentalApi":true}}),
            )],
            NativeKind::Grok => vec![rpc(
                id,
                "initialize",
                json!({"protocolVersion":1,"clientCapabilities":{},"clientInfo":{"name":"lomi","version":"1"}}),
            )],
            NativeKind::Pi => vec![WireRequest::Stdio(json!({"id":id,"type":"get_state"}))],
            NativeKind::Kilo | NativeKind::OpenCode => {
                vec![http("GET", "/global/health".into(), None)]
            }
            NativeKind::Kimi => vec![http("GET", "/api/v1/sessions".into(), None)],
            NativeKind::Agy => return Err(
                "Agy headless control initialization is unqualified; retain native PTY approvals."
                    .into(),
            ),
        })
    }
    pub(crate) fn initialized(&self) -> Option<WireRequest> {
        (self.kind == NativeKind::Codex)
            .then(|| WireRequest::Stdio(json!({"method":"initialized"})))
    }
    /// ACP native cached-token authentication. The credentials stay in Grok's
    /// owned native profile; no provider token/key is carried over this wire.
    /// Only the correlated initialize response may advertise this method.
    pub(crate) fn authentication_request(
        &mut self,
        id: &str,
    ) -> Result<Option<WireRequest>, String> {
        if self.kind != NativeKind::Grok {
            return Ok(None);
        }
        if self.grok_authenticated {
            return Ok(None);
        }
        if !self.grok_cached_token_advertised
            || self.failure.is_some()
            || self
                .requests
                .values()
                .any(|operation| matches!(operation.as_str(), "initialize" | "authenticate"))
        {
            self.failure = Some(ErrorClass::Authentication);
            return Err(
                "Grok has no acknowledged cached-token authentication method available.".into(),
            );
        }
        self.remember(id, "authenticate")?;
        Ok(Some(rpc(
            id,
            "authenticate",
            json!({"methodId":"cached_token","_meta":{"headless":true}}),
        )))
    }
    pub(crate) fn session_request(
        &mut self,
        id: &str,
        cwd: &str,
        resume: bool,
    ) -> Result<WireRequest, String> {
        if self.kind == NativeKind::Grok && !self.grok_authenticated {
            return Err(
                "Grok native authentication must succeed before creating or resuming a session."
                    .into(),
            );
        }
        if cwd.is_empty() || cwd.len() > 16 * 1024 {
            return Err("Invalid owned native workspace.".into());
        }
        if resume && self.session.is_empty() {
            return Err("Resume requires an owned native session.".into());
        }
        self.remember(id, if resume { "resume" } else { "session" })?;
        Ok(match self.kind {
            NativeKind::Codex => {
                if resume {
                    rpc(
                        id,
                        "thread/resume",
                        json!({"threadId":self.session,"cwd":cwd}),
                    )
                } else {
                    rpc(
                        id,
                        "thread/start",
                        json!({"cwd":cwd,"approvalPolicy":"untrusted"}),
                    )
                }
            }
            NativeKind::Grok => {
                if resume {
                    rpc(
                        id,
                        "session/load",
                        json!({"sessionId":self.session,"cwd":cwd,"mcpServers":[]}),
                    )
                } else {
                    rpc(id, "session/new", json!({"cwd":cwd,"mcpServers":[]}))
                }
            }
            NativeKind::Kilo | NativeKind::OpenCode => {
                if resume {
                    http("GET", format!("/session/{}", segment(&self.session)?), None)
                } else {
                    http("POST", "/session".into(), Some(json!({})))
                }
            }
            NativeKind::Kimi => {
                if resume {
                    http(
                        "GET",
                        format!("/api/v1/sessions/{}", segment(&self.session)?),
                        None,
                    )
                } else {
                    http(
                        "POST",
                        "/api/v1/sessions".into(),
                        Some(
                            json!({"metadata":{"cwd":cwd},"agent_config":{"permission_mode":"manual"}}),
                        ),
                    )
                }
            }
            // Pi resumes session FILES, never remote IDs. Owner admits file path
            // and supplies --session at launch; switch_session is not an ID API.
            NativeKind::Pi => {
                return Err("Pi session files must be admitted separately at launch.".into())
            }
            // Claude --resume/session-id is a launch-time owned binding.
            NativeKind::Claude => {
                return Err("Claude session binding belongs in admitted launch arguments.".into())
            }
            NativeKind::Agy => {
                return Err("Agy resume/control requires native PTY qualification.".into())
            }
        })
    }
    pub(crate) fn event_stream(&self) -> Result<WireRequest, String> {
        match self.kind {
            NativeKind::Kilo | NativeKind::OpenCode => Ok(http("GET", "/event".into(), None)),
            NativeKind::Kimi => Ok(WireRequest::Websocket {
                path: "/api/v1/ws".into(),
            }),
            _ => Err("Native events use stdio.".into()),
        }
    }
    /// Ordered, unfiltered read requests. Separate HTTP reads are observations,
    /// not an atomic process/child drain proof; the owner must retain that fence.
    pub(crate) fn idle_snapshot_requests(&self) -> Result<Vec<WireRequest>, String> {
        let session = segment(&self.session)?;
        let paths = match self.kind {
            NativeKind::Kimi => vec![
                format!("/api/v1/sessions/{session}"),
                format!("/api/v1/sessions/{session}/tasks"),
                format!("/api/v1/sessions/{session}/approvals?status=pending"),
                format!("/api/v1/sessions/{session}/snapshot"),
            ],
            NativeKind::Kilo | NativeKind::OpenCode => vec![
                format!("/session/{session}"),
                "/session/status".into(),
                format!("/session/{session}/message"),
                "/permission".into(),
                "/question".into(),
            ],
            _ => return Err("No qualified native idle snapshot contract.".into()),
        };
        Ok(paths
            .into_iter()
            .map(|path| http("GET", path, None))
            .collect())
    }
    pub(crate) fn observed_journal_cursor(&self) -> Option<(u64, String)> {
        Some((self.journal_seq?, self.journal_epoch.clone()?))
    }
    /// Pump the owned subscription through this exact watermark before closing.
    /// If later durable events advance the cursor, fetch and reconcile again.
    pub(crate) fn idle_snapshot_watermark(
        &self,
        views: &[Value],
    ) -> Result<Option<(u64, String)>, String> {
        if self.kind != NativeKind::Kimi {
            return Ok(None);
        }
        if views.len() != 4 {
            return Err("Kimi snapshot set is incomplete.".into());
        }
        bounds(&views[3], 0)?;
        encoded_size(&views[3])?;
        let snapshot = snapshot_body(&views[3], true)?;
        let seq = snapshot["as_of_seq"]
            .as_u64()
            .ok_or("Kimi snapshot has no journal watermark.")?;
        let epoch = snapshot["epoch"]
            .as_str()
            .ok_or("Kimi snapshot has no journal epoch.")?;
        if epoch.is_empty()
            || epoch.len() > 256
            || Some(epoch) != self.journal_epoch.as_deref()
            || snapshot["session"]["id"] != self.session
        {
            return Err("Kimi snapshot journal belongs to another subscription.".into());
        }
        Ok(Some((seq, epoch.into())))
    }
    /// Require a previously correlated native terminal. This validates quiet
    /// snapshots and final-message identity, and never manufactures completion.
    /// Kimi rest-session/rest-task/approval schemas; OpenCode/Kilo legacy
    /// httpapi groups session, permission, question and SessionStatus.list/set.
    pub(crate) fn validate_idle_snapshots(&self, cwd: &str, views: &[Value]) -> Result<(), String> {
        if self.failure == Some(ErrorClass::Protocol) || !self.permissions.is_empty() {
            return Err("Native observation or pending approval is unresolved.".into());
        }
        let terminal = self
            .terminal
            .as_ref()
            .ok_or("Idle snapshots require a correlated native terminal first.")?;
        for view in views {
            bounds(view, 0)?;
            encoded_size(view)?;
        }
        match self.kind {
            NativeKind::Kimi => {
                if views.len() != 4
                    || !self.subscription_acknowledged
                    || self.journal_seq.is_none()
                    || self.journal_epoch.is_none()
                {
                    return Err(
                        "Kimi idle snapshot set or journal continuity is incomplete.".into(),
                    );
                }
                let session = snapshot_body(&views[0], true)?;
                let tasks = snapshot_body(&views[1], true)?;
                let approvals = snapshot_body(&views[2], true)?;
                let snapshot = snapshot_body(&views[3], true)?;
                let reason = match terminal.outcome {
                    NativeOutcome::Completed => "completed",
                    NativeOutcome::Failed => "failed",
                    NativeOutcome::Cancelled => "cancelled",
                    _ => {
                        return Err(
                            "Kimi terminal outcome cannot be reconciled with an idle session."
                                .into(),
                        )
                    }
                };
                if session["id"] != self.session
                    || session["metadata"]["cwd"] != cwd
                    || session["agent_config"]["model"].as_str() != self.model.as_deref()
                    || session["busy"] != false
                    || session["main_turn_active"] != false
                    || session["pending_interaction"] != "none"
                    || !session["current_prompt_id"].is_null()
                    || session["last_turn_reason"] != reason
                    || session["last_seq"].as_u64().is_none_or(|seq| {
                        snapshot["as_of_seq"]
                            .as_u64()
                            .is_none_or(|watermark| seq > watermark)
                    })
                {
                    return Err("Kimi session remains active or does not match the observed owned terminal.".into());
                }
                // Tasks include native bash/tool/subagent background work. Their
                // independent child lifecycle is not qualified by this adapter;
                // even a historical completed task needs explicit owner review.
                require_empty_array(
                    tasks.get("items"),
                    "Kimi background tasks are untracked or still active.",
                )?;
                require_empty_array(approvals.get("items"), "Kimi approvals remain pending.")?;
                if snapshot["session"]["id"] != self.session
                    || snapshot["session"]["busy"] != false
                    || snapshot["session"]["main_turn_active"] != false
                    || snapshot["session"]["pending_interaction"] != "none"
                    || !snapshot["in_flight_turn"].is_null()
                    || snapshot["epoch"].as_str() != self.journal_epoch.as_deref()
                    || snapshot["as_of_seq"].as_u64() != self.journal_seq
                {
                    return Err(
                        "Kimi combined snapshot is active or does not match the observed journal."
                            .into(),
                    );
                }
                require_empty_array(
                    snapshot.get("subagents"),
                    "Kimi subagent child settlement is unqualified.",
                )?;
                require_empty_array(
                    snapshot.get("pending_approvals"),
                    "Kimi snapshot approvals remain pending.",
                )?;
                require_empty_array(
                    snapshot.get("pending_questions"),
                    "Kimi snapshot questions remain pending.",
                )?;
                Ok(())
            }
            NativeKind::Kilo | NativeKind::OpenCode => {
                if views.len() != 5 {
                    return Err("Native idle snapshot set is incomplete.".into());
                }
                let session = snapshot_body(&views[0], false)?;
                let statuses = snapshot_body(&views[1], false)?;
                let messages = snapshot_body(&views[2], false)?
                    .as_array()
                    .ok_or("Native messages snapshot is not a complete array.")?;
                if session["id"] != self.session
                    || session["directory"] != cwd
                    || !statuses.is_object()
                {
                    return Err(
                        "Native session snapshot does not match its owned workspace.".into(),
                    );
                }
                // Idle entries are removed from the native status map. Absence
                // qualifies only alongside the exact existing owned session.
                if statuses
                    .get(&self.session)
                    .is_some_and(|s| s["type"] != "idle")
                {
                    return Err("Native session still has running or retry work.".into());
                }
                require_empty_array(
                    Some(snapshot_body(&views[3], false)?),
                    "Native permissions remain pending.",
                )?;
                require_empty_array(
                    Some(snapshot_body(&views[4], false)?),
                    "Native questions remain pending.",
                )?;
                let user_id = format!("msg_{}", self.host_turn);
                let mut user_found = false;
                let mut ids = BTreeSet::new();
                let mut final_assistant: Option<&Value> = None;
                for message in messages {
                    let info = &message["info"];
                    let id = info["id"]
                        .as_str()
                        .ok_or("Native snapshot message has no ID.")?;
                    if info["sessionID"] != self.session || !ids.insert(id) {
                        return Err(
                            "Native snapshot contains uncorrelated or duplicate messages.".into(),
                        );
                    }
                    if info["role"] == "user" && info["id"] == user_id {
                        user_found = true;
                    }
                    if info["role"] != "assistant" || info["parentID"] != user_id {
                        continue;
                    }
                    if !self.assistant_ids.contains(id)
                        || info["modelID"].as_str() != self.model.as_deref()
                        || info["providerID"].as_str() != self.provider.as_deref()
                        || info
                            .pointer("/time/completed")
                            .and_then(Value::as_u64)
                            .is_none()
                    {
                        return Err("Native final assistant message is incomplete or was not observed for the exact model.".into());
                    }
                    let parts = message["parts"]
                        .as_array()
                        .ok_or("Native assistant snapshot omits its tool parts.")?;
                    for part in parts {
                        if part["sessionID"] != self.session || part["messageID"] != info["id"] {
                            return Err("Native assistant part is uncorrelated.".into());
                        }
                        if part["type"] == "tool" {
                            let call = part["callID"]
                                .as_str()
                                .ok_or("Native snapshot tool lacks a call ID.")?;
                            if !self.tools.contains_key(call)
                                || !matches!(
                                    part["state"]["status"].as_str(),
                                    Some("completed" | "error")
                                )
                            {
                                return Err(
                                    "Native snapshot contains running or unobserved tools.".into(),
                                );
                            }
                        }
                    }
                    if final_assistant.is_none_or(|previous| {
                        info.pointer("/time/created").and_then(Value::as_u64)
                            > previous.pointer("/time/created").and_then(Value::as_u64)
                    }) {
                        final_assistant = Some(info);
                    }
                }
                let final_message = final_assistant
                    .ok_or("Native final assistant does not match the owned prompt.")?;
                if !user_found
                    || final_message
                        .pointer("/time/created")
                        .and_then(Value::as_u64)
                        .is_none()
                    || !assistant_terminal_boundary(final_message)
                {
                    return Err(
                        "Native final message cannot prove the observed terminal boundary.".into(),
                    );
                }
                let failed = final_message.get("error").is_some_and(|e| !e.is_null());
                if !matches!(
                    (&terminal.outcome, failed),
                    (NativeOutcome::Completed, false) | (NativeOutcome::Failed, true)
                ) {
                    return Err(
                        "Native final assistant outcome differs from its observed terminal.".into(),
                    );
                }
                Ok(())
            }
            _ => Err("No qualified native idle snapshot contract.".into()),
        }
    }
    pub(crate) fn websocket_hello(&mut self, id: &str) -> Result<Value, String> {
        segment(id)?;
        segment(&self.session)?;
        if self.kind != NativeKind::Kimi {
            return Err("No WebSocket hello contract for this native client.".into());
        }
        if self.prompt_request.is_some() || self.ws_hello_id.is_some() {
            return Err("Kimi hello cannot be repeated or sent during an active prompt.".into());
        }
        self.remember(id, "ws_hello")?;
        self.ws_hello_id = Some(id.into());
        self.subscription_acknowledged = false;
        Ok(json!({"type":"client_hello","id":id,"payload":{"subscriptions":[self.session]}}))
    }
    pub(crate) fn configure_model(
        &mut self,
        id: &str,
        model: &str,
        provider: Option<&str>,
        reasoning: Option<&str>,
    ) -> Result<Vec<WireRequest>, String> {
        model_name(model)?;
        if let Some(p) = provider {
            model_name(p)?;
        }
        if let Some(r) = reasoning {
            model_name(r)?;
        }
        if self.prompt_request.is_some() {
            return Err("Cannot change native model during an active prompt.".into());
        }
        if reasoning.is_some() && matches!(self.kind, NativeKind::Claude | NativeKind::Grok) {
            return Err("This native wire has no qualified reasoning control; admit the exact launch setting separately.".into());
        }
        if matches!(
            self.kind,
            NativeKind::Pi | NativeKind::Kilo | NativeKind::OpenCode
        ) && provider.is_none()
        {
            return Err("Native model requires an exact provider ID.".into());
        }
        let requests = match self.kind {
            NativeKind::Claude => {
                self.remember(id, "model")?;
                vec![WireRequest::Stdio(
                    json!({"type":"control_request","request_id":id,"request":{"subtype":"set_model","model":model}}),
                )]
            }
            NativeKind::Grok => {
                segment(&self.session)?;
                self.remember(id, "model")?;
                vec![rpc(
                    id,
                    "session/set_model",
                    json!({"sessionId":self.session,"modelId":model}),
                )]
            }
            NativeKind::Pi => {
                self.remember(id, "model")?;
                let mut commands = vec![WireRequest::Stdio(
                    json!({"id":id,"type":"set_model","provider":provider,"modelId":model}),
                )];
                if let Some(level) = reasoning {
                    let rid = format!("{id}-thinking");
                    self.remember(&rid, "thinking")?;
                    commands.push(WireRequest::Stdio(
                        json!({"id":rid,"type":"set_thinking_level","level":level}),
                    ));
                }
                commands
            }
            NativeKind::Agy => {
                return Err(
                    "Agy coding model configuration requires native PTY qualification.".into(),
                )
            }
            _ => vec![],
        };
        self.model = Some(model.into());
        self.provider = provider.map(str::to_owned);
        self.reasoning = reasoning.map(str::to_owned);
        Ok(requests)
    }
    pub(crate) fn prompt(&mut self, id: &str, text: &str) -> Result<WireRequest, String> {
        if !self.ready_for_prompt() {
            return Err(
                "Native initialization/session/model acknowledgements are still pending.".into(),
            );
        }
        let model = self
            .model
            .clone()
            .ok_or("An exact native model must be configured before prompting.")?;
        if text.len() > MAX_FRAME / 2 || self.prompt_request.is_some() {
            return Err("Prompt bound exceeded or another turn is active.".into());
        }
        if self.session.is_empty() && !matches!(self.kind, NativeKind::Claude | NativeKind::Pi) {
            return Err("Native prompt requires a session binding.".into());
        }
        self.remember(id, "prompt")?;
        self.prompt_request = Some(id.into());
        Ok(match self.kind {
            NativeKind::Claude => WireRequest::Stdio(
                json!({"type":"user","uuid":id,"session_id":self.session,"parent_tool_use_id":null,"message":{"role":"user","content":text}}),
            ),
            NativeKind::Codex => rpc(
                id,
                "turn/start",
                json!({"threadId":self.session,"model":model,"effort":self.reasoning,"input":[{"type":"text","text":text}]}),
            ),
            NativeKind::Grok => rpc(
                id,
                "session/prompt",
                json!({"sessionId":self.session,"prompt":[{"type":"text","text":text}]}),
            ),
            NativeKind::Pi => WireRequest::Stdio(json!({"id":id,"type":"prompt","message":text})),
            NativeKind::Kimi => {
                let mut body = json!({"prompt_id":id,"model":model,"permission_mode":"manual","plan_mode":false,"swarm_mode":false,"content":[{"type":"text","text":text}]});
                if let Some(r) = &self.reasoning {
                    body["thinking"] = json!(r);
                }
                http(
                    "POST",
                    format!("/api/v1/sessions/{}/prompts", segment(&self.session)?),
                    Some(body),
                )
            }
            NativeKind::Kilo | NativeKind::OpenCode => {
                let mut body = json!({"messageID":format!("msg_{}",self.host_turn),"model":{"providerID":self.provider,"modelID":model},"parts":[{"type":"text","text":text}]});
                if let Some(r) = &self.reasoning {
                    body["variant"] = json!(r);
                }
                http(
                    "POST",
                    format!("/session/{}/prompt_async", segment(&self.session)?),
                    Some(body),
                )
            }
            NativeKind::Agy => {
                return Err("Agy coding prompt must retain the native PTY approval surface.".into())
            }
        })
    }

    pub(crate) fn permission_reply(
        &mut self,
        request_id: &Value,
        choice: Value,
    ) -> Result<WireRequest, String> {
        let key = request_id.to_string();
        let permission = self
            .permissions
            .get(&key)
            .ok_or("Unknown or already resolved native permission.")?;
        let p = &permission.raw;
        let request = match self.kind {
            NativeKind::Grok => {
                let option = choice.as_str().ok_or("ACP option ID must be a string.")?;
                if !permission
                    .choices
                    .iter()
                    .any(|c| c.get("optionId").and_then(Value::as_str) == Some(option))
                {
                    return Err("ACP choice was not offered by the native client.".into());
                }
                WireRequest::Stdio(
                    json!({"jsonrpc":"2.0","id":request_id,"result":{"outcome":{"outcome":"selected","optionId":option}}}),
                )
            }
            NativeKind::Claude => {
                if p.pointer("/request/requires_user_interaction")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    return Err("This Claude permission requires the native session UI.".into());
                }
                let decision = choice
                    .as_str()
                    .ok_or("Claude decision must be allow or deny.")?;
                let response = match decision {
                    "allow" => {
                        json!({"behavior":"allow","updatedInput":p.pointer("/request/input").cloned().unwrap_or(json!({}))})
                    }
                    "deny" => json!({"behavior":"deny","message":"Denied by host user"}),
                    _ => return Err("Unsupported Claude permission decision.".into()),
                };
                WireRequest::Stdio(
                    json!({"type":"control_response","response":{"subtype":"success","request_id":request_id,"response":response}}),
                )
            }
            NativeKind::Pi => {
                let method = p.get("method").and_then(Value::as_str).unwrap_or("");
                let mut body = json!({"type":"extension_ui_response","id":request_id});
                if choice.is_null() {
                    body["cancelled"] = json!(true);
                } else if method == "confirm" {
                    if !choice.is_boolean() {
                        return Err("Pi confirm requires a boolean.".into());
                    }
                    body["confirmed"] = choice;
                } else {
                    if !choice.is_string() {
                        return Err("Pi dialog requires a string.".into());
                    }
                    if method == "select" && !permission.choices.contains(&choice) {
                        return Err("Pi selection was not offered.".into());
                    }
                    body["value"] = choice;
                }
                WireRequest::Stdio(body)
            }
            NativeKind::Codex => {
                if !permission.choices.contains(&choice) {
                    return Err("Codex decision was not offered.".into());
                }
                WireRequest::Stdio(json!({"id":request_id,"result":{"decision":choice}}))
            }
            NativeKind::Kilo | NativeKind::OpenCode => {
                if !permission.choices.contains(&choice) {
                    return Err("Native permission choice was not offered.".into());
                }
                http(
                    "POST",
                    format!(
                        "/permission/{}/reply",
                        segment(request_id.as_str().ok_or("Invalid permission ID.")?)?
                    ),
                    Some(json!({"reply":choice})),
                )
            }
            NativeKind::Kimi => {
                if !permission.choices.contains(&choice) {
                    return Err("Kimi decision was not offered.".into());
                }
                http(
                    "POST",
                    format!(
                        "/api/v1/sessions/{}/approvals/{}",
                        segment(&self.session)?,
                        segment(request_id.as_str().ok_or("Invalid approval ID.")?)?
                    ),
                    Some(json!({"decision":choice})),
                )
            }
            NativeKind::Agy => {
                return Err(
                    "Agy headless hooks cannot enforce host approval; retain native PTY.".into(),
                )
            }
        };
        if let Some(p) = self.permissions.remove(&key) {
            self.ledger_bytes = self.ledger_bytes.saturating_sub(encoded_size(&p.raw)?);
        }
        Ok(request)
    }

    /// SSE data/WS envelope must first be decoded with strict_json; this API
    /// validates value bounds again, but cannot recover duplicate keys lost by
    /// an upstream permissive decoder.
    pub(crate) fn observe(&mut self, raw: Value) -> Result<Vec<NativeEvent>, String> {
        let events = self.decode_observation(raw)?;
        for event in &events {
            self.validate_event(event, true)?;
        }
        Ok(events)
    }

    /// Check normalized metadata against the owned codec state before exposing
    /// events. This also guards future decoders from emitting a foreign binding
    /// even when their source-envelope parser has accepted the record.
    fn validate_event(&self, event: &NativeEvent, source_request_id: bool) -> Result<(), String> {
        let binding = |session: &str, turn: Option<&str>| -> Result<(), String> {
            if session != self.session || turn.is_some_and(|id| id != self.turn_id()) {
                return Err("Decoded native event disagrees with its owned binding.".into());
            }
            Ok(())
        };
        let raw = match event {
            NativeEvent::Accepted { request_id, raw } => {
                let id = string_id(Some(request_id))
                    .ok_or("Decoded native acceptance lacks a request identifier.")?;
                segment(&id)?;
                let source_id = raw
                    .get("id")
                    .or_else(|| raw.pointer("/response/request_id"));
                if source_request_id && source_id.is_some_and(|source| source != request_id) {
                    return Err(
                        "Decoded native acceptance disagrees with its source request.".into(),
                    );
                }
                Some(raw)
            }
            NativeEvent::Session { session_id, raw } => {
                binding(session_id, None)?;
                Some(raw)
            }
            NativeEvent::TurnStarted {
                session_id,
                turn_id,
                raw,
            } => {
                binding(session_id, Some(turn_id))?;
                Some(raw)
            }
            NativeEvent::Text {
                session_id,
                turn_id,
                ..
            } => {
                binding(session_id, Some(turn_id))?;
                None
            }
            NativeEvent::Terminal(terminal) => {
                binding(&terminal.session_id, Some(&terminal.turn_id))?;
                let stored = self
                    .terminal
                    .as_ref()
                    .ok_or("Decoded native terminal is not recorded.")?;
                if terminal.session_id != stored.session_id
                    || terminal.turn_id != stored.turn_id
                    || terminal.outcome != stored.outcome
                    || terminal.error_class != stored.error_class
                    || terminal.raw != stored.raw
                {
                    return Err(
                        "Decoded native terminal disagrees with its recorded snapshot.".into(),
                    );
                }
                Some(&terminal.raw)
            }
            NativeEvent::Error { raw, .. } => Some(raw),
            NativeEvent::Tool(_)
            | NativeEvent::Permission(_)
            | NativeEvent::PermissionWithdrawn { .. }
            | NativeEvent::Observed(_) => None,
        };
        // Native HTTP acceptance may carry a null/array body, while observation
        // envelopes are objects. Recheck bounded metadata without imposing an
        // object-only schema on valid HTTP acknowledgements or housekeeping.
        if let Some(raw) = raw {
            bounds(raw, 0)?;
            encoded_size(raw)?;
        }
        Ok(())
    }

    fn decode_observation(&mut self, raw: Value) -> Result<Vec<NativeEvent>, String> {
        bounds(&raw, 0)?;
        encoded_size(&raw)?;
        if !raw.is_object() {
            return Err("Native record must be an object.".into());
        }
        if self.kind == NativeKind::Kimi {
            if raw["type"] == "ack" {
                return self.subscription_ack(raw);
            }
            if raw["type"] == "resync_required" {
                return Ok(self.protocol_error(raw));
            }
            if raw.get("session_id").and_then(Value::as_str) == Some(self.session.as_str()) {
                if !self.subscription_acknowledged {
                    return Ok(vec![NativeEvent::Observed(raw)]);
                }
                if !self.journal_contiguous(&raw) {
                    return Ok(self.protocol_error(raw));
                }
                if self.prompt_request.is_none() {
                    return Ok(vec![NativeEvent::Observed(raw)]);
                }
            } else if raw.get("seq").is_some()
                || (self.prompt_request.is_some()
                    && !matches!(raw["type"].as_str(), Some("ping" | "pong")))
            {
                return Ok(self.protocol_error(raw));
            }
        }
        if self.terminal.is_some() {
            return Ok(
                if terminal_housekeeping(
                    self.kind,
                    &raw,
                    self.prompt_request.as_deref(),
                    self.terminal.as_ref(),
                ) {
                    vec![NativeEvent::Observed(raw)]
                } else {
                    self.protocol_error(raw)
                },
            );
        }
        let mut out = Vec::new();
        let v = if matches!(
            self.kind,
            NativeKind::Kilo | NativeKind::OpenCode | NativeKind::Kimi
        ) {
            raw.get("payload")
                .filter(|p| p.get("type").is_some())
                .unwrap_or(&raw)
        } else {
            &raw
        };
        let params = v.get("params").unwrap_or(v);
        let supplied_session = string_id(
            params
                .get("sessionId")
                .or_else(|| params.get("session_id"))
                .or_else(|| params.get("threadId")),
        )
        .or_else(|| string_id(v.get("session_id")))
        .or_else(|| string_id(raw.get("session_id")));
        if let Some(s) = supplied_session {
            self.bind_session(&s)?;
        }
        let method = v.get("method").and_then(Value::as_str).unwrap_or("");
        let typ = v.get("type").and_then(Value::as_str).unwrap_or(method);
        if self.kind == NativeKind::Kimi
            && self.prompt_request.is_some()
            && (typ.starts_with("subagent.")
                || typ.starts_with("task.")
                || typ.starts_with("background.task.")
                || typ == "cron.fired")
        {
            // These effects outlive the parent's turn and require a separately
            // qualified child/task supervisor. Preserve evidence before fencing.
            if let Some(id) = string_id(v.get("subagentId").or_else(|| v.pointer("/info/taskId"))) {
                self.tool(
                    &format!("background_{id}"),
                    ToolState::Started,
                    v.clone(),
                    &mut out,
                )?;
            }
            out.extend(self.protocol_error(raw));
            return Ok(out);
        }
        if self.kind == NativeKind::Kimi && typ == "turn.started" {
            let prompt = v.get("promptId").and_then(Value::as_str);
            if prompt != self.prompt_request.as_deref() {
                return Ok(vec![NativeEvent::Observed(raw)]);
            }
            self.native_agent = string_id(v.get("agentId"));
        }
        if self.kind == NativeKind::Kimi {
            if let (Some(expected), Some(actual)) = (
                self.native_agent.as_deref(),
                v.get("agentId").and_then(Value::as_str),
            ) {
                if expected != actual {
                    return Ok(vec![NativeEvent::Observed(raw)]);
                }
            }
        }
        let supplied_turn = string_id(params.get("turnId").or_else(|| params.get("turn_id")))
            .or_else(|| string_id(params.pointer("/turn/id")));
        if self.kind == NativeKind::Kimi
            && supplied_turn.is_some()
            && self.native_turn.is_none()
            && typ != "turn.started"
        {
            return Ok(vec![NativeEvent::Observed(raw)]);
        }
        if let Some(t) = supplied_turn {
            self.bind_turn(&t)?;
        }
        if let Some(id) = string_id(v.get("id")) {
            if let Some(operation) = self.requests.remove(&id) {
                if let Some(error) = v.get("error").filter(|x| !x.is_null()) {
                    let class = if self.kind == NativeKind::Grok && operation == "authenticate" {
                        ErrorClass::Authentication
                    } else {
                        classify_error(error)
                    };
                    self.failure = Some(class.clone());
                    out.push(NativeEvent::Error {
                        class,
                        raw: raw.clone(),
                    });
                } else if v.get("result").is_some() || typ == "response" {
                    let result = v
                        .get("result")
                        .or_else(|| v.get("data"))
                        .unwrap_or(&Value::Null)
                        .clone();
                    if self.kind == NativeKind::Grok && operation == "initialize" {
                        self.grok_cached_token_advertised =
                            result["authMethods"].as_array().is_some_and(|methods| {
                                methods.iter().any(|method| method["id"] == "cached_token")
                            });
                    }
                    if self.kind == NativeKind::Grok && operation == "authenticate" {
                        if result
                            .pointer("/_meta/auth_mode")
                            .and_then(Value::as_str)
                            .is_none()
                        {
                            self.failure = Some(ErrorClass::Authentication);
                            out.push(NativeEvent::Error {
                                class: ErrorClass::Authentication,
                                raw,
                            });
                            return Ok(out);
                        }
                        self.grok_authenticated = true;
                    }
                    if typ == "response" && v.get("success").and_then(Value::as_bool) == Some(false)
                    {
                        self.failure = Some(ErrorClass::Unknown);
                        out.push(NativeEvent::Error {
                            class: ErrorClass::Unknown,
                            raw: raw.clone(),
                        });
                    } else {
                        if matches!(operation.as_str(), "session" | "resume")
                            || (self.kind == NativeKind::Pi && operation == "initialize")
                        {
                            if let Some(s) = string_id(
                                result
                                    .get("sessionId")
                                    .or_else(|| result.pointer("/thread/id")),
                            ) {
                                self.bind_session(&s)?;
                                out.push(NativeEvent::Session {
                                    session_id: s,
                                    raw: raw.clone(),
                                });
                            }
                        }
                        if operation == "prompt" {
                            if let Some(t) = string_id(result.pointer("/turn/id")) {
                                self.bind_turn(&t)?;
                            }
                            if self.kind == NativeKind::Grok {
                                let reason = result
                                    .get("stopReason")
                                    .and_then(Value::as_str)
                                    .ok_or("ACP prompt response lacks stopReason.")?;
                                self.finish(stop_outcome(reason), raw.clone(), &mut out);
                            } else {
                                out.push(NativeEvent::Accepted {
                                    request_id: v["id"].clone(),
                                    raw: raw.clone(),
                                });
                            }
                        } else {
                            out.push(NativeEvent::Accepted {
                                request_id: v["id"].clone(),
                                raw: raw.clone(),
                            });
                        }
                    }
                }
            }
        }
        // Host-correlated HTTP acceptance must be supplied explicitly by owner;
        // anonymous server responses are never inferred as turn completion.
        match self.kind {
            NativeKind::Claude | NativeKind::Agy => self.stream_json(v, typ, &mut out)?,
            NativeKind::Codex => self.codex(v, params, method, &mut out)?,
            NativeKind::Grok => self.acp(v, params, method, &mut out)?,
            NativeKind::Pi => self.pi(v, typ, &mut out)?,
            NativeKind::Kimi => self.kimi(v, typ, &mut out)?,
            NativeKind::Kilo | NativeKind::OpenCode => self.opencode(v, typ, &mut out)?,
        }
        if matches!(self.kind, NativeKind::Codex | NativeKind::Grok)
            && !method.is_empty()
            && v.get("id").is_some_and(|id| !id.is_null())
            && !out.iter().any(|event| {
                matches!(
                    event,
                    NativeEvent::Permission(_) | NativeEvent::Accepted { .. }
                )
            })
        {
            // Native user-input and MCP elicitation requests cannot be silently
            // dropped: their wait would hide unfinished native work. Preserve
            // normal notifications/responses and qualified permission replies.
            out.extend(self.protocol_error(raw.clone()));
        }
        if out.is_empty() {
            out.push(NativeEvent::Observed(raw));
        }
        Ok(out)
    }
    pub(crate) fn accepted_http(
        &mut self,
        request_id: &str,
        status: u16,
        raw: Value,
    ) -> Result<NativeEvent, String> {
        let event = self.decode_http_acceptance(request_id, status, raw)?;
        // HTTP response-body IDs belong to the native payload; ownership of
        // this acceptance is established by the host-correlated request ID.
        self.validate_event(&event, false)?;
        Ok(event)
    }

    fn decode_http_acceptance(
        &mut self,
        request_id: &str,
        status: u16,
        raw: Value,
    ) -> Result<NativeEvent, String> {
        bounds(&raw, 0)?;
        encoded_size(&raw)?;
        let operation = self
            .requests
            .remove(request_id)
            .ok_or("Uncorrelated native HTTP response.")?;
        if !(200..300).contains(&status) {
            let class = classify_error(&raw);
            self.failure = Some(class.clone());
            return Ok(NativeEvent::Error { class, raw });
        }
        let body = raw.get("data").unwrap_or(&raw);
        if operation == "session" || operation == "resume" {
            if let Some(s) = string_id(body.get("id").or_else(|| body.get("session_id"))) {
                self.bind_session(&s)?;
                return Ok(NativeEvent::Session { session_id: s, raw });
            }
            return Err("Native session response lacks an identifier.".into());
        }
        Ok(NativeEvent::Accepted {
            request_id: json!(request_id),
            raw,
        })
    }
    fn protocol_error(&mut self, raw: Value) -> Vec<NativeEvent> {
        self.failure = Some(ErrorClass::Protocol);
        self.subscription_acknowledged = false;
        vec![NativeEvent::Error {
            class: ErrorClass::Protocol,
            raw,
        }]
    }
    fn subscription_ack(&mut self, raw: Value) -> Result<Vec<NativeEvent>, String> {
        let id = raw["id"].as_str().unwrap_or("");
        let hello = self.ws_hello_id.as_deref() == Some(id);
        let subscribe = self.ws_subscribe_id.as_deref() == Some(id);
        if (!hello && !subscribe)
            || self.requests.remove(id).is_none()
            || raw["code"].as_i64() != Some(0)
            || !raw["msg"].is_string()
        {
            return Ok(self.protocol_error(raw));
        }
        let p = &raw["payload"];
        let accepted = if hello {
            &p["accepted_subscriptions"]
        } else {
            &p["accepted"]
        };
        let cursor = p.get("cursors").and_then(|c| c.get(&self.session));
        if accepted != &json!([self.session])
            || p["resync_required"] != json!([])
            || (subscribe && p["not_found"] != json!([]))
            || cursor
                .and_then(|c| c.get("seq"))
                .and_then(Value::as_u64)
                .is_none()
            || cursor
                .and_then(|c| c.get("epoch"))
                .and_then(Value::as_str)
                .is_none_or(|e| e.is_empty() || e.len() > 256 || e.chars().any(char::is_control))
        {
            return Ok(self.protocol_error(raw));
        }
        let cursor = cursor.ok_or("Kimi subscription lacks owned cursor.")?;
        self.journal_seq = cursor["seq"].as_u64();
        self.journal_epoch = cursor["epoch"].as_str().map(str::to_owned);
        self.subscription_acknowledged = true;
        Ok(vec![NativeEvent::Accepted {
            request_id: json!(id),
            raw,
        }])
    }
    fn journal_contiguous(&mut self, raw: &Value) -> bool {
        if raw["type"] != raw["payload"]["type"] {
            return false;
        }
        let Some(seq) = raw["seq"].as_u64() else {
            return false;
        };
        let Some(epoch) = raw["epoch"].as_str() else {
            return false;
        };
        let Some(previous) = self.journal_seq else {
            return false;
        };
        if self.journal_epoch.as_deref() != Some(epoch) {
            return false;
        }
        if raw["volatile"] == true {
            return seq == previous
                && matches!(
                    raw["type"].as_str(),
                    Some(
                        "assistant.delta"
                            | "thinking.delta"
                            | "tool.call.delta"
                            | "tool.progress"
                            | "shell.output"
                            | "shell.started"
                            | "shell.completed"
                            | "agent.status.updated"
                    )
                );
        }
        if raw
            .get("volatile")
            .is_some_and(|v| v != &Value::Bool(false))
            || raw.get("offset").is_some()
        {
            return false;
        }
        // Before dispatch, frames queued by subscription attachment may precede
        // its cursor. They cannot be attributed to this new logical prompt.
        if self.prompt_request.is_none() && seq <= previous {
            return true;
        }
        if previous.checked_add(1) != Some(seq) {
            return false;
        }
        self.journal_seq = Some(seq);
        true
    }
    fn bind_session(&mut self, id: &str) -> Result<(), String> {
        segment(id)?;
        if self.session.is_empty() {
            self.session = id.into();
        } else if self.session != id {
            return Err("Native event belongs to another session.".into());
        }
        Ok(())
    }
    fn bind_turn(&mut self, id: &str) -> Result<(), String> {
        segment(id)?;
        if let Some(t) = &self.native_turn {
            if t != id {
                return Err("Native event belongs to another turn.".into());
            }
        } else {
            if self.kind == NativeKind::Kimi && self.prompt_request.is_none() {
                return Err("Kimi turn has no owned prompt binding.".into());
            }
            self.native_turn = Some(id.into());
        }
        Ok(())
    }
    fn begin(&mut self, raw: Value, out: &mut Vec<NativeEvent>) {
        self.started = true;
        self.idle = false;
        out.push(NativeEvent::TurnStarted {
            session_id: self.session.clone(),
            turn_id: self.turn_id().into(),
            raw,
        });
    }
    fn text(&self, text: &str, out: &mut Vec<NativeEvent>) {
        if self.prompt_request.is_some() {
            out.push(NativeEvent::Text {
                session_id: self.session.clone(),
                turn_id: self.turn_id().into(),
                text: text.into(),
            });
        }
    }
    fn tool(
        &mut self,
        id: &str,
        state: ToolState,
        raw: Value,
        out: &mut Vec<NativeEvent>,
    ) -> Result<(), String> {
        segment(id)?;
        if self.tools.len() >= MAX_ITEMS && !self.tools.contains_key(id) {
            return Err("Native tool ledger bound exceeded.".into());
        }
        let previous = self.tools.get(id);
        let mut merged = previous.map(|tool| tool.raw.clone()).unwrap_or(json!({}));
        if let (Some(target), Some(fields)) = (merged.as_object_mut(), raw.as_object()) {
            target.extend(fields.iter().map(|(k, v)| (k.clone(), v.clone())));
        } else {
            merged = raw;
        }
        let old_size = previous
            .map(|tool| encoded_size(&tool.raw))
            .transpose()?
            .unwrap_or(0);
        let new_size = encoded_size(&merged)?;
        let new_total = self
            .ledger_bytes
            .saturating_sub(old_size)
            .checked_add(new_size)
            .ok_or("Native ledger bound exceeded.")?;
        if new_total > MAX_LEDGER_BYTES {
            return Err("Native ledger bound exceeded.".into());
        }
        // Late progress cannot downgrade a recorded terminal tool result.
        let state = previous
            .filter(|t| matches!(t.state, ToolState::Completed | ToolState::Failed))
            .map(|t| t.state.clone())
            .unwrap_or(state);
        let tool = NativeTool {
            session_id: self.session.clone(),
            turn_id: self.turn_id().into(),
            tool_id: id.into(),
            state,
            raw: merged,
        };
        self.ledger_bytes = new_total;
        self.tools.insert(id.into(), tool.clone());
        out.push(NativeEvent::Tool(tool));
        Ok(())
    }
    fn permission(
        &mut self,
        id: Value,
        tool: Option<String>,
        choices: Vec<Value>,
        raw: Value,
        out: &mut Vec<NativeEvent>,
    ) -> Result<(), String> {
        if string_id(Some(&id)).is_none() || self.permissions.len() >= MAX_ITEMS {
            return Err("Native permission identifier/bound invalid.".into());
        }
        let p = NativePermission {
            request_id: id.clone(),
            session_id: self.session.clone(),
            turn_id: self.turn_id().into(),
            tool_id: tool.clone(),
            choices,
            raw: raw.clone(),
        };
        if self.permissions.contains_key(&id.to_string()) {
            return Err("Duplicate native permission request.".into());
        }
        if let Some(t) = tool {
            self.tool(&t, ToolState::Pending, raw, out)?;
        }
        let size = encoded_size(&p.raw)?;
        if self.ledger_bytes.saturating_add(size) > MAX_LEDGER_BYTES {
            return Err("Native ledger bound exceeded.".into());
        }
        self.ledger_bytes += size;
        self.permissions.insert(id.to_string(), p.clone());
        out.push(NativeEvent::Permission(p));
        Ok(())
    }
    fn withdraw(&mut self, id: Value, out: &mut Vec<NativeEvent>) {
        if let Some(p) = self.permissions.remove(&id.to_string()) {
            self.ledger_bytes = self
                .ledger_bytes
                .saturating_sub(encoded_size(&p.raw).unwrap_or(0));
        }
        if string_id(Some(&id)).is_some() {
            out.push(NativeEvent::PermissionWithdrawn { request_id: id });
        }
    }
    fn finish(&mut self, outcome: NativeOutcome, raw: Value, out: &mut Vec<NativeEvent>) {
        if self.prompt_request.is_none() || self.terminal.is_some() {
            return;
        }
        let error_class = if outcome == NativeOutcome::Completed {
            None
        } else if outcome == NativeOutcome::Cancelled {
            Some(ErrorClass::Cancelled)
        } else {
            Some(self.failure.clone().unwrap_or_else(|| classify_error(&raw)))
        };
        let terminal = NativeTerminal {
            session_id: self.session.clone(),
            turn_id: self.turn_id().into(),
            outcome,
            error_class,
            raw,
        };
        self.terminal = Some(terminal.clone());
        out.push(NativeEvent::Terminal(terminal));
    }
    fn stream_json(
        &mut self,
        v: &Value,
        typ: &str,
        out: &mut Vec<NativeEvent>,
    ) -> Result<(), String> {
        match typ {
            "control_response" if self.kind == NativeKind::Claude => {
                let response = &v["response"];
                if let Some(id) = response["request_id"].as_str() {
                    if self.requests.remove(id).is_some() {
                        if response["subtype"] == "success" {
                            out.push(NativeEvent::Accepted {
                                request_id: json!(id),
                                raw: v.clone(),
                            });
                            if let Some(pending) =
                                response["pending_permission_requests"].as_array()
                            {
                                for request in pending {
                                    self.stream_json(request, "control_request", out)?;
                                }
                            }
                        } else {
                            let class = classify_error(response);
                            self.failure = Some(class.clone());
                            out.push(NativeEvent::Error {
                                class,
                                raw: v.clone(),
                            });
                        }
                    }
                }
            }
            "system"
                if matches!(
                    v["subtype"].as_str(),
                    Some("hook_started" | "hook_progress" | "hook_response")
                ) =>
            {
                if let Some(id) = v["hook_id"].as_str() {
                    let key = format!("hook_{id}");
                    let state = if v["subtype"] == "hook_response" {
                        if matches!(v["outcome"].as_str(), Some("error" | "cancelled"))
                            || v["exit_code"].as_i64().is_some_and(|code| code != 0)
                        {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        }
                    } else {
                        self.tools
                            .get(&key)
                            .map(|t| t.state.clone())
                            .unwrap_or(ToolState::Started)
                    };
                    self.tool(&key, state, v.clone(), out)?;
                } else {
                    out.extend(self.protocol_error(v.clone()));
                }
            }
            "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                self.begin(v.clone(), out)
            }
            "stream_event" => {
                let event = &v["event"];
                if event["type"] == "content_block_delta" && event["delta"]["type"] == "text_delta"
                {
                    if let Some(t) = event["delta"]["text"].as_str() {
                        self.text(t, out);
                    }
                }
                if event["type"] == "content_block_start"
                    && event["content_block"]["type"] == "tool_use"
                {
                    if let Some(id) = event["content_block"]["id"].as_str() {
                        self.tool(id, ToolState::Started, event.clone(), out)?;
                    }
                }
            }
            "assistant" => {
                if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                    for block in content {
                        if block["type"] == "tool_use" {
                            if let Some(id) = block["id"].as_str() {
                                self.tool(id, ToolState::Started, block.clone(), out)?;
                            }
                        }
                    }
                }
            }
            "user" => {
                if let Some(content) = v.pointer("/message/content").and_then(Value::as_array) {
                    for block in content {
                        if block["type"] == "tool_result" {
                            if let Some(id) = block["tool_use_id"].as_str() {
                                self.tool(
                                    id,
                                    if block["is_error"] == true {
                                        ToolState::Failed
                                    } else {
                                        ToolState::Completed
                                    },
                                    block.clone(),
                                    out,
                                )?;
                            }
                        }
                    }
                }
            }
            "control_request"
                if self.kind == NativeKind::Claude && v["request"]["subtype"] == "can_use_tool" =>
            {
                self.permission(
                    v["request_id"].clone(),
                    string_id(v.pointer("/request/tool_use_id")),
                    vec![json!("allow"), json!("deny")],
                    v.clone(),
                    out,
                )?;
            }
            "control_cancel_request" => {
                self.withdraw(v["request_id"].clone(), out);
            }
            "result" => {
                let subtype = v["subtype"].as_str().unwrap_or("");
                let outcome = if subtype == "success" && v["is_error"] != true {
                    NativeOutcome::Completed
                } else if v["is_error"] == true || subtype.starts_with("error_") {
                    NativeOutcome::Failed
                } else {
                    NativeOutcome::Unknown
                };
                self.finish(outcome, v.clone(), out);
            }
            _ => {}
        }
        Ok(())
    }
    fn codex(
        &mut self,
        v: &Value,
        p: &Value,
        method: &str,
        out: &mut Vec<NativeEvent>,
    ) -> Result<(), String> {
        match method {
            "turn/started" => self.begin(v.clone(), out),
            "hook/started" | "hook/completed" => {
                if let Some(id) = p["run"]["id"].as_str() {
                    self.tool(
                        &format!("hook_{id}"),
                        if method == "hook/started" {
                            ToolState::Started
                        } else if matches!(
                            p["run"]["status"].as_str(),
                            Some("failed" | "error" | "blocked" | "stopped")
                        ) {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        },
                        v.clone(),
                        out,
                    )?;
                } else {
                    out.extend(self.protocol_error(v.clone()));
                }
            }
            "item/agentMessage/delta" => {
                if let Some(t) = p["delta"].as_str() {
                    self.text(t, out);
                }
            }
            "item/started" | "item/completed" => {
                let item = &p["item"];
                match item["type"].as_str().unwrap_or("") {
                    "collabAgentToolCall" | "subAgentActivity" => {
                        if let Some(id) = item["id"].as_str() {
                            self.tool(id, ToolState::Started, item.clone(), out)?;
                        }
                        // A completed spawn/collab call is not a child settlement
                        // proof. Its agentsStates/kind require a child supervisor.
                        out.extend(self.protocol_error(v.clone()));
                    }
                    "commandExecution" | "fileChange" | "mcpToolCall" | "dynamicToolCall"
                    | "webSearch" | "imageGeneration" | "imageView" | "sleep" => {
                        if let Some(id) = item["id"].as_str() {
                            self.tool(
                                id,
                                if method == "item/started" {
                                    ToolState::Started
                                } else if matches!(
                                    item["status"].as_str(),
                                    Some("failed" | "errored" | "interrupted" | "declined")
                                ) {
                                    ToolState::Failed
                                } else {
                                    ToolState::Completed
                                },
                                item.clone(),
                                out,
                            )?;
                        } else {
                            out.extend(self.protocol_error(v.clone()));
                        }
                    }
                    "userMessage" | "agentMessage" | "reasoning" | "plan" | "contextCompaction" => {
                    }
                    _ => {
                        // Preserve possible effect evidence even when its new
                        // native item schema cannot be interpreted safely.
                        if let Some(id) = item["id"].as_str() {
                            self.tool(id, ToolState::Started, item.clone(), out)?;
                        }
                        out.extend(self.protocol_error(v.clone()));
                    }
                }
            }
            "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                let choices = p
                    .get("availableDecisions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_else(|| vec![json!("accept"), json!("decline"), json!("cancel")]);
                self.permission(
                    v["id"].clone(),
                    string_id(p.get("itemId")),
                    choices,
                    v.clone(),
                    out,
                )?;
            }
            "turn/completed" => {
                let outcome = match p["turn"]["status"].as_str() {
                    Some("completed") => NativeOutcome::Completed,
                    Some("interrupted") => NativeOutcome::Cancelled,
                    Some("failed") => NativeOutcome::Failed,
                    _ => NativeOutcome::Unknown,
                };
                if let Some(error) = p.pointer("/turn/error").filter(|e| !e.is_null()) {
                    self.failure = Some(classify_error(error));
                }
                self.finish(outcome, v.clone(), out);
            }
            "error" => {
                let class = classify_error(p);
                self.failure = Some(class.clone());
                out.push(NativeEvent::Error {
                    class,
                    raw: v.clone(),
                });
            }
            _ => {}
        }
        Ok(())
    }
    fn acp(
        &mut self,
        v: &Value,
        p: &Value,
        method: &str,
        out: &mut Vec<NativeEvent>,
    ) -> Result<(), String> {
        if method == "session/request_permission" {
            self.permission(
                v["id"].clone(),
                string_id(p.pointer("/toolCall/toolCallId")),
                p["options"].as_array().cloned().unwrap_or_default(),
                v.clone(),
                out,
            )?;
        }
        if method == "session/update" {
            let update = &p["update"];
            match update["sessionUpdate"].as_str().unwrap_or("") {
                "agent_message_chunk" => {
                    if let Some(t) = update.pointer("/content/text").and_then(Value::as_str) {
                        self.text(t, out);
                    }
                }
                "tool_call" | "tool_call_update" => {
                    if let Some(id) = update["toolCallId"].as_str() {
                        let state = match update["status"].as_str() {
                            Some("completed") => ToolState::Completed,
                            Some("failed") => ToolState::Failed,
                            Some("pending") => ToolState::Pending,
                            Some("in_progress") => ToolState::Started,
                            _ => self
                                .tools
                                .get(id)
                                .map(|t| t.state.clone())
                                .unwrap_or(ToolState::Started),
                        };
                        self.tool(id, state, update.clone(), out)?;
                    }
                }
                // Vendor turn_completed updates are advisory; ACP prompt reply
                // stopReason is the qualified completion boundary.
                _ => {}
            }
        }
        Ok(())
    }
    fn pi(&mut self, v: &Value, typ: &str, out: &mut Vec<NativeEvent>) -> Result<(), String> {
        match typ {
            "agent_start" => self.begin(v.clone(), out),
            "message_update" => {
                if v["assistantMessageEvent"]["type"] == "text_delta" {
                    if let Some(t) = v["assistantMessageEvent"]["delta"].as_str() {
                        self.text(t, out);
                    }
                }
            }
            "tool_execution_start" | "tool_execution_end" => {
                if let Some(id) = v["toolCallId"].as_str() {
                    self.tool(
                        id,
                        if typ.ends_with("start") {
                            ToolState::Started
                        } else if v["isError"] == true {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        },
                        v.clone(),
                        out,
                    )?;
                }
            }
            "extension_ui_request" => {
                if matches!(
                    v["method"].as_str(),
                    Some("select" | "confirm" | "input" | "editor")
                ) {
                    self.permission(
                        v["id"].clone(),
                        None,
                        v["options"].as_array().cloned().unwrap_or_default(),
                        v.clone(),
                        out,
                    )?;
                }
            }
            "message_end" => match v["message"]["stopReason"].as_str() {
                Some("error") => self.failure = Some(ErrorClass::Unknown),
                Some("aborted") => self.failure = Some(ErrorClass::Cancelled),
                _ => {}
            },
            "agent_settled" if self.started => {
                let outcome = match self.failure.as_ref() {
                    Some(ErrorClass::Cancelled) => NativeOutcome::Cancelled,
                    Some(_) => NativeOutcome::Failed,
                    None => NativeOutcome::Completed,
                };
                self.finish(outcome, v.clone(), out);
            }
            _ => {}
        }
        Ok(())
    }
    fn kimi(&mut self, v: &Value, typ: &str, out: &mut Vec<NativeEvent>) -> Result<(), String> {
        match typ {
            "turn.started" => self.begin(v.clone(), out),
            "assistant.delta" => {
                if let Some(t) = v["delta"].as_str() {
                    self.text(t, out);
                }
            }
            "tool.call.started" | "tool.result" => {
                if let Some(id) = v["toolCallId"].as_str() {
                    self.tool(
                        id,
                        if typ == "tool.call.started" {
                            ToolState::Started
                        } else if v["isError"] == true {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        },
                        v.clone(),
                        out,
                    )?;
                }
            }
            "shell.started" | "shell.completed" | "shell.output" => {
                if let Some(id) = v["commandId"].as_str() {
                    let tool_id = format!("shell_{id}");
                    let state = if typ == "shell.completed" {
                        if v["isError"] == true {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        }
                    } else {
                        self.tools
                            .get(&tool_id)
                            .map(|t| t.state.clone())
                            .unwrap_or(ToolState::Started)
                    };
                    self.tool(&tool_id, state, v.clone(), out)?;
                } else {
                    out.extend(self.protocol_error(v.clone()));
                }
            }
            "tool.call.delta" | "tool.progress" => {
                if let Some(id) = v["toolCallId"].as_str() {
                    let state = self
                        .tools
                        .get(id)
                        .map(|t| t.state.clone())
                        .unwrap_or(ToolState::Started);
                    self.tool(id, state, v.clone(), out)?;
                } else {
                    out.extend(self.protocol_error(v.clone()));
                }
            }
            "event.approval.requested" => {
                if let Some(id) = v.get("approval_id") {
                    self.permission(
                        id.clone(),
                        string_id(v.get("tool_call_id")),
                        vec![json!("approved"), json!("rejected"), json!("cancelled")],
                        v.clone(),
                        out,
                    )?;
                }
            }
            "event.approval.resolved" => {
                self.withdraw(v["approval_id"].clone(), out);
            }
            "turn.ended" => {
                if let Some(e) = v.get("error").filter(|e| !e.is_null()) {
                    self.failure = Some(classify_error(e));
                }
                let outcome = match v["reason"].as_str() {
                    Some("completed") => NativeOutcome::Completed,
                    Some("cancelled") => NativeOutcome::Cancelled,
                    Some("failed") => NativeOutcome::Failed,
                    Some("blocked") => NativeOutcome::Blocked,
                    _ => NativeOutcome::Unknown,
                };
                self.finish(outcome, v.clone(), out);
            }
            _ if typ.starts_with("tool.") || typ.starts_with("shell.") => {
                out.extend(self.protocol_error(v.clone()))
            }
            _ => {}
        }
        Ok(())
    }
    fn opencode(&mut self, v: &Value, typ: &str, out: &mut Vec<NativeEvent>) -> Result<(), String> {
        let p = &v["properties"];
        if let Some(s) = string_id(
            p.get("sessionID")
                .or_else(|| p.pointer("/info/sessionID"))
                .or_else(|| p.pointer("/part/sessionID")),
        ) {
            // Shared SSE server carries other sessions; filter rather than bind.
            if s != self.session {
                return Ok(());
            }
        } else {
            return Ok(());
        }
        match typ {
            "session.status" => {
                self.idle = p["status"]["type"] == "idle";
                if !self.idle && p["status"]["type"] == "busy" {
                    self.begin(v.clone(), out);
                }
            }
            "session.idle" => self.idle = true,
            "message.updated" => {
                let info = &p["info"];
                if info["role"] != "assistant"
                    || info["parentID"] != format!("msg_{}", self.host_turn)
                {
                    return Ok(());
                }
                if let Some(id) = info["id"].as_str() {
                    segment(id)?;
                    if self.assistant_ids.len() >= MAX_ITEMS {
                        return Err("Native assistant message bound exceeded.".into());
                    }
                    self.assistant_ids.insert(id.into());
                }
                if assistant_terminal_boundary(info) {
                    self.assistant_complete = true;
                    if let Some(e) = info.get("error").filter(|e| !e.is_null()) {
                        self.failure = Some(classify_error(e));
                    }
                }
            }
            "message.part.delta" => {
                if p["field"] == "text"
                    && p["messageID"]
                        .as_str()
                        .is_some_and(|id| self.assistant_ids.contains(id))
                {
                    if let Some(t) = p["delta"].as_str() {
                        self.text(t, out);
                    }
                }
            }
            "message.part.updated" => {
                let part = &p["part"];
                if !part["messageID"]
                    .as_str()
                    .is_some_and(|id| self.assistant_ids.contains(id))
                {
                    return Ok(());
                }
                if part["type"] == "tool" {
                    if let Some(id) = part["callID"].as_str() {
                        let state = match part["state"]["status"].as_str() {
                            Some("completed") => ToolState::Completed,
                            Some("error") => ToolState::Failed,
                            Some("pending") => ToolState::Pending,
                            _ => ToolState::Started,
                        };
                        self.tool(id, state, part.clone(), out)?;
                    }
                }
            }
            "permission.asked" => {
                self.permission(
                    p["id"].clone(),
                    string_id(p.pointer("/tool/callID")),
                    vec![json!("once"), json!("always"), json!("reject")],
                    v.clone(),
                    out,
                )?;
            }
            "permission.replied" => {
                self.withdraw(p["requestID"].clone(), out);
            }
            "session.error" => {
                let class = classify_error(&p["error"]);
                self.failure = Some(class.clone());
                out.push(NativeEvent::Error {
                    class,
                    raw: v.clone(),
                });
            }
            _ => {}
        }
        if self.started && self.idle && self.assistant_complete {
            self.finish(
                if self.failure.is_some() {
                    NativeOutcome::Failed
                } else {
                    NativeOutcome::Completed
                },
                v.clone(),
                out,
            );
        }
        Ok(())
    }
}
fn snapshot_body(value: &Value, kimi: bool) -> Result<&Value, String> {
    if kimi && value.get("data").is_some() {
        if value["code"].as_i64() != Some(0) {
            return Err("Kimi snapshot response is unsuccessful.".into());
        }
        return Ok(&value["data"]);
    }
    Ok(value)
}
fn require_empty_array(value: Option<&Value>, reason: &str) -> Result<(), String> {
    if value
        .and_then(Value::as_array)
        .is_none_or(|items| !items.is_empty())
    {
        return Err(reason.into());
    }
    Ok(())
}
fn assistant_terminal_boundary(info: &Value) -> bool {
    info.pointer("/time/completed")
        .and_then(Value::as_u64)
        .is_some()
        && (info.get("error").is_some_and(|error| !error.is_null())
            || info["finish"].as_str().is_some_and(|finish| {
                !matches!(finish, "tool-calls" | "unknown") && !finish.is_empty()
            }))
}
fn terminal_housekeeping(
    kind: NativeKind,
    raw: &Value,
    prompt: Option<&str>,
    terminal: Option<&NativeTerminal>,
) -> bool {
    let v = raw
        .get("payload")
        .filter(|p| p.get("type").is_some())
        .unwrap_or(raw);
    let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
    match kind {
        NativeKind::Claude | NativeKind::Agy => match typ {
            "keep_alive" => true,
            "control_cancel_request" => true,
            "control_response" => v["response"]["subtype"] == "success",
            "system" => {
                v["subtype"] == "status" && (v["status"].is_null() || v["status"] == "idle")
            }
            _ => false,
        },
        NativeKind::Codex => match raw["method"].as_str().unwrap_or("") {
            "thread/status/changed" => raw["params"]["status"]["type"] == "idle",
            "serverRequest/resolved" => true,
            "thread/tokenUsage/updated" | "account/rateLimits/updated" => true,
            _ => false,
        },
        NativeKind::Kimi => match typ {
            "ping" | "pong" => true,
            "prompt.completed" => {
                v["promptId"].as_str() == prompt
                    && terminal.is_some_and(|t| {
                        v.get("reason").is_none()
                            || match t.outcome {
                                NativeOutcome::Completed => v["reason"] == "completed",
                                NativeOutcome::Failed => v["reason"] == "failed",
                                NativeOutcome::Blocked => v["reason"] == "blocked",
                                _ => false,
                            }
                    })
            }
            "prompt.aborted" => {
                v["promptId"].as_str() == prompt
                    && terminal.is_some_and(|t| t.outcome == NativeOutcome::Cancelled)
            }
            "event.session.status_changed" => {
                matches!(v["status"].as_str(), Some("idle" | "aborted"))
            }
            "event.session.work_changed" => {
                v["busy"] == false
                    && v["main_turn_active"] != true
                    && !matches!(
                        v["pending_interaction"].as_str(),
                        Some("approval" | "question")
                    )
            }
            "agent.status.updated" => {
                let Some(phase) = v.get("phase") else {
                    return true;
                };
                if phase["kind"] == "idle" {
                    return true;
                }
                phase["kind"] == "ended"
                    && terminal.is_some_and(|t| {
                        string_id(phase.get("turnId")).as_deref() == Some(t.turn_id.as_str())
                            && match t.outcome {
                                NativeOutcome::Completed => phase["reason"] == "completed",
                                NativeOutcome::Cancelled => phase["reason"] == "cancelled",
                                NativeOutcome::Failed => phase["reason"] == "failed",
                                NativeOutcome::Blocked => phase["reason"] == "blocked",
                                NativeOutcome::Unknown => false,
                            }
                    })
            }
            "event.approval.resolved" => true,
            _ => false,
        },
        NativeKind::Kilo | NativeKind::OpenCode => match typ {
            "server.heartbeat" | "permission.replied" => true,
            "session.status" => v["properties"]["status"]["type"] == "idle",
            "session.idle" => true,
            _ => false,
        },
        NativeKind::Pi | NativeKind::Grok => false,
    }
}
fn stop_outcome(reason: &str) -> NativeOutcome {
    match reason {
        "end_turn" => NativeOutcome::Completed,
        "cancelled" => NativeOutcome::Cancelled,
        "refusal" => NativeOutcome::Blocked,
        _ => NativeOutcome::Unknown,
    }
}
fn string_id(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
/// Only machine codes qualify a diagnosis. A 429 or prose rate-limit message
/// alone cannot distinguish quota exhaustion from transient throttling.
pub(crate) fn classify_error(value: &Value) -> ErrorClass {
    let code = value
        .get("codexErrorInfo")
        .filter(|v| v.is_string())
        .or_else(|| value.get("code"))
        .or_else(|| value.get("type"))
        .or_else(|| value.get("name"))
        .or_else(|| value.pointer("/error/code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match code {
        "insufficient_quota"
        | "quota_exceeded"
        | "usage_limit_reached"
        | "credits_exhausted"
        | "usageLimitExceeded" => ErrorClass::Quota,
        "rate_limit_exceeded" | "rate_limit_error" | "throttled" | "rateLimitExceeded" => {
            ErrorClass::Throttle
        }
        "authentication_error"
        | "invalid_api_key"
        | "unauthorized"
        | "AuthError"
        | "ProviderAuthError" => ErrorClass::Authentication,
        "permission_denied" | "permission_required" => ErrorClass::Permission,
        "cancelled" | "aborted" => ErrorClass::Cancelled,
        "parse_error" | "invalid_request" => ErrorClass::Protocol,
        _ => ErrorClass::Unknown,
    }
}

/// Reject duplicate keys at every depth BEFORE serde_json::Value loses them.
/// serde_json's recursion limit remains enabled. Total bytes, node count,
/// nesting and each array/map are bounded even for WS and HTTP payloads.
pub(crate) fn strict_json(bytes: &[u8]) -> Result<Value, String> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err("Native JSON frame bound exceeded.".into());
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let value = StrictValue::deserialize(&mut decoder)
        .map_err(|_| "Malformed or duplicate-key native JSON.".to_owned())?
        .0;
    decoder
        .end()
        .map_err(|_| "Trailing native JSON data.".to_owned())?;
    bounds(&value, 0)?;
    Ok(value)
}
fn bounds(value: &Value, depth: usize) -> Result<usize, String> {
    if depth > MAX_DEPTH {
        return Err("Native JSON depth exceeded.".into());
    }
    let mut nodes = 1;
    match value {
        Value::Array(items) => {
            if items.len() > MAX_ITEMS {
                return Err("Native array bound exceeded.".into());
            }
            for item in items {
                nodes += bounds(item, depth + 1)?;
                if nodes > MAX_ITEMS * 4 {
                    return Err("Native JSON node bound exceeded.".into());
                }
            }
        }
        Value::Object(items) => {
            if items.len() > MAX_ITEMS {
                return Err("Native object bound exceeded.".into());
            }
            for (key, item) in items {
                if key.len() > MAX_FRAME {
                    return Err("Native key bound exceeded.".into());
                }
                nodes += bounds(item, depth + 1)?;
                if nodes > MAX_ITEMS * 4 {
                    return Err("Native JSON node bound exceeded.".into());
                }
            }
        }
        Value::String(s) if s.len() > MAX_FRAME => {
            return Err("Native string bound exceeded.".into())
        }
        _ => {}
    }
    Ok(nodes)
}
fn encoded_size(value: &Value) -> Result<usize, String> {
    fn add(total: &mut usize, amount: usize) -> Result<(), String> {
        *total = total
            .checked_add(amount)
            .ok_or("Native frame bound exceeded.")?;
        if *total > MAX_FRAME {
            return Err("Native frame bound exceeded.".into());
        }
        Ok(())
    }
    fn string(s: &str, total: &mut usize) -> Result<(), String> {
        add(total, 2)?;
        for c in s.chars() {
            add(
                total,
                match c {
                    '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                    c if c <= '\u{1f}' => 6,
                    c => c.len_utf8(),
                },
            )?;
        }
        Ok(())
    }
    fn walk(v: &Value, total: &mut usize) -> Result<(), String> {
        match v {
            Value::Null => add(total, 4),
            Value::Bool(b) => add(total, if *b { 4 } else { 5 }),
            Value::Number(n) => add(total, n.to_string().len()),
            Value::String(s) => string(s, total),
            Value::Array(items) => {
                add(total, 2 + items.len().saturating_sub(1))?;
                for i in items {
                    walk(i, total)?;
                }
                Ok(())
            }
            Value::Object(items) => {
                add(total, 2 + items.len().saturating_sub(1) + items.len())?;
                for (k, v) in items {
                    string(k, total)?;
                    walk(v, total)?;
                }
                Ok(())
            }
        }
    }
    let mut size = 0;
    walk(value, &mut size)?;
    Ok(size)
}
struct StrictValue(Value);
impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StrictVisitor;
        impl<'de> Visitor<'de> for StrictVisitor {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bounded JSON without duplicate object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Bool(v)))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(json!(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(json!(v)))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| StrictValue(Value::Number(n)))
                    .ok_or_else(|| E::custom("non-finite number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                self.visit_string(v.into())
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                if v.len() > MAX_FRAME {
                    return Err(E::custom("string bound"));
                }
                Ok(StrictValue(Value::String(v)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element::<StrictValue>()? {
                    if items.len() >= MAX_ITEMS {
                        return Err(de::Error::custom("array bound"));
                    }
                    items.push(item.0);
                }
                Ok(StrictValue(Value::Array(items)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut items = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if items.len() >= MAX_ITEMS || items.contains_key(&key) {
                        return Err(de::Error::custom("duplicate key or map bound"));
                    }
                    items.insert(key, map.next_value::<StrictValue>()?.0);
                }
                Ok(StrictValue(Value::Object(items)))
            }
        }
        deserializer.deserialize_any(StrictVisitor)
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;

    fn authenticated_grok() -> NativeWire {
        let mut wire = NativeWire::new(NativeKind::Grok, "", "turn_owned").unwrap();
        wire.initialize("initialize-request").unwrap();
        wire.observe(json!({"jsonrpc":"2.0","id":"initialize-request","result":{"authMethods":[{"id":"cached_token","name":"Native cached credential"}]}})).unwrap();
        let request = wire
            .authentication_request("authenticate-request")
            .unwrap()
            .unwrap();
        assert!(
            matches!(request,WireRequest::Stdio(ref value) if value["method"] == "authenticate"
            && value["params"] == json!({"methodId":"cached_token","_meta":{"headless":true}}))
        );
        wire.observe(json!({"jsonrpc":"2.0","id":"authenticate-request","result":{"_meta":{"auth_mode":"native_cached"}}})).unwrap();
        wire
    }

    #[test]
    fn grok_session_and_prompt_require_correlated_native_authentication_success() {
        let mut wire = authenticated_grok();
        wire.session_request("session-request", "/owned", false)
            .unwrap();
        wire.observe(
            json!({"jsonrpc":"2.0","id":"session-request","result":{"sessionId":"session_owned"}}),
        )
        .unwrap();
        let requests = wire
            .configure_model("model-request", "exact-model", None, None)
            .unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!wire.ready_for_prompt());
        wire.observe(json!({"jsonrpc":"2.0","id":"model-request","result":{}}))
            .unwrap();
        assert!(wire.ready_for_prompt());
        assert!(wire
            .prompt("prompt-request", "Perform the requested change")
            .is_ok());
    }

    #[test]
    fn grok_unadvertised_uncorrelated_or_malformed_authentication_is_rejected() {
        let mut wire = NativeWire::new(NativeKind::Grok, "session_owned", "turn_owned").unwrap();
        assert!(wire
            .session_request("session-request", "/owned", true)
            .is_err());
        wire.observe(json!({"jsonrpc":"2.0","id":"unsolicited","result":{"authMethods":[{"id":"cached_token"}],"_meta":{"auth_mode":"native_cached"}}})).unwrap();
        assert!(wire.authentication_request("auth-request").is_err());
        let mut wire = NativeWire::new(NativeKind::Grok, "", "turn_owned").unwrap();
        wire.initialize("initialize-request").unwrap();
        wire.observe(json!({"jsonrpc":"2.0","id":"initialize-request","result":{"authMethods":[{"id":"cached_token"}]}})).unwrap();
        wire.authentication_request("auth-request").unwrap();
        let events = wire
            .observe(
                json!({"jsonrpc":"2.0","id":"auth-request","result":{"_meta":{"auth_mode":false}}}),
            )
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            NativeEvent::Error {
                class: ErrorClass::Authentication,
                ..
            }
        )));
        assert!(!wire.ready_for_prompt());
        assert!(wire
            .session_request("session-request", "/owned", false)
            .is_err());
    }

    #[test]
    fn codex_completed_spawn_retains_unsettled_effect_and_fences_unknown_user_input() {
        let mut wire = NativeWire::new(NativeKind::Codex, "thread_owned", "turn_owned").unwrap();
        let events = wire.observe(json!({"method":"item/completed","params":{"threadId":"thread_owned","item":{
            "id":"spawn_owned","type":"collabAgentToolCall","status":"completed","agentsStates":{"child_owned":{"status":"running"}}}}})).unwrap();
        assert!(events.iter().any(|event|matches!(event,NativeEvent::Tool(tool) if tool.tool_id == "spawn_owned" && tool.state == ToolState::Started)));
        assert!(events.iter().any(|event| matches!(
            event,
            NativeEvent::Error {
                class: ErrorClass::Protocol,
                ..
            }
        )));
        let mut wire = NativeWire::new(NativeKind::Codex, "thread_owned", "turn_owned").unwrap();
        let events = wire.observe(json!({"id":"native-dialog","method":"item/tool/requestUserInput","params":{"threadId":"thread_owned","questions":[]}})).unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            NativeEvent::Error {
                class: ErrorClass::Protocol,
                ..
            }
        )));
    }

    #[test]
    fn native_hook_activity_is_effect_evidence_before_the_user_prompt() {
        let mut codex = NativeWire::new(NativeKind::Codex, "thread_owned", "turn_owned").unwrap();
        let events = codex.observe(json!({"method":"hook/started","params":{"threadId":"thread_owned","turnId":null,"run":{"id":"startup","status":"running"}}})).unwrap();
        assert!(events.iter().any(|event|matches!(event,NativeEvent::Tool(tool) if tool.tool_id == "hook_startup" && tool.state == ToolState::Started)));
        let mut claude =
            NativeWire::new(NativeKind::Claude, "session_owned", "turn_owned").unwrap();
        let events = claude.observe(json!({"type":"system","subtype":"hook_started","session_id":"session_owned","hook_id":"startup","hook_event":"SessionStart"})).unwrap();
        assert!(events.iter().any(|event|matches!(event,NativeEvent::Tool(tool) if tool.tool_id == "hook_startup" && tool.state == ToolState::Started)));
        assert!(NativeKind::Claude
            .launch()
            .arguments
            .contains(&"--include-hook-events".into()));
    }

    fn failed_opencode(kind: NativeKind) -> (NativeWire, Vec<Value>) {
        let mut wire = NativeWire::new(kind, "ses_owned", "turn_owned").unwrap();
        wire.configure_model("model-request", "exact-model", Some("exact-provider"), None)
            .unwrap();
        wire.prompt("prompt-request", "Perform the requested change")
            .unwrap();
        wire.observe(json!({"type":"session.status","properties":{"sessionID":"ses_owned","status":{"type":"busy"}}})).unwrap();
        let info = json!({"id":"msg_assistant","sessionID":"ses_owned","parentID":"msg_turn_owned","role":"assistant",
            "modelID":"exact-model","providerID":"exact-provider","time":{"created":10,"completed":11},
            "error":{"name":"APIError","code":"insufficient_quota"}});
        wire.observe(json!({"type":"message.updated","properties":{"info":info}}))
            .unwrap();
        wire.observe(json!({"type":"session.idle","properties":{"sessionID":"ses_owned"}}))
            .unwrap();
        let views = vec![
            json!({"id":"ses_owned","directory":"/owned"}),
            json!({}),
            json!([
                {"info":{"id":"msg_turn_owned","sessionID":"ses_owned","role":"user"},"parts":[]},
                {"info":info,"parts":[]}
            ]),
            json!([]),
            json!([]),
        ];
        (wire, views)
    }

    #[test]
    fn completed_error_without_finish_is_a_failed_boundary_for_both_http_clients() {
        for kind in [NativeKind::Kilo, NativeKind::OpenCode] {
            let (wire, views) = failed_opencode(kind);
            let terminal = wire.terminal().unwrap();
            assert_eq!(terminal.outcome, NativeOutcome::Failed);
            assert_eq!(terminal.error_class, Some(ErrorClass::Quota));
            assert!(wire.validate_idle_snapshots("/owned", &views).is_ok());
        }
    }

    #[test]
    fn idle_status_cannot_reconcile_another_parent_or_pending_question() {
        let (wire, mut views) = failed_opencode(NativeKind::OpenCode);
        views[2][1]["info"]["parentID"] = json!("msg_unrelated");
        assert!(wire.validate_idle_snapshots("/owned", &views).is_err());
        let (wire, mut views) = failed_opencode(NativeKind::OpenCode);
        views[4] = json!([{"id":"question_unresolved","sessionID":"ses_owned"}]);
        assert!(wire.validate_idle_snapshots("/owned", &views).is_err());
    }

    fn failed_kimi() -> (NativeWire, Vec<Value>) {
        let mut wire = NativeWire::new(NativeKind::Kimi, "session_owned", "turn_owned").unwrap();
        wire.subscription_acknowledged = true;
        wire.journal_epoch = Some("owned-epoch".into());
        wire.journal_seq = Some(3);
        wire.configure_model("model-request", "exact-model", None, None)
            .unwrap();
        wire.prompt("prompt-owned", "Perform the requested change")
            .unwrap();
        wire.native_turn = Some("1".into());
        wire.failure = Some(ErrorClass::Quota);
        wire.finish(
            NativeOutcome::Failed,
            json!({"type":"turn.ended","turnId":1,"reason":"failed"}),
            &mut Vec::new(),
        );
        let session = json!({"id":"session_owned","metadata":{"cwd":"/owned"},"agent_config":{"model":"exact-model"},
            "busy":false,"main_turn_active":false,"pending_interaction":"none","last_turn_reason":"failed","last_seq":3});
        let snapshot = json!({"session":session,"epoch":"owned-epoch","as_of_seq":3,"in_flight_turn":null,
            "subagents":[],"pending_approvals":[],"pending_questions":[]});
        (
            wire,
            vec![session, json!({"items":[]}), json!({"items":[]}), snapshot],
        )
    }

    #[test]
    fn kimi_rejects_active_tasks_and_ahead_or_changed_journal_watermarks() {
        let (wire, mut views) = failed_kimi();
        assert!(wire.validate_idle_snapshots("/owned", &views).is_ok());
        views[1] = json!({"items":[{"id":"task_running","session_id":"session_owned","status":"running","kind":"subagent"}]});
        assert!(wire.validate_idle_snapshots("/owned", &views).is_err());
        views[1] = json!({"items":[]});
        views[3]["as_of_seq"] = json!(4);
        assert_eq!(
            wire.idle_snapshot_watermark(&views).unwrap(),
            Some((4, "owned-epoch".into()))
        );
        assert!(wire.validate_idle_snapshots("/owned", &views).is_err());
        views[3]["epoch"] = json!("unrelated-epoch");
        assert!(wire.idle_snapshot_watermark(&views).is_err());
    }

    #[test]
    fn kimi_ended_phase_housekeeping_is_correlated_and_running_phase_is_rejected() {
        let (wire, _) = failed_kimi();
        let event = json!({"type":"agent.status.updated","payload":{"type":"agent.status.updated","phase":{"kind":"ended","turnId":1,"reason":"failed"}}});
        assert!(terminal_housekeeping(
            NativeKind::Kimi,
            &event,
            Some("prompt-owned"),
            wire.terminal()
        ));
        let mut wrong_turn = event.clone();
        wrong_turn["payload"]["phase"]["turnId"] = json!(2);
        assert!(!terminal_housekeeping(
            NativeKind::Kimi,
            &wrong_turn,
            Some("prompt-owned"),
            wire.terminal()
        ));
        let mut running = event;
        running["payload"]["phase"]["kind"] = json!("running");
        assert!(!terminal_housekeeping(
            NativeKind::Kimi,
            &running,
            Some("prompt-owned"),
            wire.terminal()
        ));
    }

    #[test]
    fn decoded_events_keep_owned_bindings_and_terminal_snapshot() {
        let (wire, _) = failed_kimi();
        let text = NativeEvent::Text {
            session_id: wire.session_id().into(),
            turn_id: wire.turn_id().into(),
            text: "observed".into(),
        };
        assert!(wire.validate_event(&text, true).is_ok());
        let foreign = NativeEvent::Text {
            session_id: "other-session".into(),
            turn_id: wire.turn_id().into(),
            text: "foreign".into(),
        };
        assert!(wire.validate_event(&foreign, true).is_err());
        let terminal = wire.terminal().unwrap().clone();
        assert!(wire
            .validate_event(&NativeEvent::Terminal(terminal.clone()), true)
            .is_ok());
        let mut altered = terminal;
        altered.raw = json!({"different":"receipt"});
        assert!(wire
            .validate_event(&NativeEvent::Terminal(altered), true)
            .is_err());
        assert!(wire
            .validate_event(
                &NativeEvent::Accepted {
                    request_id: json!("owned"),
                    raw: json!({"id":"foreign"}),
                },
                true
            )
            .is_err());
    }

    #[test]
    fn http_body_identifiers_are_not_reinterpreted_as_request_ids() {
        let mut wire = NativeWire::new(NativeKind::Kimi, "session_owned", "turn_owned").unwrap();
        wire.remember("request-owned", "prompt").unwrap();
        let body = json!({"id":"native-payload-id","data":{"id":"native-item"}});
        let event = wire
            .accepted_http("request-owned", 200, body.clone())
            .unwrap();
        assert!(matches!(event, NativeEvent::Accepted { request_id, raw }
            if request_id == json!("request-owned") && raw == body));
    }
}
