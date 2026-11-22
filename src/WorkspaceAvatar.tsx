import { useEffect, useState } from "react";
import { Folder } from "./icons";
import { api } from "./api";
import { githubOwner } from "./workspace-logo";
import type { WorkspaceAppearance } from "./workspace-appearance";

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
  const picked = useLogo(pickedLogos, appearance?.logo, loadPicked);
  const explicit = !!(appearance?.logo || appearance?.image);
  const auto = useLogo(autoLogos, explicit ? undefined : project, loadAuto);
  const src = picked ?? appearance?.image ?? auto;
  const [failed, setFailed] = useState<string>();
  return (
    <span className={`workspace-avatar ${className}`} aria-hidden="true">
      {src && src !== failed ? (
        <img src={src} alt="" onError={() => setFailed(src)} />
      ) : (
        <Folder size={19} />
      )}
    </span>
  );
}
