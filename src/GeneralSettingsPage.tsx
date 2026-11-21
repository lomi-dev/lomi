import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorMessage, getInfo } from "./api";
import { FolderOpen, MessageSquare, Terminal } from "./icons";
import { locales, msg, setLocale, useT, type Locale } from "./i18n";
import Select from "./Select";
import { setWorkMode, workMode, type WorkMode } from "./onboarding-state";
import { rememberedParent, rememberParent } from "./project-sources";
import {
  SettingRow,
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "./settings-ui";

const MODES = [
  {
    value: "cli",
    label: msg("CLI first"),
    text: msg("New projects open with a terminal."),
    Icon: Terminal,
  },
  {
    value: "chat",
    label: msg("Chat first"),
    text: msg("New projects open with an AI chat."),
    Icon: MessageSquare,
  },
] as const;

export default function GeneralSettingsPage() {
  const { t, locale } = useT();
  const [mode, setMode] = useState(workMode);
  const [parent, setParent] = useState(rememberedParent);
  const [home, setHome] = useState("");
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    let live = true;
    getInfo()
      .then((info) => live && setHome(info.home))
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  const chooseMode = (value: WorkMode) => {
    setWorkMode(value);
    setMode(value);
    setStatus(t("Workflow saved"));
  };

  const chooseParent = async () => {
    setError("");
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: parent ?? (home || undefined),
        title: t("Default location for new projects"),
      });
      if (typeof picked !== "string") return;
      rememberParent(picked);
      setParent(picked);
      setStatus(t("Location saved"));
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <SettingsPage title={t("General")} status={status}>
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}

      <SettingsSection
        title={t("Workflow")}
        description={t(
          "Choose what Lomi puts in front. Both stay one click away.",
        )}
        grouped={false}
      >
        <div
          className="general-mode-options"
          role="radiogroup"
          aria-label={t("Start with")}
        >
          {MODES.map(({ value, label, text, Icon }) => (
            <label key={value} data-mode={value}>
              <input
                type="radio"
                name="work-mode"
                value={value}
                checked={mode === value}
                onChange={() => chooseMode(value)}
              />
              <span className="general-mode-icon" aria-hidden="true">
                <Icon size={18} />
              </span>
              <span className="general-mode-copy">
                <strong>{t(label)}</strong>
                <small>{t(text)}</small>
              </span>
              <span className="general-mode-radio" aria-hidden="true" />
            </label>
          ))}
        </div>
      </SettingsSection>

      <SettingsSection title={t("Projects")}>
        <SettingRow
          label={t("Default location")}
          description={t(
            "New and cloned projects are created here. You can still change it per project.",
          )}
        >
          <button
            type="button"
            className="button general-location"
            title={parent ?? home}
            onClick={() => void chooseParent()}
          >
            <FolderOpen size={14} aria-hidden="true" />
            <span>
              {parent ??
                (home ? t("{home} (home)", { home }) : t("Home folder"))}
            </span>
          </button>
        </SettingRow>
      </SettingsSection>

      <SettingsSection title={t("Language")}>
        <SettingRow
          label={t("Interface language")}
          description={t("Lomi follows your system language on first launch.")}
        >
          <Select
            aria-label={t("Interface language")}
            value={locale}
            onChange={(value) => void setLocale(value as Locale)}
            options={Object.entries(locales).map(([value, label]) => ({
              value,
              label,
            }))}
          />
        </SettingRow>
      </SettingsSection>
    </SettingsPage>
  );
}
