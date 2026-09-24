import { useState } from "react";
import { api } from "./api";

export interface AndroidManagementApproval {
  operationId: string;
  clientLabel: string;
  workspaceId: string;
  secondsRemaining: number;
  plan: {
    planId: string;
    revision: string;
    target: string;
    downloadBytes: number;
    downloads: {
      id: string;
      name: string;
      revision: string;
      url: string;
      bytes: number;
      checksum: string;
    }[];
  };
  licenses: { id: string; digest: string; text: string }[];
  action: null | {
    type: string;
    file?: "devices" | "preferences";
    reset?: boolean;
    deviceId?: string;
    generation?: string;
    confirmation?: string;
    image?: string;
    profile?: string;
    hardware?: {
      ramMib: number;
      cpuCount: number;
      dataGib: number;
      gpu: string;
      quickBoot: boolean;
    };
  };
}

export function AgentAndroidApproval({
  request,
  busy,
  run,
}: {
  request: AndroidManagementApproval;
  busy: boolean;
  run: (action: () => Promise<unknown>, message: string) => Promise<void>;
}) {
  const [accepted, setAccepted] = useState<string[]>([]);
  const [confirmation, setConfirmation] = useState("");
  const requiredConfirmation =
    request.action?.type === "restore_metadata" && request.action.reset
      ? request.action.file === "devices"
        ? "RESET DEVICES"
        : "RESET PREFERENCES"
      : request.action?.type === "wipe" || request.action?.type === "delete"
        ? request.action.confirmation
        : undefined;
  const erase = requiredConfirmation !== undefined;
  const decide = (approve: boolean) =>
    run(
      () =>
        api("agent_control_decide_android_management", {
          operationId: request.operationId,
          revision: request.plan.revision,
          approve,
          accepted: approve ? accepted : [],
          confirmation: approve && erase ? confirmation : null,
        }),
      approve
        ? "Android operation approved. Progress is available in Android settings."
        : "Android operation denied.",
    );
  return (
    <article className="agent-control-request">
      <h3>
        {request.clientLabel} — {request.plan.target}
      </h3>
      <p className="settings-help">
        Expires in {request.secondsRemaining} seconds. This approval applies
        only to the plan shown here.
      </p>
      {request.plan.downloads.length > 0 && (
        <>
          <p>
            Download: {(request.plan.downloadBytes / 1024 ** 3).toFixed(2)} GiB.
            Additional space is needed to unpack and install the components.
          </p>
          <ul>
            {request.plan.downloads.map((item) => (
              <li key={item.id}>
                <strong>{item.name}</strong> — {item.revision},{" "}
                {(item.bytes / 1024 ** 2).toFixed(1)} MiB
                <details>
                  <summary>Download source and checksum</summary>
                  <p>{item.url}</p>
                  <code>{item.checksum}</code>
                </details>
              </li>
            ))}
          </ul>
        </>
      )}
      {request.action?.hardware && (
        <dl>
          <dt>System image</dt>
          <dd>{request.action.image ?? "Keep the current image"}</dd>
          <dt>Phone profile</dt>
          <dd>{request.action.profile}</dd>
          <dt>Memory</dt>
          <dd>{request.action.hardware.ramMib} MiB</dd>
          <dt>CPU cores</dt>
          <dd>{request.action.hardware.cpuCount}</dd>
          <dt>Data partition</dt>
          <dd>{request.action.hardware.dataGib} GiB</dd>
          <dt>Graphics</dt>
          <dd>{request.action.hardware.gpu}</dd>
          <dt>Boot</dt>
          <dd>
            {request.action.hardware.quickBoot ? "Quick boot" : "Cold boot"}
          </dd>
        </dl>
      )}
      {request.licenses.map((license) => (
        <section className="android-license" key={license.digest}>
          <details>
            <summary>{license.id} — provider terms</summary>
            <pre tabIndex={0} aria-label={`${license.id} terms`}>
              {license.text}
            </pre>
          </details>
          <label>
            <input
              type="checkbox"
              checked={accepted.includes(license.digest)}
              disabled={busy}
              onChange={(event) =>
                setAccepted((current) =>
                  event.target.checked
                    ? [...current, license.digest]
                    : current.filter((id) => id !== license.digest),
                )
              }
            />{" "}
            I have read and accept these provider terms
          </label>
        </section>
      ))}
      {request.action?.type === "restore_metadata" && request.action.reset && (
        <p className="settings-help">
          Android preferences will return to their defaults. Device recovery
          requires a valid backup and cannot reset the device registry.
        </p>
      )}
      {erase && (
        <label>
          {request.action?.type === "restore_metadata"
            ? "Reset metadata"
            : "Erase all data"}
          : type “{requiredConfirmation}” to confirm
          <input
            value={confirmation}
            disabled={busy}
            onChange={(event) => setConfirmation(event.target.value)}
            autoComplete="off"
            spellCheck={false}
          />
        </label>
      )}
      <div className="agent-control-actions">
        <button
          className="button"
          disabled={busy}
          onClick={() => void decide(false)}
        >
          Deny Android operation
        </button>
        <button
          className="button button-primary"
          disabled={
            busy ||
            accepted.length !== request.licenses.length ||
            (erase && confirmation !== requiredConfirmation)
          }
          onClick={() => void decide(true)}
        >
          Approve Android operation
        </button>
      </div>
    </article>
  );
}
