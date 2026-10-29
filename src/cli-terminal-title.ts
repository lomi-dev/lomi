import { cliNames } from "./cli-agents.ts";
import type { CliAgent } from "./cli-agents.ts";

const uuidTitle = /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/i;
const versionTitle = /^v?\d+(?:\.\d+){1,3}(?:[-+][\da-z.-]+)?$/i;
const versionSuffix = /\s+v?\d+(?:\.\d+){1,3}(?:[-+][\da-z.-]+)?$/i;
const executableSuffix = /\.(?:app|bat|cmd|exe)$/i;

function isCliCommandTitle(title: string, cli: CliAgent) {
  const normalized = title.trim().replaceAll("\\", "/");
  const pathParts = normalized.split("/");
  const basenameWithSuffix = pathParts.at(-1)!;
  const isPath = pathParts.length > 1;
  const executablePath =
    !isPath ||
    executableSuffix.test(basenameWithSuffix) ||
    /(?:^|\/)(?:bin|sbin|\.bin|macos)(?:\/|$)/i.test(normalized) ||
    /\.app(?:\/|$)/i.test(normalized);
  if (!executablePath) return false;

  const basename = basenameWithSuffix.replace(executableSuffix, "");
  const withoutVersion = basename.replace(versionSuffix, "");
  const displayName = cliNames[cli].toLowerCase();
  const aliases = new Set([
    cli.toLowerCase(),
    displayName,
    displayName.replace(/[^a-z0-9]+/g, "-"),
    displayName.replace(/[^a-z0-9]/g, ""),
  ]);
  return (
    aliases.has(basename.toLowerCase()) ||
    aliases.has(withoutVersion.toLowerCase())
  );
}

function isPlaceholderTitle(title: string, cli: CliAgent) {
  return (
    uuidTitle.test(title) ||
    versionTitle.test(title) ||
    isCliCommandTitle(title, cli)
  );
}

export function cliTerminalDisplayTitle(
  title: string | null | undefined,
  foregroundProgram: string | null | undefined,
  cli?: CliAgent | null,
): string | null {
  const candidate = title?.trim();
  if (cli) {
    return candidate && !isPlaceholderTitle(candidate, cli)
      ? candidate
      : cliNames[cli];
  }
  return candidate || foregroundProgram?.trim() || null;
}
