import { Orientation, type SerializedDockview } from "dockview";
import {
  layoutPanes,
  splitGeometry,
  paneMinimumSize,
  SPLIT_DIVIDER_SIZE,
  type Layout,
  type LayoutSize,
} from "./model.ts";
type Node = SerializedDockview["grid"]["root"];
type Axis = "horizontal" | "vertical";
export function dockviewLayout(
  layout: Layout,
  size: LayoutSize,
  active: string,
): SerializedDockview {
  const axis = layout.type === "split" ? layout.axis : "horizontal";
  const branch = (
    layout: Layout,
    bounds: LayoutSize,
    direction: Axis,
  ): Node => {
    const collect = (node: Layout, bounds: LayoutSize): Node[] => {
      const length = direction === "horizontal" ? bounds.width : bounds.height;
      if (node.type !== "split")
        return [
          {
            type: "leaf",
            size: length,
            data: {
              id: node.id,
              views: [node.id],
              activeView: node.id,
              hideHeader: true,
              locked: "no-drop-target",
            },
          },
        ];
      if (node.axis !== direction)
        return [{ ...branch(node, bounds, node.axis), size: length }];
      const geometry = splitGeometry(node, bounds);
      return [
        ...collect(node.first, geometry.first),
        ...collect(node.second, geometry.second),
      ];
    };
    const data = collect(layout, bounds);
    // Dockview serializes pre-gap sizes, distributing the total gap equally.
    const gap = (SPLIT_DIVIDER_SIZE * (data.length - 1)) / data.length;
    return {
      type: "branch",
      data: data.map((node) => ({ ...node, size: (node.size ?? 0) + gap })),
    };
  };
  return {
    grid: {
      root: branch(layout, size, axis),
      width: size.width,
      height: size.height,
      orientation:
        axis === "horizontal" ? Orientation.HORIZONTAL : Orientation.VERTICAL,
    },
    panels: Object.fromEntries(
      layoutPanes(layout).map((pane) => [
        pane.id,
        {
          id: pane.id,
          contentComponent: pane.type,
          minimumWidth: paneMinimumSize(pane).width,
          minimumHeight: paneMinimumSize(pane).height,
          renderer: "onlyWhenVisible",
        },
      ]),
    ),
    activeGroup: active,
  };
}
export function dockviewGapShares(state: SerializedDockview) {
  const result = new Map<string, LayoutSize>();
  const visit = (node: Node, axis: Axis, gaps: LayoutSize) => {
    if (!Array.isArray(node.data)) {
      result.set(node.data.id, gaps);
      return;
    }
    const next = {
      ...gaps,
      [axis === "horizontal" ? "width" : "height"]:
        (SPLIT_DIVIDER_SIZE * (node.data.length - 1)) / node.data.length,
    };
    for (const child of node.data)
      visit(child, axis === "horizontal" ? "vertical" : "horizontal", next);
  };
  visit(
    state.grid.root,
    state.grid.orientation === Orientation.HORIZONTAL
      ? "horizontal"
      : "vertical",
    { width: 0, height: 0 },
  );
  return result;
}
