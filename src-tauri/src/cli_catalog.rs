use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum TitleCli {
    Codex,
    Agy,
    Cursor,
    Claude,
    Gemini,
    Copilot,
    Opencode,
    Openclaw,
    Hermes,
    Pi,
    Aider,
    Goose,
    Cline,
    Kilo,
    Qwen,
    Kiro,
    Droid,
    Openhands,
    Continue,
    Amp,
    Auggie,
    Crush,
    Vibe,
    Kimi,
    Interpreter,
    Grok,
    Junie,
    Deepagents,
    Freebuff,
    Trae,
    Sweagent,
}

impl TitleCli {
    pub(crate) const MCP_CLIENTS: [Self; 15] = [
        Self::Claude,
        Self::Codex,
        Self::Gemini,
        Self::Copilot,
        Self::Cursor,
        Self::Opencode,
        Self::Openclaw,
        Self::Hermes,
        Self::Kilo,
        Self::Qwen,
        Self::Kiro,
        Self::Vibe,
        Self::Kimi,
        Self::Grok,
        Self::Agy,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Agy => "Antigravity CLI",
            Self::Cursor => "Cursor CLI",
            Self::Claude => "Claude Code",
            Self::Gemini => "Gemini CLI",
            Self::Copilot => "GitHub Copilot CLI",
            Self::Opencode => "OpenCode",
            Self::Openclaw => "OpenClaw",
            Self::Hermes => "Hermes Agent",
            Self::Pi => "Pi Coding Agent",
            Self::Aider => "Aider",
            Self::Goose => "Goose",
            Self::Cline => "Cline CLI",
            Self::Kilo => "Kilo Code CLI",
            Self::Qwen => "Qwen Code",
            Self::Kiro => "Kiro CLI",
            Self::Droid => "Factory Droid",
            Self::Openhands => "OpenHands CLI",
            Self::Continue => "Continue CLI",
            Self::Amp => "Amp CLI",
            Self::Auggie => "Auggie",
            Self::Crush => "Crush",
            Self::Vibe => "Mistral Vibe",
            Self::Kimi => "Kimi Code CLI",
            Self::Interpreter => "Open Interpreter",
            Self::Grok => "Grok Build",
            Self::Junie => "Junie CLI",
            Self::Deepagents => "Deep Agents Code",
            Self::Freebuff => "Freebuff CLI",
            Self::Trae => "Trae Agent",
            Self::Sweagent => "SWE-agent",
        }
    }

    pub(crate) fn supports_titles(self) -> bool {
        matches!(
            self,
            Self::Codex | Self::Agy | Self::Cursor | Self::Claude | Self::Gemini | Self::Qwen
        )
    }
}
