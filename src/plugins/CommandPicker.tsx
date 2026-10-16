import { useState } from "react";
import { Modal } from "../ui";
import { useKeybindings } from "../KeybindingsProvider";
import { isZoomAction, type ActionId } from "../keybindings";
import { usePlugins } from "./PluginsProvider";
import { pluginHost } from "./runtime";
import { commandAvailable } from "./host";
import { errorMessage } from "../api";
export default function CommandPicker({
  onClose,
  onBuiltin,
  onError,
  settingsActive = false,
}: {
  onClose: () => void;
  onBuiltin: (id: ActionId) => void;
  onError: (error: string) => void;
  settingsActive?: boolean;
}) {
  const [query, setQuery] = useState("");
  const { actions } = useKeybindings();
  const { catalog } = usePlugins();
  const context = pluginHost.context;
  const builtinAvailable = (id: ActionId) => {
    if (settingsActive) {
      if (
        id === "openSettings" ||
        id === "closeTab" ||
        id === "closeTerminal" ||
        isZoomAction(id)
      )
        return true;
      return (
        !!context.workspaceName &&
        ["newTab", "nextTab", "previousTab"].includes(id)
      );
    }
    if (id === "openSettings" || id === "toggleWorkspaces" || isZoomAction(id))
      return true;
    if (!context.workspaceName) return false;
    if (["chatFocusInput", "chatStop", "chatHistory"].includes(id))
      return context.viewType === "chat";
    if (id === "movePanel")
      return document.querySelectorAll(".dock-pane-host").length > 1;
    if (id === "dockTab")
      return (
        !["diff", "commit"].includes(context.viewType ?? "") &&
        document.querySelectorAll(".tab").length > 1
      );
    if (id === "runCommand")
      return !!document.querySelector(
        ".terminal-pane.is-active .composer-run:not(:disabled)",
      );
    if (["saveFile", "findFile", "goToLine", "toggleWordWrap"].includes(id))
      return context.viewType === "file";
    if (
      [
        "searchTerminal",
        "commandInput",
        "commandBlocks",
        "copyTerminal",
        "pasteTerminal",
        "changeEnvironment",
        "terminalOverview",
      ].includes(id)
    )
      return context.viewType === "terminal";
    if (["newTerminal", "splitVertical"].includes(id))
      return !!document.querySelector(".split-container");
    if (id === "toggleSourceControl")
      return !!document.querySelector(
        '[title^="Toggle Source Control"], [title^="Toggle source control"]',
      );
    return true;
  };
  const views = catalog.entries
    .flatMap((entry) =>
      (entry.manifest?.contributes?.views ?? []).map((view) => ({
        entry,
        view,
      })),
    )
    .filter(({ view }) =>
      `Open ${view.title}`.toLowerCase().includes(query.toLowerCase()),
    );
  const commands = actions.filter(
    (action) =>
      action.id !== "commandPicker" &&
      `${action.label} ${action.group}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  );
  return (
    <Modal title="Commands" onClose={onClose}>
      <div className="dialog-form">
        <input
          aria-label="Find command"
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        <div className="command-picker-list">
          {commands.map((command) => {
            const entry = catalog.entries.find((e) =>
              e.manifest?.contributes?.commands?.some(
                (c) => c.id === command.id,
              ),
            );
            const declared = entry?.manifest?.contributes?.commands?.find(
              (c) => c.id === command.id,
            );
            const disabled = entry
              ? !entry.enabled ||
                catalog.safeMode ||
                !commandAvailable(declared?.context, pluginHost.context, false)
              : !builtinAvailable(command.id);
            return (
              <button
                className="button"
                key={command.id}
                disabled={disabled}
                onClick={() => {
                  onClose();
                  if (entry)
                    void pluginHost
                      .execute(command.id)
                      .catch((error) => onError(errorMessage(error)));
                  else onBuiltin(command.id);
                }}
              >
                <span>{command.label}</span>
                <small>{command.group}</small>
              </button>
            );
          })}
          {views.map(({ entry, view }) => (
            <button
              className="button"
              key={view.id}
              disabled={
                !entry.enabled || catalog.safeMode || !context.workspaceName
              }
              onClick={() => {
                onClose();
                void pluginHost
                  .openView(view.id, null)
                  .catch((error) => onError(errorMessage(error)));
              }}
            >
              <span>Open {view.title}</span>
              <small>
                {entry.manifest!.name} · {view.placement}
              </small>
            </button>
          ))}
        </div>
      </div>
    </Modal>
  );
}
