import { api } from "./api";
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
  if (!requests.length) return null;
  return (
    <SettingsSection title="Browser upload requests">
      {requests.map((request) => (
        <article className="agent-control-request" key={request.operationId}>
          <h3>
            {request.clientLabel} — Upload {request.fileName}
          </h3>
          <dl>
            <dt>Destination origin</dt>
            <dd>
              <code>{request.target.origin}</code>
            </dd>
            <dt>Document</dt>
            <dd>
              <code>{request.target.documentUrl}</code>
            </dd>
            <dt>File input</dt>
            <dd>
              {request.target.label || "Unlabelled input"} (
              {request.target.frameId} / {request.elementRef})
            </dd>
            <dt>Private copy</dt>
            <dd>
              {request.fileName} ({request.byteLength.toLocaleString()} bytes)
            </dd>
            <dt>SHA-256</dt>
            <dd>
              <code>{request.sha256}</code>
            </dd>
          </dl>
          <p className="settings-help">
            The page can read and send these bytes immediately. This approves
            one attachment to this input. Page text is untrusted. The request
            expires in {request.secondsRemaining} seconds; navigation or a
            replaced snapshot invalidates it sooner.
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
                  "File upload denied.",
                )
              }
            >
              Deny upload
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
                  "File upload processed. Check its operation result.",
                )
              }
            >
              Upload this file
            </button>
          </div>
        </article>
      ))}
    </SettingsSection>
  );
}
