import { useEffect, useId, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, errorMessage, native } from "../api";
import { newId } from "../model";
import { DisclosureSummary, Modal } from "../ui";
import ContextMenu from "../ContextMenu";
import Select from "../Select";
import {
  Check,
  ChevronRight,
  ArrowLeft,
  MessageSquare,
  Plus,
  ShieldCheck,
  ExternalLink,
  RefreshCw,
  Search,
  Eye,
  EyeOff,
  Trash2,
} from "../icons";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  providerIds,
  providerPresets,
  validModelId,
  type Provider,
} from "./provider-presets";
import capabilities from "./model-capabilities.json";
import { ProviderIcon } from "./ProviderIcon";
import { ChatDefaults } from "./ChatDefaults";
import { ModelSelect } from "./ModelSelect";
import { useProviderModels } from "./useProviderModels";
import { providers, suggestedModel, modelOptions } from "./models";
import { defaultConfig } from "./types";
import type { Connection, Preferences } from "./types";
import "./chat.css";
export default function ChatSettingsPage() {
  const keyStorageId = useId();
  const [data, setData] = useState<Preferences>();
  const [choosingProvider, setChoosingProvider] = useState(false);
  const [selected, setSelected] = useState("");
  const [search, setSearch] = useState("");
  const [addingModel, setAddingModel] = useState(false);
  const [newModel, setNewModel] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [loading, setLoading] = useState(true);
  const [editingModel, setEditingModel] = useState("");
  const [makeDefault, setMakeDefault] = useState(false);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const [recovering, setRecovering] = useState(false);
  const [busy, setBusy] = useState(false);
  const [editingRevision, setEditingRevision] = useState(0);
  const [editing, setEditing] = useState<Connection | null>(null);
  const setupTrigger = useRef<HTMLButtonElement | null>(null);
  const setupOpen = choosingProvider || !!editing;
  const [key, setKey] = useState("");
  const [removingKey, setRemovingKey] = useState<Connection | null>(null);
  const [deleting, setDeleting] = useState<Connection | null>(null);
  const [providerMenu, setProviderMenu] = useState<{
    id: string;
    x: number;
    y: number;
  }>();
  const providerMenuTrigger = useRef<HTMLButtonElement | null>(null);
  const [testModel, setTestModel] = useState<Record<string, string>>({});
  useEffect(() => {
    // WebKit can lose the native return target when dialog contents change.
    if (!setupOpen) {
      setupTrigger.current?.focus();
      setupTrigger.current = null;
    }
  }, [setupOpen]);
  const originalConnection = data?.connections.find(
    (c) => c.id === editing?.id,
  );
  const catalog = useProviderModels(
    editing && !originalConnection ? editing.provider : undefined,
    key,
  );
  const needsKey =
    !editing?.secretId ||
    originalConnection?.provider !== editing?.provider ||
    originalConnection?.secretMode !== editing?.secretMode;
  const reload = async () =>
    setData(await api<Preferences>("chat_preferences"));
  useEffect(() => {
    void reload()
      .catch((e) => setError(errorMessage(e)))
      .finally(() => setLoading(false));
    const stop = native
      ? listen("chat-preferences-changed", () => {
          void reload().catch((e) => setError(errorMessage(e)));
        })
      : undefined;
    return () => {
      void stop?.then((fn) => fn());
    };
  }, []);
  const run = async (action: () => Promise<void>) => {
    if (busy) return false;
    setBusy(true);
    setError("");
    setStatus("");
    try {
      await action();
      return true;
    } catch (error) {
      setError(errorMessage(error));
      return false;
    } finally {
      setBusy(false);
    }
  };
  const save = async (
    next: Preferences,
    connection?: string,
    newKey?: string,
  ) => {
    const saved = await api<Preferences>("chat_preferences_save", {
      data: next,
      expected: next.revision,
      keyConnection: connection ?? null,
      newKey: newKey || null,
    });
    setData(saved);
    setStatus("Saved");
  };
  const add = (provider: Provider) => {
    setChoosingProvider(false);
    setEditingRevision(data?.revision ?? 0);
    setShowKey(false);
    setKey("");
    setError("");
    setEditingModel("");
    setMakeDefault(
      !data?.defaults.model ||
        !data.connections.some(
          (connection) =>
            connection.id === data.defaults.connectionId &&
            connection.enabled &&
            connection.secretId,
        ),
    );
    setEditing({
      id: newId(),
      name: "",
      provider,
      enabled: true,
      credentialRevision: 0,
      secretMode: "system",
      secretId: null,
      models: [],
      testedModel: null,
      testStatus: null,
    });
  };
  const connection = data?.connections.find((c) => c.id === selected);
  const menuConnection = data?.connections.find(
    (c) => c.id === providerMenu?.id,
  );
  const provider = connection?.provider ?? "openai";
  const preset = providerPresets[provider];
  const models = modelOptions(connection ?? { provider, models: [] });
  if (
    connection &&
    data?.defaults.connectionId === connection.id &&
    data.defaults.model &&
    !models.includes(data.defaults.model)
  )
    models.unshift(data.defaults.model);
  const openEdit = (value: Connection, trigger: HTMLButtonElement) => {
    setupTrigger.current = trigger;
    setEditingRevision(data?.revision ?? 0);
    setEditing(value);
    setShowKey(false);
    setKey("");
    setError("");
    setMakeDefault(data?.defaults.connectionId === value.id);
    setEditingModel(
      data?.defaults.connectionId === value.id
        ? data.defaults.model
        : suggestedModel(value),
    );
  };
  const choose = (value: string) => {
    setSelected(selected === value ? "" : value);
    setSearch("");
    setError("");
    setStatus("");
  };
  const closeSetup = () => {
    if (busy) return;
    setChoosingProvider(false);
    setEditing(null);
    setKey("");
    setShowKey(false);
    setError("");
  };
  const showProviderMenu = (
    id: string,
    trigger: HTMLButtonElement,
    x?: number,
    y?: number,
  ) => {
    if (busy) return;
    providerMenuTrigger.current = trigger;
    const bounds = trigger.getBoundingClientRect();
    setProviderMenu({ id, x: x ?? bounds.left, y: y ?? bounds.bottom });
  };
  const confirmRemoval = (value: Connection) => {
    setError("");
    setStatus("");
    setDeleting(value);
  };
  const providerDetail = connection && data && (
    <section
      className="chat-provider-detail"
      aria-label={`${preset.name} configuration`}
    >
      <div className="chat-provider-heading">
        <span>Available in chat</span>
        <label className="chat-provider-toggle">
          <input
            type="checkbox"
            role="switch"
            aria-label={`Enable ${connection.name}`}
            checked={connection.enabled}
            disabled={busy}
            onChange={(event) => {
              const enabled = event.target.checked;
              void run(() =>
                save({
                  ...data,
                  connections: data.connections.map((c) =>
                    c.id === connection.id ? { ...c, enabled } : c,
                  ),
                }),
              );
            }}
          />
          <span aria-hidden="true" />
        </label>
      </div>
      <div className="chat-key-heading">
        <span>API key</span>
        <button
          className="chat-text-button"
          type="button"
          onClick={() =>
            void openUrl(preset.keyURL).catch((e) => setError(errorMessage(e)))
          }
        >
          Get API key <ExternalLink size={12} />
        </button>
      </div>
      <div className="chat-key-summary">
        <ShieldCheck size={16} aria-hidden="true" />
        <span>
          {connection?.secretId
            ? connection.secretMode === "session"
              ? "Session-only key · re-enter after restarting"
              : "Key saved in system credential store"
            : "Add a key to start using this provider"}
        </span>
        <button
          className="button"
          disabled={busy}
          onClick={(event) => openEdit(connection, event.currentTarget)}
        >
          {connection.secretId ? "Edit key" : "Add key"}
        </button>
      </div>
      <details className="chat-models-section">
        <DisclosureSummary>
          Models{" "}
          <span className="chat-model-count">{models.length} available</span>
        </DisclosureSummary>
        <div className="chat-model-heading">
          <div>
            <button
              className="button"
              disabled={busy || !connection?.enabled || !connection.secretId}
              onClick={() =>
                connection &&
                void run(async () => {
                  const result = await api<{
                    status: string;
                    result?: { code?: string };
                  }>("chat_connection_action", {
                    connectionId: connection.id,
                    model: "",
                    operation: "list-models",
                  });
                  if (result.status !== "completed")
                    throw new Error(
                      `Could not refresh models: ${result.result?.code ?? result.status}. Your previous models are still available.`,
                    );
                  await reload();
                  setStatus("Model catalog updated");
                })
              }
            >
              <RefreshCw size={13} /> Refresh models
            </button>
            <button
              className="button"
              disabled={busy || !connection || connection.models.length >= 1000}
              onClick={() => {
                setNewModel("");
                setError("");
                setAddingModel(true);
              }}
            >
              <Plus size={13} /> Add model
            </button>
          </div>
        </div>
        <p className="chat-field-help">
          Choose a default for new conversations, or refresh to find more
          models.
        </p>
        {models.length > 6 && (
          <label className="chat-model-search">
            <Search size={14} aria-hidden="true" />
            <input
              type="search"
              aria-label="Search models"
              placeholder="Search models…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
        )}
        <ul className="chat-provider-models" aria-label="Provider models">
          {models
            .filter((id) => id.toLowerCase().includes(search.toLowerCase()))
            .map((id) => {
              const isDefault =
                !!connection &&
                data.defaults.connectionId === connection.id &&
                data.defaults.model === id;
              const vision = capabilities.models.some(
                (m) => m.provider === provider && m.id === id && m.images,
              );
              return (
                <li key={id}>
                  <div>
                    <code>{id}</code>
                    {vision && <span className="chat-model-badge">Vision</span>}
                  </div>
                  <button
                    className={`chat-model-default ${isDefault ? "is-default" : ""}`}
                    type="button"
                    disabled={
                      busy ||
                      !connection?.enabled ||
                      !connection.secretId ||
                      isDefault
                    }
                    aria-label={
                      isDefault
                        ? `${id} is the default model`
                        : `Use ${id} by default`
                    }
                    onClick={() =>
                      connection &&
                      void run(() =>
                        save({
                          ...data,
                          defaults: {
                            ...data.defaults,
                            connectionId: connection.id,
                            model: id,
                            temperature: null,
                            configured: true,
                          },
                        }),
                      )
                    }
                  >
                    {isDefault ? (
                      <>
                        <Check size={13} /> Default
                      </>
                    ) : (
                      "Use by default"
                    )}
                  </button>
                </li>
              );
            })}
          {!models.some((id) =>
            id.toLowerCase().includes(search.toLowerCase()),
          ) && (
            <li className="chat-model-empty">No models match “{search}”.</li>
          )}
        </ul>
      </details>
      {connection && (
        <details className="chat-advanced chat-provider-tools">
          <DisclosureSummary>Connection tools</DisclosureSummary>
          <dl className="chat-provider-endpoint">
            <div>
              <dt>Base URL</dt>
              <dd>{preset.baseURL}</dd>
            </div>
            <div>
              <dt>API format</dt>
              <dd>{preset.format}</dd>
            </div>
          </dl>
          <ModelSelect
            connection={connection}
            value={
              testModel[connection.id] ??
              (data.defaults.connectionId === connection.id
                ? data.defaults.model
                : suggestedModel(connection))
            }
            onChange={(model) =>
              setTestModel({ ...testModel, [connection.id]: model })
            }
          />
          <button
            className="button"
            disabled={
              busy ||
              !connection.enabled ||
              !connection.secretId ||
              !validModelId(
                testModel[connection.id] ??
                  (data.defaults.connectionId === connection.id
                    ? data.defaults.model
                    : suggestedModel(connection)),
              )
            }
            onClick={() =>
              void run(async () => {
                const result = await api<{
                  status: string;
                  result?: { code?: string };
                }>("chat_connection_action", {
                  connectionId: connection.id,
                  model:
                    testModel[connection.id] ??
                    (data.defaults.connectionId === connection.id
                      ? data.defaults.model
                      : suggestedModel(connection)),
                  operation: "test-connection",
                });
                await reload();
                if (result.status !== "completed")
                  throw new Error(
                    `Connection test failed: ${result.result?.code ?? result.status}`,
                  );
                setStatus("Connection test succeeded");
              })
            }
          >
            Test connection
          </button>
          <p className="chat-field-help">
            Sends a short prompt with a 32-token output limit. May incur
            provider charges. No project content is sent.
          </p>
          {connection.testStatus && (
            <p>
              Last test: {connection.testStatus} · {connection.testedModel}
            </p>
          )}
          <div className="chat-connection-danger">
            {connection.secretId && (
              <button
                className="button"
                disabled={busy}
                onClick={() => setRemovingKey(connection)}
              >
                Remove key
              </button>
            )}
            <button
              className="button"
              disabled={busy}
              onClick={() => confirmRemoval(connection)}
            >
              Remove provider
            </button>
          </div>
        </details>
      )}
    </section>
  );
  return (
    <main className="keybindings-page chat-settings">
      <header className="settings-page-heading">
        <div>
          <h1>Chat AI</h1>
          <p>Connect your AI providers and set up your conversations.</p>
        </div>
        {data && (
          <button
            className="button"
            disabled={busy}
            onClick={(event) => {
              setupTrigger.current = event.currentTarget;
              setError("");
              setStatus("");
              setChoosingProvider(true);
            }}
          >
            <Plus size={14} /> Add provider
          </button>
        )}
      </header>
      {error &&
        !editing &&
        !addingModel &&
        !removingKey &&
        !deleting &&
        !recovering && (
          <p className="chat-error" role="alert">
            {error}
          </p>
        )}
      {(busy || status) && (
        <p className="chat-settings-status" role="status">
          {busy ? "Working…" : status}
        </p>
      )}
      {loading ? (
        <p role="status">Loading providers…</p>
      ) : !data ? (
        <section>
          <p>
            Chat AI settings are unavailable. Existing preferences have been
            preserved.
          </p>
          <button
            className="button"
            disabled={busy}
            onClick={() =>
              void run(async () => {
                await api("chat_recover", { target: "settings", reset: false });
                await reload();
              })
            }
          >
            Retry opening settings
          </button>
          <button
            className="button"
            disabled={busy}
            onClick={() => setRecovering(true)}
          >
            Recover settings…
          </button>
        </section>
      ) : (
        <>
          <section
            className="chat-providers"
            aria-labelledby="chat-providers-title"
          >
            <h2 id="chat-providers-title" className="chat-section-title">
              Providers <span>{data.connections.length}</span>
            </h2>
            {data.connections.length ? (
              <ul className="chat-provider-list" aria-label="Your providers">
                {data.connections.map((item) => (
                  <li key={item.id}>
                    <button
                      type="button"
                      className="chat-provider-item"
                      aria-expanded={connection?.id === item.id}
                      aria-controls={`provider-${item.id}`}
                      aria-haspopup="menu"
                      onClick={() => choose(item.id)}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        showProviderMenu(
                          item.id,
                          event.currentTarget,
                          event.clientX,
                          event.clientY,
                        );
                      }}
                      onKeyDown={(event) => {
                        if (
                          event.key === "ContextMenu" ||
                          (event.shiftKey && event.key === "F10")
                        ) {
                          event.preventDefault();
                          showProviderMenu(item.id, event.currentTarget);
                        }
                      }}
                      disabled={busy}
                    >
                      <ProviderIcon provider={item.provider} />
                      <span className="chat-provider-name">
                        <strong>{item.name}</strong>
                        <small>
                          {item.name !== providers[item.provider] &&
                            `${providers[item.provider]} · `}
                          {!item.enabled
                            ? "Disabled"
                            : !item.secretId
                              ? "Needs API key"
                              : item.secretMode === "session"
                                ? "Session-only key"
                                : "Key saved"}
                        </small>
                      </span>
                      {data.defaults.connectionId === item.id && (
                        <span className="chat-provider-default">Default</span>
                      )}
                      <ChevronRight
                        className="chat-provider-chevron"
                        size={15}
                        aria-hidden="true"
                      />
                    </button>
                    <div
                      id={`provider-${item.id}`}
                      hidden={connection?.id !== item.id}
                    >
                      {connection?.id === item.id && providerDetail}
                    </div>
                  </li>
                ))}
              </ul>
            ) : (
              <div className="chat-providers-empty">
                <MessageSquare size={24} aria-hidden="true" />
                <h3>Connect your first provider</h3>
                <p>Add a provider with your API key to start chatting.</p>
              </div>
            )}
          </section>
          {!!data.connections.length && (
            <ChatDefaults
              saved={data}
              busy={busy}
              onSave={(next) => run(() => save(next))}
            />
          )}
          <p className="chat-storage-note">
            <ShieldCheck size={15} /> Keys are kept in your system credential
            store by default. Conversation history stays on this device.
          </p>
        </>
      )}
      {providerMenu && menuConnection && (
        <ContextMenu
          x={providerMenu.x}
          y={providerMenu.y}
          label="Provider actions"
          actions={[
            {
              label: "Remove provider…",
              icon: <Trash2 size={14} />,
              danger: true,
              disabled: busy,
              run: () => confirmRemoval(menuConnection),
            },
          ]}
          onClose={() => {
            setProviderMenu(undefined);
            providerMenuTrigger.current?.focus({ preventScroll: true });
          }}
        />
      )}
      {(choosingProvider || editing) && data && (
        <Modal
          className="chat-dialog chat-provider-dialog"
          title={
            editing
              ? originalConnection
                ? `Edit ${providers[editing.provider]}`
                : `Connect ${providers[editing.provider]}`
              : "Add provider"
          }
          onClose={closeSetup}
        >
          {choosingProvider ? (
            <div className="dialog-form chat-provider-picker">
              <p>Choose a provider to connect with your API key.</p>
              <div className="chat-provider-choices">
                {providerIds.map((id) => (
                  <button
                    key={id}
                    autoFocus={id === providerIds[0]}
                    className="chat-provider-choice"
                    onClick={() => add(id)}
                  >
                    <ProviderIcon provider={id} />
                    <span>{providers[id]}</span>
                    <ChevronRight size={14} aria-hidden="true" />
                  </button>
                ))}
              </div>
              <p className="chat-field-help">
                API usage is billed by your provider.
              </p>
            </div>
          ) : (
            editing && (
              <form
                className="dialog-form chat-config"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (
                    !originalConnection &&
                    (catalog.status !== "ready" ||
                      (makeDefault && !catalog.models.includes(editingModel)))
                  )
                    return;
                  void run(async () => {
                    if (data.revision !== editingRevision)
                      throw new Error(
                        "Settings changed while you were editing. Close and reopen this form to use the latest settings.",
                      );
                    const connections = data.connections.some(
                      (c) => c.id === editing.id,
                    )
                      ? data.connections.map((c) =>
                          c.id === editing.id
                            ? {
                                ...editing,
                                name:
                                  editing.name.trim() ||
                                  providers[editing.provider],
                              }
                            : c,
                        )
                      : [
                          ...data.connections,
                          {
                            ...editing,
                            models: catalog.models,
                            name:
                              editing.name.trim() ||
                              providers[editing.provider],
                          },
                        ];
                    await save(
                      {
                        ...data,
                        connections,
                        defaults: makeDefault
                          ? {
                              ...data.defaults,
                              connectionId: editing.id,
                              model: editingModel.trim(),
                              temperature: null,
                              configured: true,
                            }
                          : data.defaults,
                      },
                      key.trim() ? editing.id : undefined,
                      key.trim(),
                    );
                    setSelected(editing.id);
                    setSearch("");
                    setEditing(null);
                    setKey("");
                    setStatus(
                      makeDefault
                        ? "Provider saved. You’re ready to chat."
                        : "Provider saved.",
                    );
                  });
                }}
              >
                <div className="chat-setup-provider">
                  <ProviderIcon provider={editing.provider} />
                  <span>{providers[editing.provider]}</span>
                  {!originalConnection && (
                    <button
                      type="button"
                      className="chat-text-button"
                      disabled={busy}
                      onClick={() => {
                        setEditing(null);
                        setKey("");
                        setShowKey(false);
                        setError("");
                        setChoosingProvider(true);
                      }}
                    >
                      <ArrowLeft size={13} /> Change provider
                    </button>
                  )}
                </div>
                <div className="chat-key-heading">
                  <label htmlFor="chat-provider-key">
                    {needsKey ? "API key" : "Replacement API key"}
                  </label>
                  <button
                    type="button"
                    className="chat-text-button"
                    onClick={() =>
                      void openUrl(
                        providerPresets[editing.provider].keyURL,
                      ).catch((e) => setError(errorMessage(e)))
                    }
                  >
                    Get API key <ExternalLink size={12} />
                  </button>
                </div>
                <div className="chat-key-input">
                  <input
                    id="chat-provider-key"
                    autoFocus
                    type={showKey ? "text" : "password"}
                    autoComplete="off"
                    spellCheck={false}
                    aria-describedby="chat-provider-key-help"
                    value={key}
                    onChange={(e) => {
                      setKey(e.target.value);
                      if (!originalConnection) setEditingModel("");
                    }}
                    required={needsKey}
                    disabled={busy}
                    placeholder={
                      needsKey
                        ? "Paste your API key"
                        : "Leave empty to keep your current key"
                    }
                  />
                  <button
                    className="chat-key-reveal"
                    type="button"
                    aria-label={showKey ? "Hide key" : "Show key"}
                    aria-pressed={showKey}
                    onClick={() => setShowKey(!showKey)}
                  >
                    {showKey ? <EyeOff size={15} /> : <Eye size={15} />}
                  </button>
                </div>
                <p className="chat-field-help" id="chat-provider-key-help">
                  {needsKey
                    ? "API usage is billed separately by your provider."
                    : "Your saved key is never displayed here."}
                </p>
                <label className="chat-checkbox">
                  <input
                    type="checkbox"
                    checked={makeDefault}
                    disabled={data.defaults.connectionId === editing.id}
                    onChange={(event) => setMakeDefault(event.target.checked)}
                  />
                  Use for new conversations
                </label>
                {makeDefault && (
                  <ModelSelect
                    key={editing.provider}
                    connection={editing}
                    value={editingModel}
                    onChange={setEditingModel}
                    availableModels={
                      !originalConnection ? catalog.models : undefined
                    }
                    allowCustom={!!originalConnection}
                    disabled={
                      busy ||
                      (!originalConnection && catalog.status !== "ready")
                    }
                    placeholder={
                      originalConnection || catalog.status === "ready"
                        ? "Choose a model"
                        : catalog.status === "idle"
                          ? "Enter an API key first"
                          : catalog.status === "loading"
                            ? "Loading models…"
                            : "Models unavailable"
                    }
                  />
                )}
                {!originalConnection && catalog.status === "loading" && (
                  <p className="chat-field-help" role="status">
                    Loading available models…
                  </p>
                )}
                {!originalConnection && catalog.error && (
                  <div className="chat-catalog-error">
                    <p className="chat-field-help" role="alert">
                      {catalog.error}
                    </p>
                    <button
                      type="button"
                      className="chat-text-button"
                      onClick={catalog.retry}
                    >
                      <RefreshCw size={13} aria-hidden="true" /> Retry
                    </button>
                  </div>
                )}
                <p className="chat-storage-note">
                  <ShieldCheck size={14} />{" "}
                  {editing.secretMode === "system"
                    ? "Your key will be saved in your system credential store."
                    : "Key kept in memory until Lomi closes."}
                </p>
                <details className="chat-advanced">
                  <DisclosureSummary>Advanced options</DisclosureSummary>
                  <label>
                    Connection name
                    <input
                      placeholder={providers[editing.provider]}
                      maxLength={200}
                      value={editing.name}
                      onChange={(e) =>
                        setEditing({ ...editing, name: e.target.value })
                      }
                    />
                  </label>
                  <div className="chat-select-field">
                    <label htmlFor={keyStorageId}>Key storage</label>
                    <Select
                      id={keyStorageId}
                      value={editing.secretMode}
                      onChange={(value) =>
                        setEditing({
                          ...editing,
                          secretMode: value as Connection["secretMode"],
                        })
                      }
                      options={[
                        { value: "system", label: "System credential store" },
                        {
                          value: "session",
                          label: "Session only · expires when the app closes",
                        },
                      ]}
                    />
                  </div>
                  <label className="chat-checkbox">
                    <input
                      type="checkbox"
                      checked={editing.enabled}
                      onChange={(e) =>
                        setEditing({ ...editing, enabled: e.target.checked })
                      }
                    />{" "}
                    Enabled
                  </label>
                  {data.connections.some((c) => c.id === editing.id) && (
                    <p>
                      Changing the key or storage mode stops active requests
                      using this connection and preserves their responses.
                      Changing storage requires a new key.
                    </p>
                  )}
                </details>
                {error && <p role="alert">{error}</p>}
                <div className="dialog-actions">
                  <button
                    type="button"
                    className="button"
                    disabled={busy}
                    onClick={closeSetup}
                  >
                    Cancel
                  </button>
                  <button
                    className="button chat-primary-button"
                    disabled={
                      busy ||
                      (!originalConnection && catalog.status !== "ready") ||
                      (makeDefault && !validModelId(editingModel.trim()))
                    }
                  >
                    {busy
                      ? "Saving…"
                      : originalConnection
                        ? "Save changes"
                        : "Add provider"}
                  </button>
                </div>
              </form>
            )
          )}
        </Modal>
      )}
      {addingModel && connection && data && (
        <Modal
          className="chat-dialog"
          title="Add model"
          onClose={() => {
            if (!busy) setAddingModel(false);
          }}
        >
          <form
            className="dialog-form chat-config"
            onSubmit={(event) => {
              event.preventDefault();
              if (!validModelId(newModel.trim())) return;
              void run(async () => {
                await save({
                  ...data,
                  connections: data.connections.map((c) =>
                    c.id === connection.id
                      ? {
                          ...c,
                          models: [...new Set([...c.models, newModel.trim()])],
                        }
                      : c,
                  ),
                });
                setSearch("");
                setAddingModel(false);
                setStatus("Model added");
              });
            }}
          >
            <label>
              Model ID
              <input
                autoFocus
                required
                maxLength={200}
                spellCheck={false}
                value={newModel}
                onChange={(e) => setNewModel(e.target.value)}
                placeholder="Exact ID from the provider’s model catalog"
              />
            </label>
            <p className="chat-field-help">
              Use the exact model ID from {preset.name}. Adding a model does not
              send a request or verify access.
            </p>
            {error && <p role="alert">{error}</p>}
            <div className="dialog-actions">
              <button
                type="button"
                className="button"
                disabled={busy}
                onClick={() => setAddingModel(false)}
              >
                Cancel
              </button>
              <button
                className="button chat-primary-button"
                disabled={busy || !validModelId(newModel.trim())}
              >
                Add model
              </button>
            </div>
          </form>
        </Modal>
      )}
      {recovering && (
        <Modal
          className="chat-dialog"
          title="Recover Chat AI settings?"
          tone="warning"
          onClose={() => setRecovering(false)}
        >
          <div className="dialog-form">
            <p>
              The original preferences will be moved to a private recovery
              folder beside the preferences file. Start with empty connection
              settings; conversation history remains. Keys whose identities
              cannot be read may remain in the system credential store.
            </p>
            <button
              className="button"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  await api("chat_recover", {
                    target: "settings",
                    reset: true,
                  });
                  await reload();
                  setRecovering(false);
                })
              }
            >
              Back up and reset settings
            </button>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
      {removingKey && data && (
        <Modal
          className="chat-dialog"
          title="Remove API key?"
          tone="warning"
          onClose={() => setRemovingKey(null)}
        >
          <div className="dialog-form">
            <p>
              Remove the key for “{removingKey.name}”? Its active responses will
              stop. The connection and history remain.
            </p>
            <button
              className="button button-danger"
              disabled={busy}
              onClick={() =>
                void run(async () => {
                  setData(
                    await api<Preferences>("chat_preferences_save", {
                      data,
                      expected: data.revision,
                      clearKey: removingKey.id,
                    }),
                  );
                  setRemovingKey(null);
                })
              }
            >
              Remove key
            </button>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
      {deleting && data && (
        <Modal
          className="chat-dialog"
          title="Remove provider?"
          tone="danger"
          onClose={() => {
            if (!busy) setDeleting(null);
          }}
        >
          <div className="dialog-form">
            <p>
              Remove “{deleting.name}” and its stored key? Active responses will
              stop. Conversation history stays on this device.
            </p>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setDeleting(null)}
              >
                Cancel
              </button>
              <button
                className="button button-danger"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await save({
                      ...data,
                      connections: data.connections.filter(
                        (c) => c.id !== deleting.id,
                      ),
                      defaults:
                        data.defaults.connectionId === deleting.id
                          ? defaultConfig
                          : data.defaults,
                    });
                    setSelected((id) => (id === deleting.id ? "" : id));
                    setDeleting(null);
                    setStatus("Provider removed");
                  })
                }
              >
                Remove provider
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
    </main>
  );
}
