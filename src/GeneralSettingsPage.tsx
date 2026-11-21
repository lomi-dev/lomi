import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { errorMessage, getInfo } from "./api";
import { FolderOpen, MessageSquare, Terminal } from "./icons";
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
    label: "CLI first",
    text: "New projects open with a terminal.",
    Icon: Terminal,
  },
  {
    value: "chat",
    label: "Chat first",
    text: "New projects open with an AI chat.",
    Icon: MessageSquare,
  },
] as const;

export default function GeneralSettingsPage() {
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
    setStatus("Workflow saved");
  };

  const chooseParent = async () => {
    setError("");
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        defaultPath: parent ?? (home || undefined),
        title: "Default location for new projects",
      });
      if (typeof picked !== "string") return;
      rememberParent(picked);
      setParent(picked);
      setStatus("Location saved");
    } catch (reason) {
      setError(errorMessage(reason));
    }
  };

  return (
    <SettingsPage title="General" status={status}>
      {error && <SettingsNotice tone="error">{error}</SettingsNotice>}
      <SettingsSection
        title="Workflow"
        description="Choose what Lomi puts in front. Both stay one click away."
        grouped={false}
      >
        <div
          className="general-mode-options"
          role="radiogroup"
          aria-label="Start with"
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
                <strong>{label}</strong>
                <small>{text}</small>
              </span>
              <span className="general-mode-radio" aria-hidden="true" />
            </label>
          ))}
        </div>
      </SettingsSection>

      <SettingsSection title="Projects">
        <SettingRow
          label="Default location"
          description="New and cloned projects are created here. You can still change it per project."
        >
          <button
            type="button"
            className="button general-location"
            title={parent ?? home}
            onClick={() => void chooseParent()}
          >
            <FolderOpen size={14} aria-hidden="true" />
            <span>{parent ?? (home ? `${home} (home)` : "Home folder")}</span>
          </button>
        </SettingRow>
      </SettingsSection>
    </SettingsPage>
  );
}
