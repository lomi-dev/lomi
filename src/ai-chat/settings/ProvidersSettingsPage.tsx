import { useCallback, useEffect, useState } from "react";
import { ChevronRight, Plus, MessageSquare, RefreshCw } from "../../icons";
import {
  SettingsNotice,
  SettingsPage,
  SettingsSection,
} from "../../settings-ui";
import { ProviderIcon } from "../../ProviderIcon";
import { useChatProviders } from "../providers";
import AddProviderDialog from "./AddProviderDialog";
import ProviderDetail, {
  toggleProvider,
  type ProviderContext,
} from "./ProviderDetail";
import "./providers-settings.css";

export default function ProvidersSettingsPage() {
  const { entries, settings, error, busy, run, refreshStatus, detect } =
    useChatProviders();
  const [scanStatus, setScanStatus] = useState<string>();
  const scan = useCallback(async () => {
    const count = await detect();
    setScanStatus(
      count
        ? `Found ${count} new CLI ${count === 1 ? "provider" : "providers"}`
        : "No new CLI providers found",
    );
  }, [detect]);
  const [selected, setSelected] = useState<string>();
  const [adding, setAdding] = useState(false);
  useEffect(() => {
    void scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per mount
  }, []);
  const active = entries.find((entry) => entry.instanceId === selected);
  const ctx: ProviderContext | undefined = settings
    ? {
        settings,
        busy,
        run: (name, request) => run(name, request).catch(() => undefined),
        refreshStatus,
      }
    : undefined;
  const addButton = (
    <button
      type="button"
      className="button"
      disabled={!ctx}
      onClick={() => setAdding(true)}
    >
      <Plus size={14} aria-hidden="true" />
      Add provider
    </button>
  );
  return (
    <SettingsPage
      title="Providers"
      busy={!ctx || ctx.busy}
      status={!ctx && !error ? "Loading…" : scanStatus}
      contentClassName="prov-page"
    >
      {error && !adding && (
        <SettingsNotice tone="error">{error}</SettingsNotice>
      )}
      {ctx && entries.length === 0 && (
        <div className="prov-card prov-empty">
          <span className="prov-empty-icon">
            <MessageSquare size={20} aria-hidden="true" />
          </span>
          <h3>No providers yet</h3>
          <p>
            Installed CLIs appear here automatically. Add an API to use a hosted
            model.
          </p>
          {addButton}
        </div>
      )}
      {ctx && entries.length > 0 && !active && (
        <SettingsSection
          title="Providers"
          count={entries.length}
          actions={
            <>
              <button
                type="button"
                className="icon-button"
                title="Rescan installed CLIs"
                aria-label="Rescan installed CLIs"
                disabled={ctx.busy}
                onClick={() => void scan()}
              >
                <RefreshCw size={14} aria-hidden="true" />
              </button>
              {addButton}
            </>
          }
        >
          <ul className="prov-list" aria-label="Providers">
            {entries.map((entry) => (
              <li
                key={entry.instanceId}
                data-disabled={!entry.enabled || undefined}
              >
                <button
                  type="button"
                  className="prov-row"
                  onClick={() => setSelected(entry.instanceId)}
                >
                  <span className="prov-tile-icon">
                    <ProviderIcon kind={entry.kind} cli={entry.cli} />
                  </span>
                  <span className="prov-row-text">
                    <span className="prov-row-title">
                      <span>{entry.label}</span>
                      {entry.versionLabel && <code>{entry.versionLabel}</code>}
                    </span>
                    <small
                      className="prov-status"
                      data-ok={entry.available || undefined}
                    >
                      {entry.statusLine}
                    </small>
                  </span>
                </button>
                <input
                  type="checkbox"
                  role="switch"
                  className="settings-switch"
                  aria-label={`Enable ${entry.label}`}
                  checked={entry.enabled}
                  disabled={ctx.busy}
                  onChange={(event) =>
                    toggleProvider(ctx, entry, event.target.checked)
                  }
                />
                <ChevronRight
                  size={16}
                  aria-hidden="true"
                  className="prov-chevron"
                />
              </li>
            ))}
          </ul>
        </SettingsSection>
      )}
      {ctx && active && (
        <ProviderDetail
          key={active.instanceId}
          entry={active}
          ctx={ctx}
          onBack={() => setSelected(undefined)}
        />
      )}
      {adding && ctx && (
        <AddProviderDialog
          ctx={ctx}
          entries={entries}
          error={error}
          onClose={() => setAdding(false)}
          onAdded={setSelected}
        />
      )}
    </SettingsPage>
  );
}
