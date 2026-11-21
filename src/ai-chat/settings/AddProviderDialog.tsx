import { useId, useState } from "react";
import { useT } from "../../i18n";
import { Modal } from "../../ui";
import { ProviderIcon } from "../../ProviderIcon";
import { applyModelPrefs, slugify } from "../providers";
import type { ApiKind, ProviderEntry } from "../types";
import { presets, useProbe, type ProviderContext } from "./ProviderDetail";

const apiKinds: ApiKind[] = [
  "anthropic",
  "openai",
  "google",
  "openai-compatible",
];

/** Adds an API provider. Local CLIs are detected automatically instead. */
export default function AddProviderDialog({
  ctx,
  entries,
  onClose,
  onAdded,
  error: pageError,
}: {
  ctx: ProviderContext;
  entries: ProviderEntry[];
  onClose: () => void;
  onAdded: (instanceId: string) => void;
  error: string;
}) {
  const { t } = useT();
  const [kind, setKind] = useState<ApiKind>("anthropic");
  const [label, setLabel] = useState(presets.anthropic.name);
  const [labelEdited, setLabelEdited] = useState(false);
  const [baseUrl, setBaseUrl] = useState(presets.anthropic.baseUrl);
  const [apiKey, setApiKey] = useState("");
  const [failed, setFailed] = useState(false);
  const ids = [useId(), useId(), useId()];
  const test = useProbe({
    instanceId: null,
    kind,
    baseUrl,
    headers: [],
    apiKey,
  });
  const busy = ctx.busy || test.busy;
  const keyRequired = kind !== "openai-compatible";
  const ready =
    !!label.trim() && !!baseUrl.trim() && (!keyRequired || !!apiKey);

  const pick = (next: ApiKind) => {
    setKind(next);
    setBaseUrl(presets[next].baseUrl);
    if (!labelEdited) setLabel(presets[next].name);
    setFailed(false);
  };

  const add = async () => {
    // Test first so the provider starts with its model list; a failed test
    // offers "Add anyway" instead of blocking (offline, custom proxies).
    let models: string[] | undefined;
    if (!failed) {
      models = await test.probe();
      if (!models) return setFailed(true);
    }
    const instanceId = uniqueSlug(label, entries);
    const done = await ctx.run("ai_provider_create", {
      expectedRevision: ctx.settings.revision,
      provider: {
        type: "api",
        instanceId,
        kind,
        label: label.trim(),
        accentColor: null,
        baseUrl: baseUrl.trim(),
        headers: [],
        models: applyModelPrefs(models ?? [], []),
      },
    });
    if (!done) return;
    if (apiKey)
      await ctx
        .run("ai_provider_set_key", { instanceId, apiKey })
        .catch(() => {});
    onAdded(instanceId);
    onClose();
  };

  const message = test.busy ? t("Connecting…") : test.message || pageError;
  const isError =
    !test.busy && (failed || !test.ok || (!!pageError && !test.message));
  return (
    <Modal
      title={t("Add API provider")}
      className="prov-add-dialog"
      closeDisabled={busy}
      onClose={onClose}
    >
      <form
        className="dialog-form dialog-layout"
        onSubmit={(event) => {
          event.preventDefault();
          if (ready && !busy) void add();
        }}
      >
        <div className="dialog-content prov-add-body">
          <div
            className="prov-kinds"
            role="radiogroup"
            aria-label={t("Service")}
          >
            {apiKinds.map((item) => (
              <button
                key={item}
                type="button"
                role="radio"
                aria-checked={kind === item}
                className="prov-kind"
                onClick={() => pick(item)}
              >
                <ProviderIcon kind={item} cli={null} />
                {item === "google" ? "Google AI Studio" : presets[item].name}
              </button>
            ))}
          </div>
          <label htmlFor={ids[0]}>{t("Name")}</label>
          <input
            id={ids[0]}
            maxLength={64}
            value={label}
            onChange={(event) => {
              setLabel(event.target.value);
              setLabelEdited(true);
            }}
          />
          <label htmlFor={ids[1]}>{t("Base URL")}</label>
          <input
            id={ids[1]}
            className="prov-mono"
            spellCheck={false}
            placeholder="https://api.example.com/v1"
            value={baseUrl}
            onChange={(event) => {
              setBaseUrl(event.target.value);
              setFailed(false);
            }}
          />
          <label htmlFor={ids[2]}>{t("API key")}</label>
          <input
            id={ids[2]}
            type="password"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            placeholder={keyRequired ? t("Paste an API key") : t("Optional")}
            value={apiKey}
            onChange={(event) => {
              setApiKey(event.target.value);
              setFailed(false);
            }}
          />
          <small className="settings-help">
            {t(
              "Stored in the system key store. Models are fetched automatically.",
            )}
          </small>
        </div>
        <div className="dialog-actions">
          <span
            className={`prov-add-status${isError ? " prov-error" : ""}`}
            role="status"
          >
            {message}
          </span>
          <button
            type="button"
            className="button"
            disabled={busy}
            onClick={onClose}
          >
            {t("Cancel")}
          </button>
          <button
            type="submit"
            className="button button-primary"
            disabled={!ready || busy}
          >
            {failed ? t("Add anyway") : t("Add provider")}
          </button>
        </div>
      </form>
    </Modal>
  );
}

function uniqueSlug(name: string, entries: ProviderEntry[]) {
  const base = slugify(name) || "provider";
  const used = new Set(entries.map((entry) => entry.instanceId));
  for (let n = 1; ; n++) {
    const candidate = n === 1 ? base : `${base.slice(0, 45)}-${n}`;
    if (!used.has(candidate)) return candidate;
  }
}
