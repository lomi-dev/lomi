import { useState } from "react";
import {
  useHostContext,
  type PluginContext,
  type ViewProps,
} from "@lomi-dev/plugin-sdk";
export function activate(context: PluginContext) {
  function View({ panel, setState }: ViewProps) {
    const host = useHostContext();
    const [details, showDetails] = useState("");
    const expanded =
      typeof panel.state === "object" &&
      panel.state !== null &&
      !Array.isArray(panel.state) &&
      panel.state.expanded === true;
    return (
      <section className="context-plugin">
        <img
          src={context.assetUrl("compass.svg")}
          width="24"
          height="24"
          alt=""
        />
        <h2>Workspace context</h2>
        <dl>
          <dt>Folder</dt>
          <dd>{host.projectPath ?? "No folder"}</dd>
          <dt>Workspace</dt>
          <dd>{host.workspaceName ?? "No workspace"}</dd>
          <dt>Appearance</dt>
          <dd>{host.appearance}</dd>
        </dl>
        <button
          className="button"
          onClick={async () => {
            setState({ expanded: !expanded });
            if (!expanded) {
              try {
                showDetails((await import("./details")).description);
              } catch (error) {
                showDetails(`Could not load details: ${String(error)}`);
              }
            }
          }}
        >
          {" "}
          {expanded ? "Hide details" : "Show details"}{" "}
        </button>
        {expanded && (
          <p>
            {details ||
              "This view follows the selected workspace and remembers its display choice."}
          </p>
        )}
      </section>
    );
  }
  function Status() {
    const host = useHostContext();
    return (
      <button
        className="text-button"
        disabled={!host.workspaceName}
        onClick={() =>
          void context
            .executeCommand("simplebench.context.open")
            .catch(() => {})
        }
      >
        Context
      </button>
    );
  }
  context.registerView("simplebench.context.view", View);
  context.registerCommand("simplebench.context.open", async () => {
    await context.openView("simplebench.context.view", { expanded: false });
  });
  context.registerFill("simplebench.context.status", Status);
}
