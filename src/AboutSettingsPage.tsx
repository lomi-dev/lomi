import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ExternalLink, Github, Globe } from "./icons";
import { api, errorMessage, native } from "./api";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";
import { license, version } from "../package.json";

interface AboutInfo {
  platform: string;
  arch: string;
  version: string;
  identifier: string;
}

const repositoryUrl = "https://github.com/lomi-dev/lomi";
const issueUrl = "https://github.com/lomi-dev/lomi/issues";
const websiteUrl = "https://lomi.dev";

function platformName(platform: string) {
  switch (platform) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return platform;
  }
}

export default function AboutSettingsPage({
  error,
  onError,
  onClearError,
}: {
  error: string;
  onError: (error: string) => void;
  onClearError: () => void;
}) {
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [metadataReady, setMetadataReady] = useState(!native);

  useEffect(() => {
    if (!native) return;
    let current = true;
    void api<AboutInfo>("about_info")
      .then((result) => {
        if (!current) return;
        setInfo(result);
        setMetadataReady(true);
      })
      .catch((reason) => {
        if (!current) return;
        setMetadataReady(true);
        onError(errorMessage(reason));
      });
    return () => {
      current = false;
    };
  }, [onError]);

  const openExternal = (url: string) => {
    onClearError();
    if (native) {
      void openUrl(url).catch((reason) => onError(errorMessage(reason)));
    } else {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  };

  const unavailable = !metadataReady
    ? "Loading…"
    : native
      ? "Unavailable"
      : "Unavailable in browser preview";
  const build = info
    ? `${platformName(info.platform)} · ${info.arch} · v${info.version}`
    : unavailable;
  const bundleId = info?.identifier ?? unavailable;

  return (
    <SettingsPage
      title="About"
      description={`Lomi ${info?.version ?? version}`}
      actions={
        <button
          className="button"
          disabled={!native}
          onClick={() => {
            onClearError();
            void api("request_update_check").catch((reason) =>
              onError(errorMessage(reason)),
            );
          }}
        >
          Check for updates
        </button>
      }
    >
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      <SettingsSection title="Application">
        <SettingRow label="Build">
          <span className="setting-value">{build}</span>
        </SettingRow>
        <SettingRow label="Bundle ID">
          <span className="setting-value">{bundleId}</span>
        </SettingRow>
        <SettingRow label="License">
          <span className="setting-value">
            {license === "Apache-2.0" ? "Apache 2.0" : license}
          </span>
        </SettingRow>
      </SettingsSection>
      <SettingsSection title="Resources">
        <SettingRow label="Source code" description="lomi-dev/lomi">
          <button
            className="button"
            onClick={() => openExternal(repositoryUrl)}
          >
            <Github size={14} aria-hidden="true" />
            View on GitHub
          </button>
        </SettingRow>
        <SettingRow label="Website" description="lomi.dev">
          <button className="button" onClick={() => openExternal(websiteUrl)}>
            <Globe size={14} aria-hidden="true" />
            Open website
          </button>
        </SettingRow>
        <SettingRow
          label="Feedback"
          description="Report a bug or suggest an improvement."
        >
          <button className="button" onClick={() => openExternal(issueUrl)}>
            <ExternalLink size={14} aria-hidden="true" />
            Report an issue
          </button>
        </SettingRow>
      </SettingsSection>
    </SettingsPage>
  );
}
