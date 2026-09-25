import { useLayoutEffect, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { createDockview, type DockviewApi } from "dockview";
import { dockviewLayout, dockviewGapShares } from "./dockview-layout";
import {
  layoutPositions,
  layoutPanes,
  type Layout,
  type LayoutSize,
} from "./model";
import "dockview/dist/styles/dockview.css";
interface Props {
  layout: Layout;
  size: LayoutSize;
  activePaneId: string;
  render: (id: string) => ReactNode;
  onResize: (id: string, ratio: number) => void;
}
export default function DockviewLayout(props: Props) {
  const container = useRef<HTMLDivElement>(null);
  const dock = useRef<DockviewApi | null>(null);
  const hosts = useRef(new Map<string, HTMLElement>());
  const updating = useRef(false);
  const restoreFocus = useRef<HTMLElement | null>(null);
  const current = useRef(props);
  current.current = props;
  const panes = layoutPanes(props.layout);
  const singlePaneId = props.layout.type === "split" ? null : props.layout.id;
  for (const pane of panes)
    if (!hosts.current.has(pane.id)) {
      const host = document.createElement("div");
      host.className = "dock-pane-host";
      hosts.current.set(pane.id, host);
    }
  const topology = JSON.stringify(props.layout, (key, value) =>
    [
      "ratio",
      "cwd",
      "profileId",
      "position",
      "previewView",
      "previewRatio",
      "title",
      "customTitle",
      "url",
      "deviceId",
      "state",
    ].includes(key)
      ? undefined
      : value,
  );
  useLayoutEffect(() => {
    const rememberFocus = () => {
      if (
        document.activeElement instanceof HTMLElement &&
        container.current?.contains(document.activeElement)
      )
        restoreFocus.current = document.activeElement;
    };
    // Dockview's outer grid has a fixed 100px minimum, even with no splits.
    // Keep the same portal host while letting a single pane fill smaller areas.
    if (singlePaneId) {
      const host = hosts.current.get(singlePaneId)!;
      container.current!.appendChild(host);
      return () => {
        rememberFocus();
        host.remove();
      };
    }
    const api = createDockview(container.current!, {
      disableDnd: true,
      disableFloatingGroups: true,
      theme: {
        name: "lomi",
        className: "dockview-theme-lomi",
        gap: 3,
      },
      createComponent: ({ id }) => ({
        element: hosts.current.get(id)!,
        init() {},
      }),
    });
    dock.current = api;
    const measure = () => {
      if (updating.current) return;
      const visit = (layout: Layout): DOMRect | undefined => {
        if (layout.type !== "split")
          return hosts.current.get(layout.id)?.getBoundingClientRect();
        const first = visit(layout.first),
          second = visit(layout.second);
        if (!first || !second) return;
        const horizontal = layout.axis === "horizontal";
        const length = horizontal
          ? first.width + second.width
          : first.height + second.height;
        const ratio = (horizontal ? first.width : first.height) / length;
        if (
          length > 0 &&
          Number.isFinite(ratio) &&
          Math.abs(ratio - layout.ratio) > 0.002
        )
          current.current.onResize(layout.id, ratio);
        return new DOMRect(
          Math.min(first.x, second.x),
          Math.min(first.y, second.y),
          Math.max(first.right, second.right) - Math.min(first.x, second.x),
          Math.max(first.bottom, second.bottom) - Math.min(first.y, second.y),
        );
      };
      visit(current.current.layout);
    };
    const changes = api.onDidLayoutChange(measure);
    return () => {
      rememberFocus();
      changes.dispose();
      api.dispose();
      dock.current = null;
    };
  }, [singlePaneId]);
  useLayoutEffect(() => {
    updating.current = true;
    const focused =
      document.activeElement instanceof HTMLElement &&
      hosts.current.get(props.activePaneId)?.contains(document.activeElement)
        ? document.activeElement
        : hosts.current.get(props.activePaneId)?.contains(restoreFocus.current)
          ? restoreFocus.current
          : null;
    restoreFocus.current = null;
    try {
      dock.current?.fromJSON(
        dockviewLayout(props.layout, props.size, props.activePaneId),
      );
    } finally {
      updating.current = false;
    }
    if (focused?.isConnected && document.activeElement === document.body)
      focused.focus({ preventScroll: true });
    for (const id of hosts.current.keys())
      if (!panes.some((p) => p.id === id)) hosts.current.delete(id);
  }, [topology]);
  useLayoutEffect(() => {
    updating.current = true;
    try {
      dock.current?.layout(props.size.width, props.size.height);
    } finally {
      updating.current = false;
    }
  }, [singlePaneId, props.size.width, props.size.height]);
  useLayoutEffect(() => {
    if (!dock.current) return;
    updating.current = true;
    const gaps = dockviewGapShares(
      dockviewLayout(props.layout, props.size, props.activePaneId),
    );
    try {
      for (const { layout, bounds } of layoutPositions(
        props.layout,
        props.size,
      )) {
        if (layout.type === "split") continue;
        const group = dock.current!.getPanel(layout.id)?.group;
        const rect = hosts.current.get(layout.id)?.getBoundingClientRect();
        if (
          group &&
          rect &&
          (Math.abs(rect.width - bounds.width) > 1 ||
            Math.abs(rect.height - bounds.height) > 1)
        )
          group.api.setSize({
            width: bounds.width + gaps.get(layout.id)!.width,
            height: bounds.height + gaps.get(layout.id)!.height,
          });
      }
    } finally {
      updating.current = false;
    }
  }, [props.layout, props.size]);
  return (
    <>
      <div className="dock-layout" ref={container} />
      {panes.map((pane) =>
        createPortal(
          props.render(pane.id),
          hosts.current.get(pane.id)!,
          pane.id,
        ),
      )}
    </>
  );
}
