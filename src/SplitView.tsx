import { builtinViews } from "./plugins/builtins";
import {
  Suspense,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  layoutFits,
  MIN_PANE_HEIGHT,
  MIN_PANE_WIDTH,
  layoutPanes,
  layoutPositions,
  SPLIT_DIVIDER_SIZE,
  splitGeometry,
} from "./model";
import type {
  EditorPosition,
  Layout,
  LayoutBounds,
  LayoutSize,
  FilePreviewView,
  ShellProfile,
  Split,
  TabDropSide,
} from "./model";
import PluginPanel from "./plugins/PluginPanel";
import type { Json } from "@lomi-dev/plugin-sdk";
import DockviewLayout from "./DockviewLayout";
import { usePaneDrag } from "./pane-drag";

interface Props {
  layout: Layout;
  profile?: ShellProfile;
  profiles: ShellProfile[];
  activePaneId: string;
  overview: boolean;
  revealTitles: boolean;
  onFocus: (id: string) => void;
  onPluginState: (id: string, state: Json) => void;
  onRestart: (id: string, useProjectDirectory?: boolean) => void;
  onResize: (id: string, ratio: number) => void;
  onMove: (id: string, targetId: string, side: TabDropSide) => void;
  onFilePosition: (id: string, position: EditorPosition) => void;
  onPreviewView: (id: string, view: FilePreviewView) => void;
  onOpenFile: (root: string, relative: string) => void;
  onClosePane: (id: string) => void;
}

export default function SplitView({
  onKeepActivePane,
  ...props
}: Props & { onKeepActivePane: () => void }) {
  const root = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<LayoutSize>();
  const [maximizedPaneId, setMaximizedPaneId] = useState<string | null>(null);
  const allPanes = layoutPanes(props.layout);
  const maximizedPane = allPanes.find(
    (pane) =>
      pane.type === "terminal" &&
      pane.id === maximizedPaneId &&
      pane.id === props.activePaneId,
  );
  const { beginDrag, controlHeld, suppressClick } = usePaneDrag({
    layout: props.layout,
    root,
    enabled: allPanes.length > 1 && !props.overview && !maximizedPane,
    onMove: props.onMove,
  });
  useEffect(() => {
    if (!maximizedPane) setMaximizedPaneId(null);
  }, [maximizedPane]);
  const measure = useCallback(() => {
    const container = root.current!;
    const width = container.clientWidth;
    const height = container.clientHeight;
    setSize((previous) =>
      previous?.width === width && previous.height === height
        ? previous
        : { width, height },
    );
  }, []);
  // Commit sidebar size changes before pane motion reads final geometry.
  useLayoutEffect(measure);
  useLayoutEffect(() => {
    const observer = new ResizeObserver(measure);
    observer.observe(root.current!);
    return () => observer.disconnect();
  }, [measure]);
  const visibleLayout = props.overview
    ? props.layout
    : (maximizedPane ?? props.layout);
  const positions = useMemo(
    () =>
      size &&
      (visibleLayout.type !== "split" || layoutFits(visibleLayout, size))
        ? layoutPositions(visibleLayout, size)
        : null,
    [visibleLayout, size],
  );
  return (
    <div
      className={`split-container${props.layout.type === "split" ? " is-split" : ""}`}
      ref={root}
      onPointerDown={beginDrag}
      onPointerDownCapture={(event) => {
        if (event.isPrimary) suppressClick.current = false;
      }}
      onClickCapture={(event) => {
        if (suppressClick.current && event.detail !== 0) {
          event.preventDefault();
          event.stopPropagation();
          suppressClick.current = false;
        }
      }}
    >
      {size &&
        size.width > 0 &&
        size.height > 0 &&
        (positions ? (
          <>
            <DockviewLayout
              layout={visibleLayout}
              size={size}
              activePaneId={props.activePaneId}
              onResize={props.onResize}
              render={(id) => {
                const layout = allPanes.find((pane) => pane.id === id)!;
                return layout.type === "terminal" ? (
                  <div key={layout.id} className="split-child">
                    <builtinViews.terminal
                      pane={layout}
                      profile={
                        layout.profileId !== undefined
                          ? props.profiles.find(
                              (profile) => profile.id === layout.profileId,
                            )
                          : props.profile
                      }
                      active={props.activePaneId === layout.id}
                      overview={props.overview}
                      revealTitle={props.revealTitles}
                      canMove={controlHeld}
                      canMaximize={props.layout.type === "split"}
                      maximized={maximizedPane?.id === layout.id}
                      onToggleMaximize={() => {
                        props.onFocus(layout.id);
                        setMaximizedPaneId(maximizedPane ? null : layout.id);
                      }}
                      onFocus={() => props.onFocus(layout.id)}
                      onRestart={(useProjectDirectory) =>
                        props.onRestart(layout.id, useProjectDirectory)
                      }
                    />
                  </div>
                ) : layout.type === "browser" ? (
                  <div key={layout.id} className="split-child">
                    <builtinViews.browser
                      tab={layout}
                      overview={props.overview}
                      onFocus={() => props.onFocus(layout.id)}
                      onClose={() => props.onClosePane(layout.id)}
                    />
                  </div>
                ) : layout.type === "android" ? (
                  <div key={layout.id} className="split-child">
                    <Suspense
                      fallback={<div role="status">Loading Android…</div>}
                    >
                      <builtinViews.android
                        tab={layout}
                        overview={props.overview}
                        onFocus={() => props.onFocus(layout.id)}
                        onClose={() => props.onClosePane(layout.id)}
                      />
                    </Suspense>
                  </div>
                ) : layout.type === "chat" ? (
                  <div key={layout.id} className="split-child">
                    <Suspense
                      fallback={<div role="status">Loading conversation…</div>}
                    >
                      <builtinViews.chat
                        tab={layout}
                        onFocus={() => props.onFocus(layout.id)}
                        onClose={() => props.onClosePane(layout.id)}
                      />
                    </Suspense>
                  </div>
                ) : layout.type === "plugin" ? (
                  <div className="split-child">
                    <PluginPanel
                      panel={layout}
                      active={props.activePaneId === layout.id}
                      setState={(state) =>
                        props.onPluginState(layout.id, state)
                      }
                      onFocus={() => props.onFocus(layout.id)}
                      onClose={() => props.onClosePane(layout.id)}
                    />
                  </div>
                ) : (
                  <div
                    key={layout.id}
                    className="split-child"

                    data-file-pane-id={layout.id}
                    onPointerDownCapture={() => props.onFocus(layout.id)}
                    onFocusCapture={() => props.onFocus(layout.id)}
                  >
                    <Suspense
                      fallback={
                        <div className="empty-message" role="status">
                          Loading editor…
                        </div>
                      }
                    >
                      <builtinViews.file
                        tab={layout}
                        onOpenFile={props.onOpenFile}
                        onPreviewView={(view) =>
                          props.onPreviewView(layout.id, view)
                        }
                        active={props.activePaneId === layout.id}
                        onClose={() => props.onClosePane(layout.id)}
                        onPosition={(position) =>
                          props.onFilePosition(layout.id, position)
                        }
                      />
                    </Suspense>
                  </div>
                );
              }}
            />
            {positions
              .filter((position) => position.layout.type === "split")
              .map(({ layout, bounds }) => (
                <Divider
                  key={layout.id}
                  layout={layout as Split}
                  bounds={bounds}
                  onResize={props.onResize}
                />
              ))}
          </>
        ) : (
          <div className="layout-recovery" role="status">
            <h2>This panel layout needs more space</h2>
            <p>
              This tab has {layoutPanes(props.layout).length} panels. Each panel
              needs at least {MIN_PANE_WIDTH} × {MIN_PANE_HEIGHT} pixels.
              Enlarge the window or hide the sidebar to show them.
            </p>
            <p>
              Your layout is preserved. Existing shells keep running; saved
              terminals will start when the layout fits.
            </p>
            <button className="button" onClick={onKeepActivePane}>
              Keep only the active panel
            </button>
            <p className="muted">
              This closes the other {layoutPanes(props.layout).length - 1}{" "}
              panels in this tab.
            </p>
          </div>
        ))}
    </div>
  );
}

function Divider({
  layout,
  bounds,
  onResize,
}: {
  layout: Split;
  bounds: LayoutBounds;
  onResize: Props["onResize"];
}) {
  const horizontal = layout.axis === "horizontal";
  const geometry = splitGeometry(layout, bounds);
  const resize = (ratio: number) =>
    onResize(
      layout.id,
      Math.max(geometry.minRatio, Math.min(geometry.maxRatio, ratio)),
    );
  return (
    <div
      className="split-divider"
      style={
        horizontal
          ? {
              left: bounds.left + geometry.first.width,
              top: bounds.top,
              width: SPLIT_DIVIDER_SIZE,
              height: bounds.height,
            }
          : {
              left: bounds.left,
              top: bounds.top + geometry.first.height,
              width: bounds.width,
              height: SPLIT_DIVIDER_SIZE,
            }
      }
      role="separator"
      aria-label={
        horizontal ? "Resize terminal columns" : "Resize terminal rows"
      }
      aria-orientation={horizontal ? "vertical" : "horizontal"}
      aria-valuemin={Math.round(geometry.minRatio * 100)}
      aria-valuemax={Math.round(geometry.maxRatio * 100)}
      aria-valuenow={Math.round(geometry.ratio * 100)}
      tabIndex={0}
      onDoubleClick={() => resize(0.5)}
      onKeyDown={(event) => {
        if (
          ["ArrowLeft", "ArrowUp", "ArrowRight", "ArrowDown"].includes(
            event.key,
          )
        ) {
          event.preventDefault();
          resize(
            geometry.ratio +
              (["ArrowLeft", "ArrowUp"].includes(event.key) ? -0.05 : 0.05),
          );
        }
      }}
      onPointerDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId)) return;
        const origin =
          event.currentTarget.parentElement!.getBoundingClientRect();
        resize(
          horizontal
            ? (event.clientX -
                origin.left -
                bounds.left -
                SPLIT_DIVIDER_SIZE / 2) /
                (bounds.width - SPLIT_DIVIDER_SIZE)
            : (event.clientY -
                origin.top -
                bounds.top -
                SPLIT_DIVIDER_SIZE / 2) /
                (bounds.height - SPLIT_DIVIDER_SIZE),
        );
      }}
      onPointerUp={(event) => {
        if (event.currentTarget.hasPointerCapture(event.pointerId))
          event.currentTarget.releasePointerCapture(event.pointerId);
      }}
    />
  );
}
