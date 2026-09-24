import type { CSSProperties } from "react";
import type { CliAgent } from "./cli-agents";
import { Terminal } from "./icons";
import agy from "./cli-icons/agy.svg";
import claude from "./cli-icons/claude.svg";
import codex from "./cli-icons/codex.svg";
import copilot from "./cli-icons/copilot.svg";
import cursor from "./cli-icons/cursor.svg";
import gemini from "./cli-icons/gemini.svg";
import grok from "./cli-icons/grok.svg";
import hermes from "./cli-icons/hermes.svg";
import kilo from "./cli-icons/kilo.svg";
import kimi from "./cli-icons/kimi.svg";
import kiro from "./cli-icons/kiro.svg";
import openclaw from "./cli-icons/openclaw.svg";
import opencode from "./cli-icons/opencode.svg";
import qwen from "./cli-icons/qwen.svg";
import vibe from "./cli-icons/vibe.svg";

const icons: Partial<Record<CliAgent, { src: string; monochrome?: boolean }>> =
  {
    agy: { src: agy },
    claude: { src: claude },
    codex: { src: codex, monochrome: true },
    copilot: { src: copilot, monochrome: true },
    cursor: { src: cursor, monochrome: true },
    gemini: { src: gemini },
    grok: { src: grok, monochrome: true },
    hermes: { src: hermes, monochrome: true },
    kilo: { src: kilo, monochrome: true },
    kimi: { src: kimi, monochrome: true },
    kiro: { src: kiro, monochrome: true },
    openclaw: { src: openclaw },
    opencode: { src: opencode, monochrome: true },
    qwen: { src: qwen },
    vibe: { src: vibe },
  };

export function CliAgentIcon({
  cli,
  className = "",
}: {
  cli: CliAgent;
  className?: string;
}) {
  const icon = icons[cli];
  return (
    <span className={`cli-agent-icon ${className}`} aria-hidden="true">
      {!icon ? (
        <Terminal size={24} />
      ) : icon.monochrome ? (
        <span
          className="cli-agent-icon-mask"
          style={{ "--cli-agent-icon": `url("${icon.src}")` } as CSSProperties}
        />
      ) : (
        <img src={icon.src} alt="" width={24} height={24} />
      )}
    </span>
  );
}
