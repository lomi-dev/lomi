import { useEffect, useRef, useState } from "react";
import { native } from "./api";
import type { AppInfo, Session, ShellProfile } from "./model";
import {
  importSharedWorkspace,
  setSharedHostWorkspaceSharing,
  sharedHostStatus,
  sharedHostWorkspaceBindings,
  sharedHostWorkspaces,
} from "./shared-host";
import type { SharedHostDesktopUncertaintyWarning } from "./shared-host";
import type { RemoteState } from "./remote-workspace-domain";
export type { RemoteState } from "./remote-workspace-domain";

export function useRemoteWorkspaces(
  _session: Session | undefined,
  current: { current: Session | undefined },
  _profiles: ShellProfile[] | undefined,
  relocateWorkspaceRoot: (
    workspaceId: string,
    previousRootPath: string,
    rootPath: string,
  ) => void | Promise<void>,
  workspaceBackend: AppInfo["workspaceBackend"] | undefined,
) {
  const [state, setState] = useState<RemoteState>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState<string>();
  const [uncertaintyWarnings, setUncertaintyWarnings] = useState<
    SharedHostDesktopUncertaintyWarning[]
  >([]);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const available = native && workspaceBackend === "shared-host";
  const disabledReason = !native
    ? "Remote sharing requires the Lomi desktop app."
    : workspaceBackend !== "shared-host"
      ? "Remote is disabled in this version of Lomi."
      : "Remote sharing status is unavailable.";

  useEffect(() => {
    if (!available) {
      setState(undefined);
      setError("");
      setBusy(undefined);
      setUncertaintyWarnings([]);
      refreshRef.current = async () => {};
      return;
    }
    let alive = true;
    let refreshing = false;
    const refresh = async () => {
      if (refreshing) return;
      refreshing = true;
      try {
        const workspaceIds =
          current.current?.projects.flatMap((project) =>
            project.workspaces.map((workspace) => workspace.id),
          ) ?? [];
        let bindingResponse = await sharedHostWorkspaceBindings(workspaceIds);
        if (!alive) return;
        setUncertaintyWarnings(bindingResponse.uncertaintyWarnings);
        if (!bindingResponse.bindings)
          throw new Error(
            bindingResponse.bindingError ??
              "The local Host workspace bindings are unavailable.",
          );
        let bindingList = bindingResponse.bindings;
        const projectRoots = new Map(
          current.current?.projects.flatMap((project) =>
            project.workspaces.map(
              (workspace) =>
                [
                  workspace.id,
                  { projectId: project.id, path: project.path },
                ] as const,
            ),
          ) ?? [],
        );
        const relocatedProjects = new Set<string>();
        for (const binding of bindingList) {
          if (binding.state === "active") {
            const project = projectRoots.get(binding.workspaceId);
            if (!project || project.path === binding.rootPath) continue;
            const relocationKey = JSON.stringify([
              project.projectId,
              project.path,
              binding.rootPath,
            ]);
            if (relocatedProjects.has(relocationKey)) continue;
            relocatedProjects.add(relocationKey);
            await relocateWorkspaceRoot(
              binding.workspaceId,
              project.path,
              binding.rootPath,
            );
          }
        }
        const activeProject = current.current?.projects.find(
          (project) => project.id === current.current?.activeProjectId,
        );
        const activeWorkspace = activeProject?.workspaces.find(
          (workspace) => workspace.id === activeProject.activeWorkspaceId,
        );
        const activeBinding = bindingList.find(
          (binding) => binding.workspaceId === activeWorkspace?.id,
        );
        if (
          activeProject &&
          activeWorkspace &&
          activeBinding?.state === "missing" &&
          !activeBinding.previousRootPath
        ) {
          await importSharedWorkspace(
            activeWorkspace.id,
            activeWorkspace.name,
            activeProject.path,
          );
          bindingResponse = await sharedHostWorkspaceBindings(
            current.current?.projects.flatMap((project) =>
              project.workspaces.map((workspace) => workspace.id),
            ) ?? [],
          );
          setUncertaintyWarnings(bindingResponse.uncertaintyWarnings);
          if (!bindingResponse.bindings)
            throw new Error(
              bindingResponse.bindingError ??
                "The local Host workspace bindings are unavailable.",
            );
          bindingList = bindingResponse.bindings;
        }
        const localByCanonical = new Map(
          bindingList.flatMap((binding) =>
            binding.state === "active"
              ? [[binding.canonicalWorkspaceId, binding.workspaceId] as const]
              : [],
          ),
        );
        const [hostStatus, hostWorkspaces] = await Promise.all([
          sharedHostStatus(),
          sharedHostWorkspaces(),
        ]);
        if (!alive) return;
        setUncertaintyWarnings(bindingResponse.uncertaintyWarnings);
        setState({
          qualified: true,
          enabled: hostStatus.sharing === "enabled",
          message: null,
          workspaces: hostWorkspaces.map((workspace) => ({
            id:
              localByCanonical.get(workspace.workspaceId) ??
              workspace.workspaceId,
            name: workspace.name,
            shared: workspace.sharing === "enabled",
            message: null,
          })),
        });
        setError("");
      } catch (cause) {
        if (alive) {
          setState(undefined);
          setError(cause instanceof Error ? cause.message : String(cause));
        }
      } finally {
        refreshing = false;
      }
    };
    refreshRef.current = refresh;
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => {
      alive = false;
      clearInterval(timer);
      if (refreshRef.current === refresh) refreshRef.current = async () => {};
    };
  }, [available, current, _session, relocateWorkspaceRoot]);

  const share = async (workspaceId: string, shared: boolean) => {
    if (!available) return;
    const session = current.current;
    const project = session?.projects.find((candidate) =>
      candidate.workspaces.some((workspace) => workspace.id === workspaceId),
    );
    const workspace = project?.workspaces.find(
      (candidate) => candidate.id === workspaceId,
    );
    if (!project || !workspace) {
      setError("The workspace was closed.");
      return;
    }
    setBusy(workspaceId);
    setError("");
    try {
      const bindings = await sharedHostWorkspaceBindings([workspace.id]);
      setUncertaintyWarnings(bindings.uncertaintyWarnings);
      if (!bindings.bindings)
        throw new Error(
          bindings.bindingError ??
            "The local Host workspace bindings are unavailable.",
        );
      const binding = bindings.bindings.find(
        (candidate) => candidate.workspaceId === workspace.id,
      );
      if (!binding)
        throw new Error(
          "The workspace has no authoritative local Host binding.",
        );
      if (binding.state === "retired")
        throw new Error(
          "The local Host retired this workspace. Its local project and drafts are preserved.",
        );
      if (binding.state === "pending")
        throw new Error(
          "The local Host is still resolving this workspace binding.",
        );
      if (binding.state === "missing") {
        if (binding.previousRootPath)
          throw new Error(
            "The local Host no longer has this imported workspace. Its local project and drafts are preserved.",
          );
        await importSharedWorkspace(workspace.id, workspace.name, project.path);
      }
      await setSharedHostWorkspaceSharing(workspace.id, shared);
      await refreshRef.current();
    } catch (cause) {
      setState(undefined);
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(undefined);
    }
  };

  const acknowledgeUncertaintyWarning = async (
    warning: SharedHostDesktopUncertaintyWarning,
  ) => {
    if (!available) return;
    try {
      const workspaceIds =
        current.current?.projects.flatMap((project) =>
          project.workspaces.map((workspace) => workspace.id),
        ) ?? [];
      const response = await sharedHostWorkspaceBindings(workspaceIds, {
        operationId: warning.operationId,
        requestHash: warning.requestHash,
      });
      setUncertaintyWarnings(response.uncertaintyWarnings);
      if (response.bindings) await refreshRef.current();
      else {
        setState(undefined);
        setError(response.bindingError ?? "The local Host is unavailable.");
      }
    } catch (cause) {
      setState(undefined);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return {
    state: available ? state : undefined,
    error: available ? error : "",
    busy: available ? busy : undefined,
    share,
    uncertaintyWarnings: available ? uncertaintyWarnings : [],
    acknowledgeUncertaintyWarning,
    available,
    disabledReason,
  };
}
