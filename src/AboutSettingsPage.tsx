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
import { useT } from "./i18n";

interface AboutInfo {
  platform: string;
  arch: string;
  version: string;
  identifier: string;
}

const repositoryUrl = "https://github.com/lomi-dev/lomi";
const issueUrl = "https://github.com/lomi-dev/lomi/issues";
const websiteUrl = "https://lomi.dev";
const documentationUrl = "https://docs.lomi.dev";

// Original authors of the dictation models (Settings → Speech); CC BY 4.0
// requires crediting them alongside the license.
const modelAuthors: Record<string, string> = {
  parakeet: "NVIDIA",
  canary: "NVIDIA",
  whisper: "OpenAI",
  silero: "Silero Team",
};
const licenseNames: Record<string, string> = {
  "cc-by-4.0": "CC BY 4.0",
  "apache-2.0": "Apache 2.0",
  mit: "MIT",
};

interface SpeechModel {
  id: string;
  name: string;
  license: string;
  repository: string;
}

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
  const { t } = useT();
  const [info, setInfo] = useState<AboutInfo | null>(null);
  const [metadataReady, setMetadataReady] = useState(!native);
  // Absent in builds without dictation: the section stays hidden.
  const [speechModels, setSpeechModels] = useState<SpeechModel[]>([]);
  useEffect(() => {
    if (native)
      void api<SpeechModel[]>("stt_list_models")
        // The voice-activity model is downloaded with the first speech model.
        .then((models) =>
          setSpeechModels([
            ...models,
            {
              id: "silero-vad",
              name: "Silero VAD",
              license: "mit",
              repository: "handy-computer/silero-vad-v6.2-gguf",
            },
          ]),
        )
        .catch(() => {});
  }, []);

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
    ? t("Loading…")
    : native
      ? t("Unavailable")
      : t("Unavailable in browser preview");
  const build = info
    ? `${platformName(info.platform)} · ${info.arch} · v${info.version}`
    : unavailable;
  const bundleId = info?.identifier ?? unavailable;

  return (
    <SettingsPage title={t("About")}>
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      <SettingsSection title={t("Application")}>
        <SettingRow label={t("Version")}>
          <span className="setting-value">{info?.version ?? version}</span>
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
            {t("Check for updates")}
          </button>
        </SettingRow>
        <SettingRow label={t("License")}>
          <span className="setting-value">
            {license === "Apache-2.0" ? "Apache 2.0" : license}
          </span>
        </SettingRow>
      </SettingsSection>
      <SettingsSection title={t("Resources")}>
        <SettingRow label={t("Source code")} description="lomi-dev/lomi">
          <button
            className="button"
            onClick={() => openExternal(repositoryUrl)}
          >
            <Github size={14} aria-hidden="true" />
            {t("View on GitHub")}
          </button>
        </SettingRow>
        <SettingRow label={t("Website")} description="lomi.dev">
          <button className="button" onClick={() => openExternal(websiteUrl)}>
            <Globe size={14} aria-hidden="true" />
            {t("Open website")}
          </button>
        </SettingRow>
        <SettingRow label={t("Documentation")} description="docs.lomi.dev">
          <button
            className="button"
            onClick={() => openExternal(documentationUrl)}
          >
            <ExternalLink size={14} aria-hidden="true" />
            {t("Open documentation")}
          </button>
        </SettingRow>
        <SettingRow
          label={t("Feedback")}
          description={t("Report a bug or suggest an improvement.")}
        >
          <button className="button" onClick={() => openExternal(issueUrl)}>
            <ExternalLink size={14} aria-hidden="true" />
            {t("Report an issue")}
          </button>
        </SettingRow>
      </SettingsSection>
      {speechModels.length > 0 && (
        <SettingsSection
          title={t("Speech models")}
          description={t(
            "Dictation models are downloaded from Hugging Face in the GGUF format converted by Handy.",
          )}
        >
          {speechModels.map((model) => (
            <SettingRow
              key={model.id}
              label={model.name}
              description={[
                modelAuthors[model.id.split("-")[0]],
                licenseNames[model.license] ?? model.license,
              ]
                .filter(Boolean)
                .join(" · ")}
            >
              <button
                className="button"
                onClick={() =>
                  openExternal(`https://huggingface.co/${model.repository}`)
                }
              >
                <ExternalLink size={14} aria-hidden="true" />
                {t("Source")}
              </button>
            </SettingRow>
          ))}
        </SettingsSection>
      )}
      <SettingsSection title={t("Build details")}>
        <SettingRow label={t("Build")}>
          <span className="setting-value">{build}</span>
        </SettingRow>
        <SettingRow label={t("Bundle ID")}>
          <span className="setting-value">{bundleId}</span>
        </SettingRow>
      </SettingsSection>
    </SettingsPage>
  );
}
