import "../settings-sections.css";
import "../settings-speech.css";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { Channel } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { api, errorMessage, native } from "../api";
import { formatBytes } from "../android/settings-model";
import { ExternalLink } from "../icons";
import { useT } from "../i18n";
import { formatShortcut } from "../keybindings";
import { useKeybindings } from "../KeybindingsProvider";
import type { SettingsPageId } from "../settings-pages";
import { SettingRow, SettingsPage, SettingsSection } from "../settings-ui";
import { Modal } from "../ui";
import {
  chooseModel,
  chosenModel,
  subscribeChosenModel,
} from "./stt-preferences";

interface ModelView {
  id: string;
  name: string;
  size: number;
  languages: string[];
  license: string;
  repository: string;
  errorPl: number | null;
  errorEn: number | null;
  cpuSecondsPer30s: number | null;
  gpuSecondsPer30s: number | null;
  state: "ready" | "partial" | "missing" | "downloading";
  downloadedBytes: number;
}

interface Progress {
  phase: "downloading" | "verifying";
  bytes: number;
  total: number;
  bytesPerSec: number;
}

export default function SpeechSettingsPage({
  onNavigate,
}: {
  onNavigate: (page: SettingsPageId) => void;
}) {
  const { t, tp, locale } = useT();
  const keybindings = useKeybindings();
  const chosen = useSyncExternalStore(subscribeChosenModel, chosenModel);
  const [available, setAvailable] = useState(false);
  const [models, setModels] = useState<ModelView[]>([]);
  const [progress, setProgress] = useState<{ id: string } & Progress>();
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [deleting, setDeleting] = useState<ModelView>();

  const refresh = useCallback(
    () =>
      api<ModelView[]>("stt_list_models")
        .then(setModels)
        .catch(() => {}),
    [],
  );
  useEffect(() => {
    api<{ available: boolean }>("stt_status")
      .then((status) => {
        setAvailable(status.available);
        if (status.available) void refresh();
      })
      .catch(() => {});
  }, [refresh]);

  const fail = (id: string, reason: unknown) => {
    const message = errorMessage(reason);
    const code = /^([a-z-]+): /.exec(message)?.[1];
    if (code === "cancelled") return;
    setErrors((all) => ({
      ...all,
      [id]:
        code === "network"
          ? t("Download interrupted. Resume to continue.")
          : code === "integrity"
            ? t(
                "The file was damaged in transit and was removed. Download it again.",
              )
            : code === "disk-full"
              ? t("Not enough disk space.")
              : code === "busy"
                ? t("This model is in use. Try again when dictation ends.")
                : message,
    }));
  };
  const clear = (id: string) =>
    setErrors(({ [id]: _removed, ...rest }) => rest);

  const download = async (id: string) => {
    clear(id);
    const channel = new Channel<Progress>();
    channel.onmessage = (next) => setProgress({ id, ...next });
    setProgress({
      id,
      phase: "downloading",
      bytes: 0,
      total: 0,
      bytesPerSec: 0,
    });
    // The list marks this model as downloading from the first poll on.
    void refresh();
    try {
      await api("stt_download_model", { id, progress: channel });
    } catch (reason) {
      fail(id, reason);
    }
    setProgress(undefined);
    void refresh();
  };
  const cancel = (id: string) =>
    api("stt_cancel_download", { id }).catch((reason) => fail(id, reason));
  const remove = async (model: ModelView) => {
    setDeleting(undefined);
    clear(model.id);
    try {
      await api("stt_delete_model", { id: model.id });
      if (chosenModel() === model.id) chooseModel(null);
    } catch (reason) {
      fail(model.id, reason);
    }
    void refresh();
  };

  if (!available) return null;

  const effective =
    models.find((m) => m.id === chosen && m.state === "ready") ??
    models.find((m) => m.state === "ready");
  const recommended =
    locale === "en" ? "parakeet-unified-en-0.6b" : "parakeet-ultra";
  const languages = (list: string[]) =>
    list.length === 1 && list[0] === "en"
      ? t("English")
      : list.includes("pl")
        ? tp(
            list.length - 1,
            "Polish and {count} more language",
            "Polish and {count} more languages",
          )
        : tp(list.length, "{count} language", "{count} languages");
  const openSource = (model: ModelView) => {
    const url = `https://huggingface.co/${model.repository}`;
    if (native) {
      void openUrl(url).catch((reason) => fail(model.id, reason));
    } else {
      window.open(url, "_blank", "noopener,noreferrer");
    }
  };

  return (
    <SettingsPage title={t("Speech")} className="speech-settings-page">
      <SettingsSection
        title={t("Speech model")}
        description={t(
          "Dictation runs locally. Audio never leaves this computer.",
        )}
        grouped={false}
      >
        {models.map((model) => {
          const live = progress?.id === model.id ? progress : undefined;
          const busy = model.state === "downloading" || live !== undefined;
          const total = live?.total || model.size;
          const bytes = live ? live.bytes : model.downloadedBytes;
          const percent = Math.min(100, Math.floor((bytes / total) * 100));
          const facts = [
            languages(model.languages),
            formatBytes(model.size),
            model.errorPl !== null &&
              t("Polish {error}% errors", { error: model.errorPl }),
            model.errorEn !== null &&
              t("English {error}% errors", { error: model.errorEn }),
            model.cpuSecondsPer30s !== null &&
              model.gpuSecondsPer30s !== null &&
              t("~{cpu} s CPU / ~{gpu} s GPU per 30 s of speech", {
                cpu: model.cpuSecondsPer30s,
                gpu: model.gpuSecondsPer30s,
              }),
            model.license,
          ].filter(Boolean);
          return (
            <div key={model.id} className="settings-group speech-model">
              <SettingRow
                stacked
                label={
                  <>
                    {model.name}
                    {model.id === recommended && (
                      <span className="speech-badge">{t("Recommended")}</span>
                    )}
                  </>
                }
                description={
                  <>
                    {facts.join(" · ")}
                    {" · "}
                    <button
                      type="button"
                      className="speech-link"
                      onClick={() => openSource(model)}
                    >
                      {t("Source")}
                      <ExternalLink size={12} aria-hidden="true" />
                    </button>
                  </>
                }
              >
                {busy && (
                  <div className="speech-progress">
                    <progress
                      aria-label={t("Download progress for {title}", {
                        title: model.name,
                      })}
                      max={live ? total || undefined : undefined}
                      value={live && total ? bytes : undefined}
                    />
                    {live && (
                      <span className="speech-meta">
                        {live.phase === "verifying"
                          ? t("Verifying…")
                          : t("{percent}% · {done} of {total} · {speed}/s", {
                              percent,
                              done: formatBytes(bytes),
                              total: formatBytes(total),
                              speed: formatBytes(live.bytesPerSec),
                            })}
                      </span>
                    )}
                  </div>
                )}
                {errors[model.id] && (
                  <p className="speech-error" role="alert">
                    {errors[model.id]}
                  </p>
                )}
                <div className="speech-actions">
                  {busy ? (
                    <button
                      type="button"
                      className="button"
                      onClick={() => void cancel(model.id)}
                    >
                      {t("Cancel")}
                    </button>
                  ) : model.state === "ready" ? (
                    <>
                      <label className="speech-use">
                        <input
                          type="radio"
                          name="speech-model"
                          checked={effective?.id === model.id}
                          onChange={() => chooseModel(model.id)}
                        />
                        {t("Use")}
                      </label>
                      <button
                        type="button"
                        className="button button-danger"
                        onClick={() => setDeleting(model)}
                      >
                        {t("Delete")}
                      </button>
                    </>
                  ) : (
                    <button
                      type="button"
                      className="button"
                      disabled={progress !== undefined}
                      onClick={() => void download(model.id)}
                    >
                      {model.state === "partial" ? t("Resume") : t("Download")}
                    </button>
                  )}
                </div>
              </SettingRow>
            </div>
          );
        })}
      </SettingsSection>

      <SettingsSection title={t("Shortcut")}>
        <SettingRow
          label={t("Dictation shortcut")}
          description={t("Tap to start and stop, or hold to talk.")}
        >
          <kbd>
            {formatShortcut(keybindings.bindings.toggleDictation ?? null) ||
              t("Not set")}
          </kbd>
          <button
            type="button"
            className="button"
            onClick={() => onNavigate("keybinds")}
          >
            {t("Change in Keybinds")}
          </button>
        </SettingRow>
      </SettingsSection>

      {deleting && (
        <Modal
          role="alertdialog"
          tone="danger"
          title={t("Delete {name}?", { name: deleting.name })}
          descriptionId="speech-delete-description"
          onClose={() => setDeleting(undefined)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p id="speech-delete-description">
                {t("You can download it again later.")}
              </p>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                onClick={() => setDeleting(undefined)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-primary button-danger"
                onClick={() => void remove(deleting)}
              >
                {t("Delete")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </SettingsPage>
  );
}
