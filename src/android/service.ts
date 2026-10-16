import {
  active,
  androidTabs,
  layoutPanes,
  newAndroidTab,
  updateAndroid,
  type AndroidTab,
  type Session,
} from "../model";
import { api } from "../api";
import { reportAppNotification } from "../app-notifications";

let retained: AndroidTab[] = [];
let runtime: typeof import("./runtime") | undefined;
let loading: Promise<typeof import("./runtime")> | undefined;
let handlers:
  | {
      change: (
        id: string,
        change: Partial<Pick<AndroidTab, "deviceId" | "title">>,
      ) => void;
      session: () => Session | undefined;
      commit: (update: (session: Session) => Session) => void;
      blocked: () => boolean;
    }
  | undefined;

export function configureAndroid(value: NonNullable<typeof handlers>) {
  handlers = value;
}
export function changeAndroid(
  id: string,
  change: Partial<Pick<AndroidTab, "deviceId" | "title">>,
) {
  handlers?.change(id, change);
}
export function reportAndroidError(message: string) {
  if (message)
    void reportAppNotification({
      source: "android",
      body: message,
      action: "open-android-settings",
    });
}
export function retainAndroid(session?: Session) {
  retained = androidTabs(session);
  runtime?.retain(retained);
}
export function androidRuntime() {
  return (loading ??= import("./runtime").then((module) => {
    runtime = module;
    module.retain(retained);
    return module;
  }));
}
export function retainedAndroid() {
  return retained;
}

interface SetupContext {
  requestId: string;
  workspaceId: string;
  panelId: string;
}
export interface OpenIntent {
  id: string;
  context: SetupContext | null;
  deviceId: string;
  coldBoot: boolean;
  deadlineMs: number;
}
const setups = new Map<string, SetupContext>();
const received = new Set<string>();
export async function openAndroidSettings(panelId: string) {
  const session = handlers?.session();
  const workspace = session?.projects
    .flatMap((project) => project.workspaces)
    .find((workspace) =>
      workspace.tabs.some((tab) =>
        (tab.type === "terminal" ? layoutPanes(tab.layout) : [tab]).some(
          (panel) => panel.type === "android" && panel.id === panelId,
        ),
      ),
    );
  if (!workspace || handlers?.blocked())
    throw new Error("The Android setup target is no longer available.");
  const context = await api<SetupContext>("android_prepare_setup", {
    workspaceId: workspace.id,
    panelId,
  });
  setups.set(context.requestId, context);
  for (const [id, target] of setups)
    if (target.panelId !== panelId) setups.delete(id);
  await api("open_settings", { page: "android" });
}
export async function receiveAndroidOpen(intent: OpenIntent) {
  if (received.has(intent.id)) return;
  received.add(intent.id);
  if (received.size > 128) received.delete(received.values().next().value!);
  let failure: string | null = null;
  try {
    const target = intent.context && setups.get(intent.context.requestId);
    if (
      intent.context &&
      (!target ||
        target.panelId !== intent.context.panelId ||
        target.workspaceId !== intent.context.workspaceId)
    )
      throw new Error(
        "This setup request is no longer available. Open a new Android tab explicitly.",
      );
    const module = await androidRuntime();
    const { refreshAndroid, androidSnapshot } = await import("./state");
    await refreshAndroid();
    const device = androidSnapshot()?.devices?.devices.find(
      (device) => device.id === intent.deviceId,
    );
    if (!device)
      throw new Error(
        "This device is no longer available. Refresh Android settings.",
      );
    const session = handlers?.session();
    if (
      !session ||
      !handlers ||
      handlers.blocked() ||
      Date.now() >= intent.deadlineMs
    )
      throw new Error(
        "The workspace is unavailable or preparing to close. Retry from the main window.",
      );
    const workspaceId = target?.workspaceId ?? active(session)?.workspace.id;
    const project = session.projects.find((project) =>
      project.workspaces.some((workspace) => workspace.id === workspaceId),
    );
    const workspace = project?.workspaces.find(
      (workspace) => workspace.id === workspaceId,
    );
    if (!project || !workspace)
      throw new Error(
        "The requested workspace was closed. Choose a workspace in the main window, then explicitly open a new Android tab.",
      );
    const parent =
      target &&
      workspace.tabs.find((tab) =>
        (tab.type === "terminal" ? layoutPanes(tab.layout) : [tab]).some(
          (panel) => panel.id === target.panelId && panel.type === "android",
        ),
      );
    if (target && !parent)
      throw new Error(
        "The waiting Android panel was closed. Use Open in new tab to create another one.",
      );
    const previous =
      target && androidTabs(session).find((tab) => tab.id === target.panelId);
    if (previous?.deviceId && previous.deviceId !== device.id)
      throw new Error(
        "This panel already uses another device. Use Open in new tab to keep both phones.",
      );
    const added = target ? null : newAndroidTab(device.id, device.name);
    // Mark a requested restart before rendering so a first mount cannot race it.
    const restarting = intent.coldBoot ? module.restart(device.id) : undefined;
    handlers.commit((current) => {
      const next = target
        ? updateAndroid(current, target.panelId, {
            deviceId: device.id,
            title: device.name,
          })
        : current;
      return {
        ...next,
        activeProjectId: project.id,
        projects: next.projects.map((item) =>
          item.id !== project.id
            ? item
            : {
                ...item,
                activeWorkspaceId: workspace.id,
                workspaces: item.workspaces.map((item) =>
                  item.id !== workspace.id
                    ? item
                    : {
                        ...item,
                        activeTabId: parent?.id ?? added!.id,
                        tabs: added
                          ? [...item.tabs, added]
                          : item.tabs.map((tab) =>
                              tab.type === "terminal" && tab.id === parent?.id
                                ? { ...tab, activePaneId: target!.panelId }
                                : tab,
                            ),
                      },
                ),
              },
        ),
      };
    });
    void (restarting ?? module.start(device.id)).catch((error) =>
      reportAndroidError(String(error)),
    );
  } catch (error) {
    failure = error instanceof Error ? error.message : String(error);
  }
  await api("android_open_result", { id: intent.id, error: failure });
}

/** Called after existing close guards and before removing descriptors. */
export async function stopLastAndroidViews(
  ids: ReadonlySet<string>,
  current: () => Session | undefined,
) {
  const before = androidTabs(current());
  const targets = new Set(
    before
      .filter((view) => ids.has(view.id))
      .map((view) => view.deviceId)
      .filter((id): id is string => !!id),
  );
  if (!targets.size || !runtime) return;
  for (const deviceId of targets) {
    const views = androidTabs(current()).filter(
      (view) => view.deviceId === deviceId,
    );
    if (views.length && views.every((view) => ids.has(view.id)))
      await runtime.stop(deviceId);
  }
}
