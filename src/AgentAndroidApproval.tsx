import { useState } from "react";
import { api } from "./api";
import { useT } from "./i18n";

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
  const { t } = useT();
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
        ? t(
            "Android operation approved. Progress is available in Android settings.",
          )
        : t("Android operation denied."),
    );
  return (
    <article className="agent-control-request">
      <h3>
        {request.clientLabel} — {request.plan.target}
      </h3>
      <p className="settings-help">
        {t(
          "Expires in {seconds} seconds. This approval applies only to the plan shown here.",
          { seconds: request.secondsRemaining },
        )}
      </p>
      {request.plan.downloads.length > 0 && (
        <>
          <p>
            {t(
              "Download: {size} GiB. Additional space is needed to unpack and install the components.",
              { size: (request.plan.downloadBytes / 1024 ** 3).toFixed(2) },
            )}
          </p>
          <ul>
            {request.plan.downloads.map((item) => (
              <li key={item.id}>
                <strong>{item.name}</strong> — {item.revision},{" "}
                {t("{size} MiB", { size: (item.bytes / 1024 ** 2).toFixed(1) })}
                <details>
                  <summary>{t("Download source and checksum")}</summary>
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
          <dt>{t("System image")}</dt>
          <dd>{request.action.image ?? t("Keep the current image")}</dd>
          <dt>{t("Phone profile")}</dt>
          <dd>{request.action.profile}</dd>
          <dt>{t("Memory")}</dt>
          <dd>{t("{size} MiB", { size: request.action.hardware.ramMib })}</dd>
          <dt>{t("CPU cores")}</dt>
          <dd>{request.action.hardware.cpuCount}</dd>
          <dt>{t("Data partition")}</dt>
          <dd>{t("{size} GiB", { size: request.action.hardware.dataGib })}</dd>
          <dt>{t("Graphics")}</dt>
          <dd>{request.action.hardware.gpu}</dd>
          <dt>{t("Boot")}</dt>
          <dd>
            {request.action.hardware.quickBoot
              ? t("Quick boot")
              : t("Cold boot")}
          </dd>
        </dl>
      )}
      {request.licenses.map((license) => (
        <section className="android-license" key={license.digest}>
          <details>
            <summary>{t("{id} — provider terms", { id: license.id })}</summary>
            <pre tabIndex={0} aria-label={t("{id} terms", { id: license.id })}>
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
            {t("I have read and accept these provider terms")}
          </label>
        </section>
      ))}
      {request.action?.type === "restore_metadata" && request.action.reset && (
        <p className="settings-help">
          {t(
            "Android preferences will return to their defaults. Device recovery requires a valid backup and cannot reset the device registry.",
          )}
        </p>
      )}
      {erase && (
        <label>
          {request.action?.type === "restore_metadata"
            ? t("Reset metadata: type “{text}” to confirm", {
                text: requiredConfirmation ?? "",
              })
            : t("Erase all data: type “{text}” to confirm", {
                text: requiredConfirmation ?? "",
              })}
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
          {t("Deny Android operation")}
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
          {t("Approve Android operation")}
        </button>
      </div>
    </article>
  );
}
