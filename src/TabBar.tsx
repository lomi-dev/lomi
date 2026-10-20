import ResourceIcon from "./ResourceIcon";
import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import type { RefObject } from "react";
import {
  ChevronLeft,
  ChevronRight,
  GitCommitHorizontal,
  FileCode,
  FileDiff,
  Globe,
  Layers,
  MessageSquare,
  Monitor,
  Plus,
  Terminal,
  Puzzle,
  Settings,
  X,
} from "./icons";
import type { Tab, TabCloseAction, TabDropSide } from "./model";
import { tabTitle, layoutPanes } from "./model";
import { useTabDrag } from "./tab-drag";
import { IconButton } from "./ui";
import TabContextMenu from "./TabContextMenu";
import type { TabMenuAnchor } from "./TabContextMenu";
import ContextMenu from "./ContextMenu";
import { SETTINGS_TAB_ID } from "./useSettingsTab";

import {
  chatActivity,
  chatActivityRevision,
  subscribeChatActivity,
} from "./chat/chat-service";

interface Props {
  tabs: Tab[];
  activeTabId: string;
  newTabTitle: string;
  onNew: () => void;
  onNewFile: () => void;
  onNewBrowser: () => void;
  onNewAndroid: () => void;
  onNewChat: () => void;
  onNewAgents: () => void;
  onSelect: (id: string) => void;
  onClose: (id: string, action?: TabCloseAction) => void;
  onRename: (tab: Tab) => void;
  onMove: (id: string, beforeId: string | null) => void;
  onMerge: (id: string, targetId: string, side: TabDropSide) => void;
  mergeContainer: RefObject<HTMLDivElement | null>;
  stripRef?: RefObject<HTMLDivElement | null>;
  modified?: ReadonlySet<string>;
  settings?: SettingsTabControl;
}

export interface SettingsTabControl {
  opened: boolean;
  active: boolean;
  onSelect: () => void;
  onClose: () => void;
}

export function SettingsTabButton({
  active,
  onSelect,
  onClose,
  onNavigate,
}: Omit<SettingsTabControl, "opened"> & {
  onNavigate?: (event: React.KeyboardEvent) => void;
}) {
  return (
    <div
      className={`tab${active ? " active-tab" : ""}`}
      data-tab-id={SETTINGS_TAB_ID}
      role="presentation"
    >
      <button
        id={`tab-${SETTINGS_TAB_ID}`}
        role="tab"
        aria-selected={active}
        aria-controls={`panel-${SETTINGS_TAB_ID}`}
        tabIndex={active ? 0 : -1}
        draggable={false}
        onClick={onSelect}
        onKeyDown={onNavigate}
      >
        <Settings size={14} aria-hidden="true" />
        <span>Settings</span>
      </button>
      <button
        className="tab-close"
        tabIndex={active ? 0 : -1}
        aria-label="Close Settings"
        title="Close Settings"
        onClick={onClose}
      >
        <X size={13} iconId="tab-close" />
      </button>
    </div>
  );
}

export default function TabBar({
  tabs,
  activeTabId,
  newTabTitle,
  onNew,
  onNewFile,
  onNewBrowser,
  onNewAndroid,
  onNewChat,
  onNewAgents,
  onSelect,
  onClose,
  onRename,
  onMove,
  onMerge,
  mergeContainer,
  stripRef,
  modified = new Set<string>(),
  settings,
}: Props) {
  const selectedId = settings?.active ? SETTINGS_TAB_ID : activeTabId;
  const navigationIds = [
    ...tabs.map((tab) => tab.id),
    ...(settings?.opened ? [SETTINGS_TAB_ID] : []),
  ];
  useSyncExternalStore(subscribeChatActivity, chatActivityRevision);
  const activity = (tab: Tab) =>
    (tab.type === "terminal" ? layoutPanes(tab.layout) : [tab])
      .filter((p) => p.type === "chat")
      .map((p) => chatActivity(p.conversationId))
      .find(Boolean);
  const localStrip = useRef<HTMLDivElement>(null);
  const strip = stripRef ?? localStrip;
  const newButton = useRef<HTMLButtonElement>(null);
  const renameCandidate = useRef<string | null>(null);
  const [newMenu, setNewMenu] = useState<{ x: number; y: number } | null>(null);
  const { beginDrag, suppressClick } = useTabDrag({
    tabs,
    activeTabId,
    strip,
    mergeContainer,
    onMove,
    onMerge,
  });
  const [contextMenu, setContextMenu] = useState<TabMenuAnchor | null>(null);
  const dismissMenu = useCallback(() => setContextMenu(null), []);
  const [scroll, setScroll] = useState({ left: false, right: false });
  const updateScroll = useCallback(() => {
    const element = strip.current;
    if (!element) return;
    const left = element.scrollLeft > 1;
    const right =
      element.scrollWidth - element.clientWidth - element.scrollLeft > 1;
    setScroll((previous) =>
      previous.left === left && previous.right === right
        ? previous
        : { left, right },
    );
  }, [strip]);

  useLayoutEffect(() => {
    const element = strip.current!;
    const revealActive = () => {
      element
        .querySelector(".active-tab")
        ?.scrollIntoView({ block: "nearest", inline: "nearest" });
      updateScroll();
    };
    revealActive();
    const observer = new ResizeObserver(revealActive);
    observer.observe(element);
    return () => observer.disconnect();
  }, [selectedId, tabs.length, settings?.opened, updateScroll, strip]);

  const focusTab = (id: string) => {
    if (id === SETTINGS_TAB_ID) settings?.onSelect();
    else onSelect(id);
    requestAnimationFrame(() => document.getElementById(`tab-${id}`)?.focus());
  };
  const navigate = (event: React.KeyboardEvent, id: string) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
      return;
    const index = navigationIds.indexOf(id);
    const next =
      event.key === "Home"
        ? navigationIds[0]
        : event.key === "End"
          ? navigationIds.at(-1)
          : event.key === "ArrowLeft" || event.key === "ArrowRight"
            ? navigationIds[
                (index +
                  (event.key === "ArrowLeft" ? -1 : 1) +
                  navigationIds.length) %
                  navigationIds.length
              ]
            : undefined;
    if (!next) return;
    event.preventDefault();
    focusTab(next);
  };
  const scrollTabs = (direction: number) => {
    const element = strip.current;
    if (element) element.scrollLeft += direction * element.clientWidth * 0.8;
  };

  return (
    <div
      className="tab-bar"
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
      <div
        className={`tab-viewport${scroll.left ? " scrollable-left" : ""}${scroll.right ? " scrollable-right" : ""}`}
      >
        <div
          ref={strip}
          className="tab-strip"
          role="tablist"
          aria-label="Workspace tabs"
          data-tauri-drag-region
          onScroll={updateScroll}
          onWheel={(event) => {
            if (
              !event.ctrlKey &&
              Math.abs(event.deltaY) > Math.abs(event.deltaX)
            ) {
              const unit =
                event.deltaMode === 1
                  ? 32
                  : event.deltaMode === 2
                    ? event.currentTarget.clientWidth
                    : 1;
              event.currentTarget.scrollLeft += event.deltaY * unit;
            }
          }}
        >
          {tabs.map((tab) => (
            <div
              className={`tab${tab.id === selectedId ? " active-tab" : ""}`}
              key={tab.id}
              data-tab-id={tab.id}
              role="presentation"
              onContextMenu={(event) => {
                event.preventDefault();
                const bounds = event.currentTarget.getBoundingClientRect();
                setContextMenu({
                  id: tab.id,
                  x: event.clientX || bounds.left,
                  y: event.clientY || bounds.bottom,
                });
              }}
            >
              <button
                id={`tab-${tab.id}`}
                role="tab"
                aria-selected={tab.id === selectedId}
                aria-controls={`panel-${tab.id}`}
                aria-haspopup="menu"
                aria-expanded={contextMenu?.id === tab.id}
                tabIndex={tab.id === selectedId ? 0 : -1}
                draggable={false}
                onPointerDown={(event) => {
                  dismissMenu();
                  beginDrag(event, tab);
                }}
                title={
                  tab.type === "file"
                    ? `${tab.untitled ? tab.title : tab.relative}${modified?.has(tab.id) ? " • Modified" : ""}`
                    : tabTitle(tab)
                }
                onClick={(event) => {
                  // The first click selects the tab before double-click fires.
                  if (event.detail === 1)
                    renameCandidate.current =
                      tab.id === activeTabId ? tab.id : null;
                  onSelect(tab.id);
                }}
                onDoubleClick={() => {
                  if (
                    tab.type !== "file" &&
                    tab.id === activeTabId &&
                    renameCandidate.current === tab.id
                  )
                    onRename(tab);
                }}
                onKeyDown={(event) => {
                  if (
                    event.key === "ContextMenu" ||
                    (event.shiftKey && event.key === "F10")
                  ) {
                    event.preventDefault();
                    const bounds = event.currentTarget.getBoundingClientRect();
                    setContextMenu({
                      id: tab.id,
                      x: bounds.left,
                      y: bounds.bottom,
                    });
                    return;
                  }
                  if (
                    event.altKey ||
                    event.ctrlKey ||
                    event.metaKey ||
                    event.shiftKey
                  )
                    return;
                  navigate(event, tab.id);
                }}
              >
                {tab.type === "commit" ? (
                  <GitCommitHorizontal size={14} />
                ) : tab.type === "diff" ? (
                  <ResourceIcon
                    path={`${tab.root}/${tab.relative}`}
                    size={14}
                    fallback={FileDiff}
                  />
                ) : tab.type === "browser" ? (
                  <Globe size={14} />
                ) : tab.type === "android" ? (
                  <Monitor size={14} aria-hidden="true" />
                ) : tab.type === "agent-task" ? (
                  <Terminal size={14} aria-hidden="true" />
                ) : tab.type === "chat" ? (
                  <MessageSquare size={14} />
                ) : tab.type === "file" ? (
                  <ResourceIcon
                    path={`${tab.root}/${tab.relative}`}
                    size={14}
                  />
                ) : tab.type === "plugin" ? (
                  <Puzzle size={14} />
                ) : (
                  <Terminal size={14} />
                )}
                <span>{tabTitle(tab)}</span>
                {activity(tab) && (
                  <span
                    className="tab-modified"
                    aria-hidden="true"
                    title={activity(tab)}
                  >
                    {activity(tab) === "Generating"
                      ? "◌"
                      : activity(tab) === "Chat error"
                        ? "!"
                        : "●"}
                  </span>
                )}
                {modified?.has(tab.id) && (
                  <span className="tab-modified" aria-label="Unsaved changes">
                    ●
                  </span>
                )}
              </button>
              <button
                className="tab-close"
                tabIndex={tab.id === selectedId ? 0 : -1}
                aria-label={`Close ${tabTitle(tab)}`}
                title={`Close ${tabTitle(tab)}`}
                onClick={() => onClose(tab.id)}
              >
                <X size={13} iconId="tab-close" />
              </button>
            </div>
          ))}
          {settings?.opened && (
            <SettingsTabButton
              active={settings.active}
              onSelect={settings.onSelect}
              onClose={settings.onClose}
              onNavigate={(event) => navigate(event, SETTINGS_TAB_ID)}
            />
          )}
          <IconButton
            title={newTabTitle}
            aria-haspopup="menu"
            aria-expanded={!!newMenu}
            onClick={(event) => {
              newButton.current = event.currentTarget;
              dismissMenu();
              const bounds = event.currentTarget.getBoundingClientRect();
              setNewMenu({ x: bounds.left, y: bounds.bottom });
            }}
          >
            <Plus size={16} aria-hidden="true" />
          </IconButton>
        </div>
      </div>
      {(scroll.left || scroll.right) && (
        <div className="tab-navigation">
          <IconButton
            title="Scroll tabs left"
            disabled={!scroll.left}
            onClick={() => scrollTabs(-1)}
          >
            <ChevronLeft size={15} />
          </IconButton>
          <IconButton
            title="Scroll tabs right"
            disabled={!scroll.right}
            onClick={() => scrollTabs(1)}
          >
            <ChevronRight size={15} />
          </IconButton>
        </div>
      )}
      {contextMenu && tabs.some((tab) => tab.id === contextMenu.id) && (
        <TabContextMenu
          anchor={contextMenu}
          tabs={tabs}
          modified={modified}
          onClose={onClose}
          onRename={onRename}
          onDismiss={dismissMenu}
        />
      )}
      {newMenu && (
        <ContextMenu
          {...newMenu}
          label="New tab"
          actions={[
            {
              label: "New terminal",
              icon: <Terminal size={14} aria-hidden="true" />,
              run: onNew,
            },
            {
              label: "New file",
              icon: <FileCode size={14} aria-hidden="true" />,
              run: onNewFile,
            },
            {
              label: "Chat AI",
              icon: <MessageSquare size={14} aria-hidden="true" />,
              run: onNewChat,
            },
            {
              label: "Agents",
              icon: <Layers size={14} aria-hidden="true" />,
              run: onNewAgents,
            },
            {
              label: "New browser",
              icon: <Globe size={14} aria-hidden="true" />,
              run: onNewBrowser,
            },
            {
              label: "New android symulator",
              icon: <Monitor size={14} aria-hidden="true" />,
              run: onNewAndroid,
            },
          ]}
          onClose={() => {
            setNewMenu(null);
            newButton.current?.focus({ preventScroll: true });
          }}
        />
      )}
    </div>
  );
}
