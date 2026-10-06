import { useEffect, useId, useRef, useState } from "react";
import { emitTo } from "@tauri-apps/api/event";
import { api, errorMessage } from "../../api";
import { cliNames, type CliAgent } from "../../cli-agents";
import { newId, type AppInfo } from "../../model";
import { SettingsSection } from "../../settings-ui";
import { Modal } from "../../ui";
import { useAccountsSnapshot } from "../useAccountsSnapshot";
import type { AccountInstance } from "../types";
import "./accounts.css";
export default function AccountsSettings() {
  const { snapshot, error: nativeError, busy, command } = useAccountsSnapshot();
  const [cli, setCli] = useState<CliAgent>("codex");
  const [label, setLabel] = useState("");
  const [error, setError] = useState("");
  const [info, setInfo] = useState<AppInfo>();
  const [removing, setRemoving] = useState<AccountInstance>();
  const [renaming, setRenaming] = useState<AccountInstance>();
  const [recovering, setRecovering] = useState<AccountInstance>();
  const [reviewedEffects, setReviewedEffects] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [shellProfileId, setShellProfileId] = useState("");
  const cancel = useRef<HTMLButtonElement>(null);
  const reviewHelpId = useId();
  useEffect(() => {
    let live = true;
    void api<AppInfo>("app_info")
      .then((next) => {
        if (live) {
          setInfo(next);
          setShellProfileId(next.profiles[0]?.id ?? "");
        }
      })
      .catch((cause) => {
        if (live) setError(errorMessage(cause));
      });
    return () => {
      live = false;
    };
  }, []);
  const perform = async (name: string, request: Record<string, unknown>) => {
    setError("");
    try {
      return await command(name, { operationId: newId(), ...request });
    } catch (cause) {
      setError(errorMessage(cause));
    }
  };
  const currentRecovery = snapshot?.accounts.find(
    (account) => account.accountId === recovering?.accountId,
  )?.recovery;
  const canRecover = (account: AccountInstance) =>
    account.recovery?.state === "effects_review_required" &&
    account.recovery.recoverable &&
    !account.recovery.requiresVerifiedBootChange;
  return (
    <div className="dialog-form" aria-busy={busy}>
      <SettingsSection
        title="CLI Accounts"
        description="Separate native CLI logins. Each task explicitly selects its next account."
      >
        <div className="dialog-form">
          <label>
            CLI
            <select
              value={cli}
              disabled={busy}
              onChange={(event) => setCli(event.target.value as CliAgent)}
            >
              {snapshot?.capabilities.map((capability) => (
                <option key={capability.cli} value={capability.cli}>
                  {cliNames[capability.cli]}
                </option>
              ))}
            </select>
          </label>
          <label>
            Account label
            <input
              value={label}
              disabled={busy}
              onChange={(event) => setLabel(event.target.value)}
            />
          </label>
          <button
            className="button button-primary"
            disabled={busy || !label.trim()}
            onClick={() =>
              void perform("agent_account_create", {
                cli,
                label: label.trim(),
              }).then((next) => {
                if (next) setLabel("");
              })
            }
          >
            Add account
          </button>
          <label>
            Shell for verification
            <select
              value={shellProfileId}
              onChange={(event) => setShellProfileId(event.target.value)}
            >
              {info?.profiles.map((profile) => (
                <option key={profile.id} value={profile.id}>
                  {profile.name}
                </option>
              ))}
            </select>
          </label>
        </div>
        <p role="note" aria-label="Native account operation limitation">
          Native login and verification can require a computer restart before
          you can use this account or edit its project again. Lomi verifies the
          restart, then asks you to review any changes.
        </p>
        {(error || nativeError) && <p role="alert">{error || nativeError}</p>}
        {!snapshot && <p role="status">Loading accounts…</p>}
        {snapshot?.accounts.map((account) => {
          const capability = snapshot.capabilities.find(
            (item) => item.cli === account.cli,
          );
          return (
            <section key={account.accountId} aria-label={account.label}>
              <h3>
                {account.label} · {cliNames[account.cli]}
              </h3>
              <p role="status">
                {account.authState} · login revision {account.authRevision}
                {account.acceptedVersion
                  ? ` · CLI ${account.acceptedVersion}`
                  : ""}
              </p>
              {account.availabilityReason && (
                <p>{account.availabilityReason}</p>
              )}
              <p className="settings-help">
                {capability?.reason} Quota unavailable.
              </p>
              {account.recovery && (
                <div>
                  <p id={`account-recovery-${account.accountId}`} role="alert">
                    {account.recovery.reason}
                  </p>
                  {account.recovery.requiresVerifiedBootChange && (
                    <p className="settings-help">
                      Restart your computer, then reopen Lomi so it can verify
                      that the previous background work ended.
                    </p>
                  )}
                  <button
                    className="button"
                    aria-describedby={`account-recovery-${account.accountId}`}
                    disabled={busy || !canRecover(account)}
                    onClick={() => {
                      setError("");
                      setReviewedEffects(false);
                      setRecovering(account);
                    }}
                  >
                    Review interrupted operation
                  </button>
                </div>
              )}
              <div className="dialog-actions">
                <button
                  className="button"
                  disabled={busy || !!account.recovery}
                  onClick={() => {
                    setRenaming(account);
                    setNewLabel(account.label);
                  }}
                >
                  Rename
                </button>
                <button
                  className="button"
                  disabled={busy || !!account.recovery}
                  onClick={() =>
                    void perform("agent_account_update", {
                      accountId: account.accountId,
                      expectedRevision: account.revision,
                      enabled: !account.enabled,
                      label: null,
                    })
                  }
                >
                  {account.enabled ? "Disable" : "Enable"}
                </button>
                <button
                  className="button"
                  disabled={
                    busy ||
                    !!account.recovery ||
                    !account.enabled ||
                    !capability?.accountTerminal
                  }
                  onClick={() =>
                    void emitTo(
                      { kind: "Webview", label: "main" },
                      "agent-account-open-terminal",
                      { accountId: account.accountId },
                    ).catch((cause) => setError(errorMessage(cause)))
                  }
                >
                  Open login terminal
                </button>
                <button
                  className="button"
                  disabled={
                    busy ||
                    !!account.recovery ||
                    !account.enabled ||
                    !capability?.accountTerminal ||
                    !shellProfileId ||
                    !info
                  }
                  onClick={() =>
                    void perform("agent_account_verify", {
                      accountId: account.accountId,
                      expectedRevision: account.revision,
                      shellProfileId,
                      cwd: info!.directory,
                    })
                  }
                >
                  Verify account
                </button>
                <button
                  className="button"
                  disabled={busy || !!account.recovery}
                  onClick={() => setRemoving({ ...account })}
                >
                  Remove account
                </button>
              </div>
            </section>
          );
        })}
      </SettingsSection>
      <SettingsSection
        title="CLI capabilities"
        description="Execution support depends on the exact admitted CLI version and platform."
      >
        <table>
          <thead>
            <tr>
              <th>CLI</th>
              <th>Account terminal</th>
              <th>Native task</th>
              <th>Admitted versions</th>
              <th>Qualification</th>
            </tr>
          </thead>
          <tbody>
            {snapshot?.capabilities.map((capability) => (
              <tr key={capability.cli}>
                <td>{cliNames[capability.cli]}</td>
                <td>
                  {capability.accountTerminal ? "Available" : "Unavailable"}
                </td>
                <td>
                  {capability.managedExecution ? "Available" : "Unavailable"}
                </td>
                <td>{capability.versions.join(", ") || "None"}</td>
                <td>{capability.reason}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </SettingsSection>
      {recovering && (
        <Modal
          title={`Review ${recovering.label}`}
          initialFocus={cancel}
          closeDisabled={busy}
          onClose={() => setRecovering(undefined)}
        >
          <div className="dialog-form">
            <p>{currentRecovery?.reason ?? recovering.recovery?.reason}</p>
            <p id={reviewHelpId}>
              Review the account and any project changes from the interrupted
              operation. Completing recovery invalidates previous login
              verification and history access grants.
            </p>
            <label className="agent-account-recovery-confirmation">
              <input
                type="checkbox"
                aria-describedby={reviewHelpId}
                checked={reviewedEffects}
                disabled={busy}
                onChange={(event) => setReviewedEffects(event.target.checked)}
              />
              I reviewed the account and project effects
            </label>
            <div className="dialog-actions">
              <button
                ref={cancel}
                className="button"
                disabled={busy}
                onClick={() => setRecovering(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={
                  busy ||
                  !reviewedEffects ||
                  currentRecovery?.state !== "effects_review_required" ||
                  !currentRecovery.recoverable ||
                  currentRecovery.requiresVerifiedBootChange
                }
                onClick={() =>
                  void perform("agent_account_recover", {
                    accountId: recovering.accountId,
                    expectedRevision: recovering.revision,
                    acknowledgeEffects: reviewedEffects,
                  }).then((next) => {
                    if (next) setRecovering(undefined);
                  })
                }
              >
                Complete recovery
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
      {renaming && (
        <Modal
          title="Rename account"
          closeDisabled={busy}
          onClose={() => setRenaming(undefined)}
        >
          <div className="dialog-form">
            <label>
              Account label
              <input
                autoFocus
                value={newLabel}
                onChange={(event) => setNewLabel(event.target.value)}
              />
            </label>
            <div className="dialog-actions">
              <button
                className="button"
                disabled={busy}
                onClick={() => setRenaming(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-primary"
                disabled={busy || !newLabel.trim()}
                onClick={() =>
                  void perform("agent_account_update", {
                    accountId: renaming.accountId,
                    expectedRevision: renaming.revision,
                    label: newLabel.trim(),
                    enabled: null,
                  }).then((next) => {
                    if (next) setRenaming(undefined);
                  })
                }
              >
                Save label
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
      {removing && (
        <Modal
          title="Remove CLI account?"
          initialFocus={cancel}
          closeDisabled={busy}
          onClose={() => setRemoving(undefined)}
        >
          <div className="dialog-form">
            <p>
              Remove {removing.label} from available accounts? Its physical
              login and history files remain preserved.
            </p>
            <div className="dialog-actions">
              <button
                ref={cancel}
                className="button"
                disabled={busy}
                onClick={() => setRemoving(undefined)}
              >
                Cancel
              </button>
              <button
                className="button button-danger"
                disabled={busy}
                onClick={() =>
                  void perform("agent_account_remove", {
                    accountId: removing.accountId,
                    expectedRevision: removing.revision,
                  }).then((next) => {
                    if (next) setRemoving(undefined);
                  })
                }
              >
                Remove account
              </button>
            </div>
            {error && <p role="alert">{error}</p>}
          </div>
        </Modal>
      )}
    </div>
  );
}
