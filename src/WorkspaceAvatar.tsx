import { useEffect, useState, type CSSProperties } from "react";
import { Bot, Briefcase, Rocket } from "lucide-react";
import { Code, Folder, Globe, Layers, Terminal } from "./icons";
import { api } from "./api";
import { msg } from "./i18n";
import { githubOwner } from "./workspace-logo";
import type { WorkspaceAppearance } from "./workspace-appearance";

export const workspaceIconOptions = [
  { id: "layers", label: msg("Layers"), Icon: Layers },
  { id: "folder", label: msg("Folder"), Icon: Folder },
  { id: "code", label: msg("Code"), Icon: Code },
  { id: "terminal", label: msg("Terminal"), Icon: Terminal },
  { id: "globe", label: msg("Globe"), Icon: Globe },
  { id: "rocket", label: msg("Rocket"), Icon: Rocket },
  { id: "bot", label: msg("Bot"), Icon: Bot },
  { id: "briefcase", label: msg("Briefcase"), Icon: Briefcase },
];

// Resolved once per key for the app lifetime; project favicons refresh on restart.
const autoLogos = new Map<string, Promise<string | null>>();
const pickedLogos = new Map<string, Promise<string | null>>();

function cached(
  cache: Map<string, Promise<string | null>>,
  key: string,
  load: () => Promise<string | null>,
) {
  let value = cache.get(key);
  if (!value) {
    value = load().catch(() => null);
    cache.set(key, value);
  }
  return value;
}

async function loadAuto(project: string) {
  const favicon = await api<string | null>("workspace_favicon", { project });
  if (favicon) return favicon;
  const origin = await api<string | null>("workspace_origin_url", {
    project,
  });
  const owner = origin && githubOwner(origin);
  return owner ? `https://github.com/${owner}.png?size=64` : null;
}

function useLogo(
  cache: Map<string, Promise<string | null>>,
  key: string | undefined,
  load: (key: string) => Promise<string | null>,
) {
  const [resolved, setResolved] = useState<{
    key: string;
    src: string | null;
  }>();
  useEffect(() => {
    if (!key) return;
    let current = true;
    void cached(cache, key, () => load(key)).then(
      (src) => current && setResolved({ key, src }),
    );
    return () => {
      current = false;
    };
  }, [cache, key, load]);
  return key && resolved?.key === key ? resolved.src : null;
}

const loadPicked = (id: string) =>
  api<string | null>("workspace_logo_read", { id });

export default function WorkspaceAvatar({
  appearance,
  project,
  className = "",
}: {
  appearance?: WorkspaceAppearance;
  /** Project folder used to find an automatic logo. */
  project?: string;
  className?: string;
}) {
  const icon = appearance?.icon;
  const Icon = workspaceIconOptions.find((option) => option.id === icon)?.Icon;
  const picked = useLogo(pickedLogos, appearance?.logo, loadPicked);
  const explicit = !!(appearance?.logo || appearance?.image || icon);
  const auto = useLogo(autoLogos, explicit ? undefined : project, loadAuto);
  const src = picked ?? appearance?.image ?? (icon ? null : auto);
  const [failed, setFailed] = useState<string>();
  const color = appearance?.color;
  const rgb = color
    ? [1, 3, 5].map(
        (start) => parseInt(color.slice(start, start + 2), 16) / 255,
      )
    : [];
  const luminance = rgb.reduce(
    (sum, value, index) =>
      sum +
      (value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4) *
        [0.2126, 0.7152, 0.0722][index],
    0,
  );
  return (
    <span
      className={`workspace-avatar ${className}`}
      aria-hidden="true"
      style={
        color
          ? ({
              "--workspace-avatar-color": color,
              "--workspace-avatar-ink":
                luminance > 0.179 ? "#111111" : "#ffffff",
            } as CSSProperties)
          : undefined
      }
    >
      {src && src !== failed ? (
        <img src={src} alt="" onError={() => setFailed(src)} />
      ) : Icon ? (
        <Icon size={19} />
      ) : icon ? (
        <span className="workspace-avatar-emoji">{icon}</span>
      ) : (
        <Folder size={19} />
      )}
    </span>
  );
}
