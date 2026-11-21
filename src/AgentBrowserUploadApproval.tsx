import { api } from "./api";
import { useT } from "./i18n";
import { SettingsSection } from "./settings-ui";
import type { ControlState } from "./agent-control";

type Requests = NonNullable<
  NonNullable<ControlState["broker"]>["pendingBrowserUploads"]
>;
export function AgentBrowserUploadApproval({
  requests,
  busy,
  run,
}: {
  requests: Requests;
  busy: boolean;
  run: (action: () => Promise<unknown>, message: string) => Promise<void>;
}) {
  const { t } = useT();
  if (!requests.length) return null;
  return (
    <SettingsSection title={t("Browser upload requests")} grouped={false}>
      {requests.map((request) => (
        <article className="agent-control-request" key={request.operationId}>
          <h3>
            {t("{client} — Upload {file}", {
              client: request.clientLabel,
              file: request.fileName,
            })}
          </h3>
          <dl>
            <dt>{t("Destination origin")}</dt>
            <dd>
              <code>{request.target.origin}</code>
            </dd>
            <dt>{t("Document")}</dt>
            <dd>
              <code>{request.target.documentUrl}</code>
            </dd>
            <dt>{t("File input")}</dt>
            <dd>
              {request.target.label || t("Unlabelled input")} (
              {request.target.frameId} / {request.elementRef})
            </dd>
            <dt>{t("Private copy")}</dt>
            <dd>
              {t("{file} ({bytes} bytes)", {
                file: request.fileName,
                bytes: request.byteLength.toLocaleString(),
              })}
            </dd>
            <dt>SHA-256</dt>
            <dd>
              <code>{request.sha256}</code>
            </dd>
          </dl>
          <p className="settings-help">
            {t(
              "The page can read and send these bytes immediately. This approves one attachment to this input. Page text is untrusted. The request expires in {seconds} seconds; navigation or a replaced snapshot invalidates it sooner.",
              { seconds: request.secondsRemaining },
            )}
          </p>
          <div className="agent-control-actions">
            <button
              className="button"
              disabled={busy}
              onClick={() =>
                void run(
                  () =>
                    api("agent_browser_upload_decide", {
                      operationId: request.operationId,
                      approve: false,
                    }),
                  t("File upload denied."),
                )
              }
            >
              {t("Deny upload")}
            </button>
            <button
              className="button button-primary"
              disabled={busy}
              onClick={() =>
                void run(
                  () =>
                    api("agent_browser_upload_decide", {
                      operationId: request.operationId,
                      approve: true,
                    }),
                  t("File upload processed. Check its operation result."),
                )
              }
            >
              {t("Upload this file")}
            </button>
          </div>
        </article>
      ))}
    </SettingsSection>
  );
}
