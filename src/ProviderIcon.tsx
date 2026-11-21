import type { CSSProperties } from "react";
import type { CliAgent } from "./cli-agents";
import { CliAgentIcon } from "./CliAgentIcon";
import { Globe } from "./icons";
import type { ProviderKind } from "./ai-chat/types";
import anthropic from "./cli-icons/anthropic.svg";
import openai from "./cli-icons/openai.svg";

const masks = { openai, anthropic } as const;

export function ProviderIcon({
  kind,
  cli,
  className = "",
}: {
  kind: ProviderKind;
  cli: CliAgent | null;
  className?: string;
}) {
  if (kind === "cli" && cli)
    return <CliAgentIcon cli={cli} className={className} />;
  if (kind === "google")
    return <CliAgentIcon cli="gemini" className={className} />;
  return (
    <span className={`cli-agent-icon ${className}`} aria-hidden="true">
      {kind === "openai" || kind === "anthropic" ? (
        <span
          className="cli-agent-icon-mask"
          style={
            { "--cli-agent-icon": `url("${masks[kind]}")` } as CSSProperties
          }
        />
      ) : (
        <Globe size={24} />
      )}
    </span>
  );
}
