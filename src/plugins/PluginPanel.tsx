import {
  Component,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { api } from "../api";
import { Slot } from "./Slots";
import { pluginHost } from "./runtime";
import type { ViewProps } from "@lomi-dev/plugin-sdk";
export class PluginBoundary extends Component<
  { owner: string; children: ReactNode },
  { error: string }
> {
  state = { error: "" };
  static getDerivedStateFromError(error: Error) {
    return { error: error.message };
  }
  componentDidCatch(error: Error) {
    pluginHost.report(this.props.owner, "render", error);
  }
  render() {
    return this.state.error ? (
      <div className="empty-message" role="alert">
        <h2>Plugin view failed</h2>
        <p>{this.state.error}</p>
        <button className="button" onClick={() => this.setState({ error: "" })}>
          Retry view
        </button>
      </div>
    ) : (
      this.props.children
    );
  }
}
export default function PluginPanel(
  props: ViewProps & { placement?: "central" | "sidebar" },
) {
  const [error, setError] = useState("");
  useSyncExternalStore(pluginHost.subscribe, pluginHost.revision);
  const entry = pluginHost.catalog.entries.find(
    (e) => e.id === props.panel.owner,
  );
  const declared = entry?.manifest?.contributes?.views?.find(
    (v) => v.id === props.panel.viewType,
  );
  const compatible =
    declared?.stateVersion === props.panel.stateVersion &&
    declared.placement === (props.placement ?? "central");
  const status = pluginHost.statuses.get(props.panel.owner);
  useEffect(() => {
    if (
      entry?.enabled &&
      declared &&
      compatible &&
      !pluginHost.catalog.safeMode
    )
      void pluginHost.activate(entry.id).catch(() => {});
  }, [
    entry?.id,
    entry?.enabled,
    entry?.revision,
    declared?.stateVersion,
    props.panel.stateVersion,
    compatible,
  ]);
  const View = pluginHost.views.get(props.panel.viewType)?.component;
  return (
    <section
      className="plugin-panel"
      data-plugin-pane-id={props.panel.id}
      tabIndex={-1}
      onFocusCapture={props.onFocus}
      onPointerDownCapture={props.onFocus}
    >
      <header className="editor-heading" data-pane-drag-handle>
        <span className="editor-path">{props.panel.title}</span>
        <Slot name="view-actions" />
        <button
          className="text-button"
          aria-label={`Close ${props.panel.title}`}
          onClick={props.onClose}
        >
          Close
        </button>
      </header>
      {View && entry?.enabled && compatible ? (
        <PluginBoundary key={entry.revision} owner={entry.id}>
          <View {...props} />
        </PluginBoundary>
      ) : (
        <div className="empty-message" role="status">
          <h2>
            {status?.phase === "activating"
              ? "Opening plugin view…"
              : "Plugin view unavailable"}
          </h2>
          <p>
            {error ||
              status?.error ||
              (declared && !compatible
                ? "This view uses an unsupported state version or placement."
                : "Enable the matching plugin to restore this view. Its saved state is preserved.")}
          </p>
          {entry?.enabled && status?.phase === "failed" && (
            <button
              className="button"
              onClick={() => void pluginHost.activate(entry.id).catch(() => {})}
            >
              Retry activation
            </button>
          )}
          <button
            className="button"
            onClick={() =>
              void api("open_settings", { page: "plugins" }).catch((error) =>
                setError(String(error)),
              )
            }
          >
            Open Plugins
          </button>
        </div>
      )}
    </section>
  );
}
