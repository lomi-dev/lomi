import { useEffect, useId, useState } from "react";
import { api, errorMessage } from "../../api";
import { tp, useT } from "../../i18n";
import { ChevronLeft, Copy, Plus, RefreshCw, Trash2, X } from "../../icons";
import { SettingRow, SettingsSection } from "../../settings-ui";
import { Modal } from "../../ui";
import { ProviderIcon } from "../../ProviderIcon";
import { cliNames, type CliAgent } from "../../cli-agents";
import { loadCliModels } from "../ProviderModelPicker";
import { applyModelPrefs } from "../providers";
import ModelsSection from "./ModelsSection";
import type {
  ApiKind,
  ChatSettings,
  CliModel,
  Header,
  ModelEntry,
  ProviderEntry,
} from "../types";

/** Shared by the page, its detail pane and the add-provider wizard. */
export interface ProviderContext {
  settings: ChatSettings;
  busy: boolean;
  run: (
    name: string,
    request: Record<string, unknown>,
  ) => Promise<ChatSettings | undefined>;
  refreshStatus: (cli: CliAgent) => Promise<void>;
}

export const presets: Record<ApiKind, { name: string; baseUrl: string }> = {
  openai: { name: "OpenAI", baseUrl: "https://api.openai.com/v1" },
  anthropic: { name: "Anthropic", baseUrl: "https://api.anthropic.com/v1" },
  google: {
    name: "Google",
    baseUrl: "https://generativelanguage.googleapis.com/v1beta/openai",
  },
  "openai-compatible": { name: "Custom", baseUrl: "" },
};

export function useProbe(input: {
  instanceId: string | null;
  kind: ApiKind;
  baseUrl: string;
  headers: Header[];
  apiKey: string;
}) {
  const [state, setState] = useState<{
    busy: boolean;
    message: string;
    ok: boolean;
  }>({ busy: false, message: "", ok: true });
  const probe = async () => {
    setState({ busy: true, message: "", ok: true });
    try {
      const result = await api<{ models: string[] }>("ai_provider_probe", {
        request: {
          instanceId: input.instanceId,
          kind: input.kind,
          baseUrl: input.baseUrl.trim(),
          headers: input.headers,
          apiKey: input.apiKey || null,
        },
      });
      setState({
        busy: false,
        ok: true,
        message: tp(
          result.models.length,
          "Connected · {count} model available",
          "Connected · {count} models available",
        ),
      });
      return result.models;
    } catch (cause) {
      setState({ busy: false, ok: false, message: errorMessage(cause) });
    }
  };
  return { ...state, probe };
}

export function HeadersEditor({
  headers,
  onChange,
}: {
  headers: Header[];
  onChange: (headers: Header[]) => void;
}) {
  const { t } = useT();
  const edit = (index: number, patch: Partial<Header>) =>
    onChange(
      headers.map((item, at) => (at === index ? { ...item, ...patch } : item)),
    );
  return (
    <div className="prov-stack">
      {headers.map((header, index) => (
        <div className="prov-header-row" key={index}>
          <input
            aria-label={t("Header name")}
            placeholder={t("Name")}
            value={header.name}
            onChange={(event) => edit(index, { name: event.target.value })}
          />
          <input
            aria-label={t("Header value")}
            placeholder={t("Value")}
            value={header.value}
            onChange={(event) => edit(index, { value: event.target.value })}
          />
          <button
            type="button"
            className="icon-button"
            title={t("Remove header")}
            aria-label={t("Remove header")}
            onClick={() => onChange(headers.filter((_, at) => at !== index))}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      ))}
      <div className="prov-inline">
        <button
          type="button"
          className="button"
          disabled={headers.length >= 16}
          onClick={() => onChange([...headers, { name: "", value: "" }])}
        >
          <Plus size={14} aria-hidden="true" />
          {t("Add header")}
        </button>
        <small className="prov-hint">
          {t("Headers are stored unencrypted. Never put secrets here.")}
        </small>
      </div>
    </div>
  );
}

export function toggleProvider(
  ctx: ProviderContext,
  entry: ProviderEntry,
  enabled: boolean,
) {
  if (entry.kind === "cli")
    void ctx.run("ai_provider_update", {
      expectedRevision: ctx.settings.revision,
      instanceId: entry.instanceId,
      patch: { type: "cli", label: entry.label, enabled, models: null },
    });
  else if (entry.api)
    void ctx.run("ai_provider_update", {
      expectedRevision: ctx.settings.revision,
      instanceId: entry.instanceId,
      patch: {
        type: "api",
        label: entry.api.label,
        accentColor: entry.api.accentColor,
        enabled,
        baseUrl: entry.api.baseUrl,
        headers: entry.api.headers,
        models: entry.api.models,
      },
    });
}

function DetailHead({
  entry,
  ctx,
  onBack,
}: {
  entry: ProviderEntry;
  ctx: ProviderContext;
  onBack: () => void;
}) {
  const { t } = useT();
  return (
    <>
      <button type="button" className="prov-back" onClick={onBack}>
        <ChevronLeft size={14} aria-hidden="true" />
        {t("Providers")}
      </button>
      <header className="prov-hero">
        <span className="prov-tile-icon prov-tile-icon-large">
          <ProviderIcon kind={entry.kind} cli={entry.cli} />
        </span>
        <div className="prov-hero-text">
          <h2>
            {entry.label}
            {entry.versionLabel && <code>{entry.versionLabel}</code>}
          </h2>
          <span
            className="prov-status"
            data-ok={entry.available || undefined}
            title={entry.unavailableReason ?? undefined}
          >
            {entry.statusLine}
          </span>
        </div>
        <input
          type="checkbox"
          role="switch"
          className="settings-switch"
          aria-label={t("Enable {name}", { name: entry.label })}
          checked={entry.enabled}
          disabled={ctx.busy}
          onChange={(event) => toggleProvider(ctx, entry, event.target.checked)}
        />
      </header>
    </>
  );
}

function RemoveRow({
  title,
  description,
  onConfirm,
  busy,
}: {
  title: string;
  description: string;
  onConfirm: () => Promise<unknown>;
  busy: boolean;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  return (
    <div className="prov-remove">
      <button
        type="button"
        className="button button-danger"
        disabled={busy}
        onClick={() => setOpen(true)}
      >
        <Trash2 size={14} aria-hidden="true" />
        {t("Remove provider")}
      </button>
      {open && (
        <Modal
          title={title}
          tone="danger"
          closeDisabled={busy}
          onClose={() => setOpen(false)}
        >
          <div className="dialog-form dialog-layout">
            <div className="dialog-content">
              <p>{description}</p>
            </div>
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setOpen(false)}
              >
                {t("Cancel")}
              </button>
              <button
                type="button"
                className="button button-danger"
                disabled={busy}
                onClick={() => void onConfirm().then(() => setOpen(false))}
              >
                {t("Remove provider")}
              </button>
            </div>
          </div>
        </Modal>
      )}
    </div>
  );
}

const loginCommands: Partial<Record<CliAgent, string>> = {
  claude: "claude auth login",
  codex: "codex login",
  kimi: "kimi login",
  agy: "agy login",
};

function CliDetail({
  entry,
  ctx,
  onBack,
}: {
  entry: ProviderEntry;
  ctx: ProviderContext;
  onBack: () => void;
}) {
  const { t } = useT();
  const cli = entry.cli!;
  const status = entry.cliStatus;
  const [label, setLabel] = useState(entry.label);
  const [reported, setReported] = useState<CliModel[]>();
  const [checking, setChecking] = useState(false);
  const [copied, setCopied] = useState(false);
  const labelId = useId();
  const loadModels = (refresh = false) => {
    setReported(undefined);
    void loadCliModels(cli, refresh).then(setReported);
  };
  // eslint-disable-next-line react-hooks/exhaustive-deps -- reload per CLI
  useEffect(() => loadModels(), [cli]);
  const models = applyModelPrefs(
    (reported ?? []).map((model) => model.id),
    entry.modelPrefs,
  );
  const save = (change: { label?: string; models?: ModelEntry[] }) =>
    void ctx.run("ai_provider_update", {
      expectedRevision: ctx.settings.revision,
      instanceId: entry.instanceId,
      patch: {
        type: "cli",
        label: change.label ?? entry.label,
        enabled: entry.enabled,
        models: change.models ?? null,
      },
    });
  useEffect(() => setLabel(entry.label), [entry.label]);
  const saveLabel = () => {
    const next = label.trim();
    if (!next) return setLabel(entry.label);
    if (next !== entry.label) save({ label: next });
  };
  const refresh = async () => {
    setChecking(true);
    await ctx.refreshStatus(cli);
    setChecking(false);
  };
  const command = loginCommands[cli] ?? `${cli} login`;
  const mono = (value: string | null | undefined, fallback = t("Unknown")) => (
    <code className="prov-value" title={value || undefined}>
      {value || fallback}
    </code>
  );
  return (
    <>
      <DetailHead entry={entry} ctx={ctx} onBack={onBack} />
      <SettingsSection title={t("General")}>
        <SettingRow label={t("Display name")} htmlFor={labelId}>
          <input
            id={labelId}
            value={label}
            maxLength={64}
            disabled={ctx.busy}
            onChange={(event) => setLabel(event.target.value)}
            onBlur={saveLabel}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
            }}
          />
        </SettingRow>
      </SettingsSection>
      <SettingsSection title={t("Account")}>
        <SettingRow
          label={t("Status")}
          description={
            status?.signedIn === false
              ? t("Run in a terminal, then refresh.")
              : status?.error || undefined
          }
        >
          <span>
            {!status
              ? t("Checking…")
              : status.signedIn
                ? status.plan
                  ? t("Signed in · {plan}", { plan: status.plan })
                  : t("Signed in")
                : status.signedIn === false
                  ? t("Not signed in")
                  : t("Unknown")}
          </span>
          <button
            type="button"
            className="button"
            disabled={checking}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} aria-hidden="true" />
            {t("Refresh")}
          </button>
        </SettingRow>
        {status?.signedIn === false && (
          <SettingRow label={t("Login command")}>
            <code className="prov-value">{command}</code>
            <button
              type="button"
              className="button"
              onClick={() =>
                void navigator.clipboard.writeText(command).then(
                  () => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1500);
                  },
                  () => {},
                )
              }
            >
              <Copy size={14} aria-hidden="true" />
              {copied ? t("Copied") : t("Copy")}
            </button>
          </SettingRow>
        )}
      </SettingsSection>
      {/* The model list matters only once the CLI is signed in. */}
      {status?.signedIn === true && (
        <ModelsSection
          models={models}
          labels={
            new Map((reported ?? []).map((model) => [model.id, model.label]))
          }
          onChange={(next) => save({ models: next })}
          onRefresh={() => loadModels(true)}
          refreshing={!reported}
          disabled={ctx.busy}
        />
      )}
      <SettingsSection title={t("Runtime")}>
        <dl className="prov-facts">
          <dt>CLI</dt>
          <dd>{cliNames[cli]}</dd>
          <dt>{t("Version")}</dt>
          <dd>{mono(status?.version)}</dd>
          <dt>{t("Binary")}</dt>
          <dd>
            {mono(status?.binaryPath, status ? t("Not found") : t("Checking…"))}
          </dd>
        </dl>
      </SettingsSection>
      <RemoveRow
        title={t("Remove provider?")}
        description={t(
          "Remove {name} from Chat AI. The CLI and its login stay installed, and it will not be added again automatically.",
          { name: entry.label },
        )}
        busy={ctx.busy}
        onConfirm={async () => {
          const done = await ctx.run("ai_provider_remove", {
            expectedRevision: ctx.settings.revision,
            instanceId: entry.instanceId,
          });
          if (done) onBack();
        }}
      />
    </>
  );
}

export function ApiDetail({
  entry,
  ctx,
  onBack,
}: {
  entry: ProviderEntry;
  ctx: ProviderContext;
  onBack: () => void;
}) {
  const { t } = useT();
  const provider = entry.api!;
  const [label, setLabel] = useState(provider.label);
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl);
  const [headers, setHeaders] = useState(provider.headers);
  const [models, setModels] = useState(provider.models);
  const [apiKey, setApiKey] = useState("");
  const [replacing, setReplacing] = useState(false);
  const ids = [useId(), useId(), useId()];
  const test = useProbe({
    instanceId: provider.instanceId,
    kind: provider.kind,
    baseUrl,
    headers,
    apiKey,
  });
  const dirty =
    JSON.stringify([label.trim(), baseUrl.trim(), headers, models]) !==
    JSON.stringify([
      provider.label,
      provider.baseUrl,
      provider.headers,
      provider.models,
    ]);
  const setKey = (key: string | null) =>
    ctx
      .run("ai_provider_set_key", {
        instanceId: provider.instanceId,
        apiKey: key,
      })
      .then((done) => {
        if (done) {
          setApiKey("");
          setReplacing(false);
        }
      });
  const save = () =>
    void ctx.run("ai_provider_update", {
      expectedRevision: ctx.settings.revision,
      instanceId: provider.instanceId,
      patch: {
        type: "api",
        label: label.trim(),
        accentColor: provider.accentColor,
        enabled: provider.enabled,
        baseUrl: baseUrl.trim(),
        headers: headers.filter((item) => item.name.trim()),
        models,
      },
    });
  const fetchModels = () =>
    void test.probe().then((found) => {
      if (found) setModels((current) => applyModelPrefs(found, current));
    });
  return (
    <>
      <DetailHead entry={entry} ctx={ctx} onBack={onBack} />
      <SettingsSection title={t("General")}>
        <SettingRow label={t("Display name")} htmlFor={ids[0]}>
          <input
            id={ids[0]}
            value={label}
            maxLength={64}
            onChange={(event) => setLabel(event.target.value)}
          />
        </SettingRow>
      </SettingsSection>
      <SettingsSection title={t("Connection")}>
        <SettingRow label={t("Base URL")} htmlFor={ids[1]} stacked>
          <input
            id={ids[1]}
            className="prov-mono"
            spellCheck={false}
            placeholder="https://api.example.com/v1"
            value={baseUrl}
            onChange={(event) => setBaseUrl(event.target.value)}
          />
        </SettingRow>
        <SettingRow
          label={t("API key")}
          htmlFor={ids[2]}
          description={
            provider.apiKeySet && !replacing
              ? t("Stored in the system key store")
              : t("Write-only. Saved to the system key store.")
          }
          stacked={!provider.apiKeySet || replacing}
        >
          {provider.apiKeySet && !replacing ? (
            <div className="prov-inline">
              <button
                type="button"
                className="button"
                disabled={ctx.busy}
                onClick={() => setReplacing(true)}
              >
                {t("Replace")}
              </button>
              <button
                type="button"
                className="button"
                disabled={ctx.busy}
                onClick={() => void setKey(null)}
              >
                {t("Remove")}
              </button>
            </div>
          ) : (
            <div className="prov-inline">
              <input
                id={ids[2]}
                type="password"
                autoComplete="off"
                spellCheck={false}
                placeholder={
                  provider.kind === "openai-compatible"
                    ? t("Optional")
                    : t("Paste an API key")
                }
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
              />
              <button
                type="button"
                className="button"
                disabled={ctx.busy || !apiKey}
                onClick={() => void setKey(apiKey)}
              >
                {t("Save key")}
              </button>
              {replacing && (
                <button
                  type="button"
                  className="button"
                  onClick={() => {
                    setReplacing(false);
                    setApiKey("");
                  }}
                >
                  {t("Cancel")}
                </button>
              )}
            </div>
          )}
        </SettingRow>
        <SettingRow label={t("Headers")} stacked>
          <HeadersEditor headers={headers} onChange={setHeaders} />
        </SettingRow>
        <SettingRow
          label={t("Test connection")}
          description={test.message || t("Requests the model list.")}
        >
          <button
            type="button"
            className="button"
            disabled={test.busy}
            onClick={() => void test.probe()}
          >
            {test.busy ? t("Testing…") : t("Test connection")}
          </button>
        </SettingRow>
      </SettingsSection>
      <ModelsSection
        models={models}
        onChange={setModels}
        onRefresh={fetchModels}
        refreshing={test.busy}
      />
      {dirty && (
        <div className="prov-savebar">
          <button
            type="button"
            className="button"
            onClick={() => {
              setLabel(provider.label);
              setBaseUrl(provider.baseUrl);
              setHeaders(provider.headers);
              setModels(provider.models);
            }}
          >
            {t("Discard")}
          </button>
          <button
            type="button"
            className="button button-primary"
            disabled={ctx.busy || !label.trim() || !baseUrl.trim()}
            onClick={save}
          >
            {t("Save changes")}
          </button>
        </div>
      )}
      <RemoveRow
        title={t("Remove provider?")}
        description={t(
          "Remove {name} and its stored API key. Existing threads keep their messages.",
          { name: provider.label },
        )}
        busy={ctx.busy}
        onConfirm={async () => {
          const done = await ctx.run("ai_provider_remove", {
            expectedRevision: ctx.settings.revision,
            instanceId: provider.instanceId,
          });
          if (done) onBack();
        }}
      />
    </>
  );
}

export default function ProviderDetail(props: {
  entry: ProviderEntry;
  ctx: ProviderContext;
  onBack: () => void;
}) {
  return (
    <div className="prov-detail">
      {props.entry.kind === "cli" ? (
        <CliDetail key={props.entry.instanceId} {...props} />
      ) : (
        <ApiDetail key={props.entry.instanceId} {...props} />
      )}
    </div>
  );
}
