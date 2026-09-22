import { useCallback, useEffect, useRef, useState } from "react";
import { api, errorMessage, native } from "./api";
import type { GitRepositoryScan } from "./api";
import { containsPath, gitFilePath } from "./explorer-model";

export default function useGit(root: string) {
  const cache = useRef<Record<string, GitRepositoryScan>>({});
  const [, render] = useState(0);
  const request = useRef<() => void>(() => {});
  const refresh = useCallback(() => request.current(), []);
  useEffect(() => {
    if (!root || !native) return;
    let current = true;
    let busy = false;
    let queued = false;
    let discoveredAt = 0;
    const update = async (discover = false) => {
      if (document.visibilityState === "hidden") return;
      if (busy) {
        queued ||= discover;
        return;
      }
      busy = true;
      const previous = cache.current[root];
      const full = discover || !previous || Date.now() - discoveredAt >= 60_000;
      try {
        const result = await api<GitRepositoryScan>("git_repositories", {
          root,
          knownRoots: full
            ? undefined
            : previous.repositories.map((repo) => repo.root),
        });
        if (!current) return;
        const retained = (previous?.repositories ?? []).filter(
          (repo) =>
            !result.repositories.some((next) => next.root === repo.root) &&
            (result.limited ||
              result.errors.some((error) =>
                containsPath(gitFilePath(error.root), gitFilePath(repo.root)),
              )),
        );
        cache.current[root] = {
          ...result,
          repositories: [...result.repositories, ...retained].sort((a, b) =>
            a.root.localeCompare(b.root),
          ),
          // A status-only refresh cannot resolve a discovery failure or scan limit.
          errors: full
            ? result.errors
            : [
                ...(previous?.errors ?? []).filter(
                  (error) =>
                    !previous?.repositories.some(
                      (repo) => repo.root === error.root,
                    ),
                ),
                ...result.errors,
              ],
          limited: full
            ? result.limited
            : (previous?.limited ?? result.limited),
        };
        if (full) discoveredAt = Date.now();
      } catch (error) {
        if (!current) return;
        cache.current[root] = {
          repositories: previous?.repositories ?? [],
          limited: previous?.limited ?? false,
          errors: [{ root, message: errorMessage(error) }],
        };
      } finally {
        busy = false;
        if (current) {
          render((value) => value + 1);
          if (queued) {
            queued = false;
            void update(true);
          }
        }
      }
    };
    const discover = () => void update(true);
    request.current = discover;
    discover();
    const timer = setInterval(() => void update(), 4000);
    window.addEventListener("focus", discover);
    return () => {
      current = false;
      clearInterval(timer);
      window.removeEventListener("focus", discover);
    };
  }, [root]);
  const result = cache.current[root];
  return {
    repositories: result?.repositories ?? [],
    errors: result?.errors ?? [],
    limited: result?.limited ?? false,
    loading: !!root && !result,
    refresh,
  };
}
