import type { IconKind } from "./theme/icon-theme";
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { watch } from "@tauri-apps/plugin-fs";
import { api, errorMessage, native } from "./api";
import { opaqueWindowBackground } from "./window-background";
import {
  builtinPreferences,
  type Appearance,
  type AppearancePreference,
  type ThemeBundle,
  type ThemeCurrent,
  type ThemePreferences,
} from "./theme/format";
import {
  effectiveTheme,
  prepareTheme,
  subscribeTheme,
  holdThemePreview,
} from "./theme/runtime";
interface Themes {
  preferences: ThemePreferences;
  ready: boolean;
  error: string;
  safeMode: boolean;
  fixedAppearance: Appearance | null;
  snapshot: ReturnType<typeof effectiveTheme>;
  reload: () => Promise<void>;
  select: (
    id: string | null,
    appearance?: AppearancePreference,
  ) => Promise<void>;
  selectIcons: (kind: IconKind, id: string | null) => Promise<void>;
  preview: (bundle: ThemeBundle) => Promise<void>;
  cancelPreview: () => Promise<void>;
}
const Context = createContext<Themes | null>(null);
export function ThemeProvider({ children }: { children: ReactNode }) {
  const [preferences, setPreferences] = useState(builtinPreferences),
    [ready, setReady] = useState(!native),
    [error, setError] = useState("");
  const [safeMode, setSafeMode] = useState(false),
    [fixedAppearance, setFixedAppearance] = useState<Appearance | null>(null),
    [directory, setDirectory] = useState("");
  const snapshot = useSyncExternalStore(subscribeTheme, effectiveTheme);
  const mounted = useRef(false),
    generation = useRef(0),
    saving = useRef(false),
    previewing = useRef<ThemeBundle | null>(null),
    synchronizing = useRef(0),
    applied = useRef<AppearancePreference>("system");
  const previewAppearance = useRef<AppearancePreference>("system");
  const builtin = useRef(false),
    previewBuiltin = useRef(false);
  const restorePreview = useRef<null | (() => Promise<void>)>(null);
  const latestPreferences = useRef(preferences);
  latestPreferences.current = preferences;
  const abort = useRef<AbortController | null>(null);
  const nativeQueue = useRef(Promise.resolve());
  const sync = (appearance: AppearancePreference) => {
    if (!native) return Promise.resolve();
    const operation = nativeQueue.current.then(async () => {
      ++synchronizing.current;
      try {
        await api("sync_theme_window", { appearance });
      } finally {
        --synchronizing.current;
      }
    });
    nativeQueue.current = operation.catch(() => {});
    return operation;
  };
  const syncBackground = (isBuiltin: boolean, request: number) => {
    if (!native) return Promise.resolve();
    const operation = nativeQueue.current.then(async () => {
      if (!mounted.current || request !== generation.current) return;
      await api("sync_theme_webview_background", {
        background: isBuiltin ? opaqueWindowBackground() : null,
      });
    });
    nativeQueue.current = operation.catch(() => {});
    // Failure leaves the transparent background established by sync(). The
    // committed theme remains usable even when this optional optimization fails.
    return operation.catch(console.error);
  };
  const apply = useCallback(
    async (
      bundle: ThemeBundle | null,
      prefs: ThemePreferences,
      request: number,
      sourceRevision?: number,
      icons?: { file: ThemeBundle | null; product: ThemeBundle | null },
    ) => {
      abort.current?.abort();
      const controller = new AbortController();
      abort.current = controller;
      const prepared = await prepareTheme(
        bundle,
        prefs,
        controller.signal,
        sourceRevision,
        icons,
      );
      if (!mounted.current || request !== generation.current) {
        prepared.dispose();
        return false;
      }
      try {
        await sync(prepared.appearance);
      } catch (error) {
        prepared.dispose();
        throw error;
      }
      if (!mounted.current || request !== generation.current) {
        prepared.dispose();
        return false;
      }
      await prepared.commit(() => {
        if (mounted.current && request === generation.current) setReady(true);
      });
      if (!mounted.current || request !== generation.current) return false;
      builtin.current = bundle === null;
      applied.current = prepared.appearance;
      await syncBackground(builtin.current, request);
      if (!mounted.current || request !== generation.current) return false;
      setFixedAppearance(
        prepared.manifest.appearance === "light" ||
          prepared.manifest.appearance === "dark"
          ? prepared.manifest.appearance
          : null,
      );
      return true;
    },
    [],
  );
  const reload = useCallback(async () => {
    if (!native || saving.current || previewing.current) return;
    const request = ++generation.current;
    try {
      const current = await api<ThemeCurrent>("load_theme_preferences");
      if (request !== generation.current) return;
      if (
        await apply(
          current.theme,
          current.preferences,
          request,
          current.revision,
          {
            file: current.fileIcons ?? null,
            product: current.productIcons ?? null,
          },
        )
      ) {
        setPreferences(current.preferences);
        setSafeMode(current.safeMode);
        setDirectory(
          JSON.stringify(
            [current.theme, current.fileIcons, current.productIcons].flatMap(
              (bundle) => (bundle?.directory ? [bundle.directory] : []),
            ),
          ),
        );
        setError("");
      }
    } catch (error) {
      if (mounted.current && request === generation.current)
        setError(
          `${errorMessage(error)} The last working appearance is still in use.`,
        );
    } finally {
      if (mounted.current && request === generation.current) setReady(true);
    }
  }, [apply]);
  useEffect(() => {
    mounted.current = true;
    let active = true;
    const stop = native
      ? listen("theme-changed", () => void reload())
      : Promise.resolve(() => {});
    void stop
      .then(() => {
        if (active) void reload();
      })
      .catch((error) => {
        if (active) {
          setError(errorMessage(error));
          setReady(true);
        }
      });
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const changed = () => {
      if (synchronizing.current || saving.current) return;
      if (applied.current === "system") {
        const bundle = previewing.current;
        if (bundle)
          void apply(
            bundle,
            latestPreferences.current,
            ++generation.current,
          ).catch((error) => setError(errorMessage(error)));
        else void reload();
      } else if (media.matches !== (applied.current === "dark")) {
        const request = generation.current;
        void sync(applied.current)
          .then(() => syncBackground(builtin.current, request))
          .catch((error) => setError(errorMessage(error)));
      }
    };
    window.addEventListener("focus", reload);
    media.addEventListener("change", changed);
    return () => {
      active = false;
      mounted.current = false;
      ++generation.current;
      abort.current?.abort();
      window.removeEventListener("focus", reload);
      media.removeEventListener("change", changed);
      void stop.then((stop) => stop()).catch(() => {});
    };
  }, [reload, apply]);
  useEffect(() => {
    if (!native || !directory || directory === "[]" || safeMode) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = watch(
      JSON.parse(directory) as string[],
      (event) => {
        if (typeof event.type === "object" && "access" in event.type) return;
        clearTimeout(timer);
        timer = setTimeout(() => {
          if (active) void reload();
        }, 350);
      },
      { recursive: true, delayMs: 200 },
    );
    void stop
      .then(() => {
        // Catch changes between the initial read and watch registration.
        if (active) void reload();
      })
      .catch((error) => {
        if (active)
          setError(
            `Theme watch failed: ${errorMessage(error)} Use Refresh to retry loading.`,
          );
      });
    return () => {
      active = false;
      clearTimeout(timer);
      void stop.then((stop) => stop()).catch(() => {});
    };
  }, [directory, safeMode, reload]);
  const select = async (
    id: string | null,
    appearance = preferences.appearance,
    iconSelection?: { kind: IconKind; id: string | null },
  ) => {
    if (saving.current) return;
    saving.current = true;
    const request = ++generation.current;
    try {
      const next: ThemePreferences = {
        ...preferences,
        version: 1,
        active: id,
        appearance,
      };
      if (iconSelection)
        next[iconSelection.kind === "file" ? "fileIcons" : "productIcons"] =
          iconSelection.id;
      const bundle = id ? await api<ThemeBundle>("load_theme", { id }) : null;
      const [file, product] = await Promise.all(
        [next.fileIcons, next.productIcons].map((id) =>
          id ? api<ThemeBundle>("load_theme", { id }) : null,
        ),
      );
      const prepared = await prepareTheme(bundle, next, undefined, undefined, {
        file,
        product,
      });
      try {
        await sync(prepared.appearance);
        if (native) await api("save_theme_preferences", { data: next });
        if (!mounted.current || request !== generation.current) {
          prepared.dispose();
          return;
        }
        previewing.current = null;
        await prepared.commit();
        if (!mounted.current || request !== generation.current) return;
        builtin.current = bundle === null;
        applied.current = prepared.appearance;
        await syncBackground(builtin.current, request);
        if (!mounted.current || request !== generation.current) return;
        setPreferences(next);
        setDirectory(
          JSON.stringify(
            [bundle, file, product].flatMap((bundle) =>
              bundle?.directory ? [bundle.directory] : [],
            ),
          ),
        );
        setFixedAppearance(
          prepared.manifest.appearance === "light" ||
            prepared.manifest.appearance === "dark"
            ? prepared.manifest.appearance
            : null,
        );
        setError("");
      } catch (error) {
        prepared.dispose();
        await sync(applied.current);
        throw error;
      }
    } finally {
      saving.current = false;
      await reload();
    }
  };
  const preview = async (bundle: ThemeBundle) => {
    const request = ++generation.current;
    const first = !restorePreview.current;
    if (first) {
      previewAppearance.current = applied.current;
      previewBuiltin.current = builtin.current;
      restorePreview.current = holdThemePreview();
    }
    const restore = restorePreview.current;
    try {
      if (await apply(bundle, preferences, request)) {
        previewing.current = bundle;
        setError("");
      }
    } catch (error) {
      if (
        first &&
        restore &&
        mounted.current &&
        request === generation.current &&
        restorePreview.current === restore
      ) {
        await sync(previewAppearance.current);
        if (
          !mounted.current ||
          request !== generation.current ||
          restorePreview.current !== restore
        )
          throw error;
        await restore();
        if (
          !mounted.current ||
          request !== generation.current ||
          restorePreview.current !== restore
        )
          throw error;
        restorePreview.current = null;
        builtin.current = previewBuiltin.current;
        await syncBackground(builtin.current, request);
      }
      throw error;
    }
  };
  const cancelPreview = async () => {
    const restore = restorePreview.current;
    if (restore) {
      const request = ++generation.current;
      abort.current?.abort();
      await sync(previewAppearance.current);
      if (
        !mounted.current ||
        request !== generation.current ||
        restorePreview.current !== restore
      )
        return;
      await restore();
      if (
        !mounted.current ||
        request !== generation.current ||
        restorePreview.current !== restore
      )
        return;
      restorePreview.current = null;
      applied.current = previewAppearance.current;
      builtin.current = previewBuiltin.current;
      await syncBackground(builtin.current, request);
      if (!mounted.current || request !== generation.current) return;
    }
    previewing.current = null;
    await reload();
  };
  return (
    <Context.Provider
      value={{
        preferences,
        ready,
        error,
        safeMode,
        fixedAppearance,
        snapshot,
        reload,
        select,
        selectIcons: (kind, id) =>
          select(preferences.active, preferences.appearance, { kind, id }),
        preview,
        cancelPreview,
      }}
    >
      {ready ? children : null}
    </Context.Provider>
  );
}
export function useThemes() {
  const context = useContext(Context);
  if (!context) throw new Error("ThemeProvider is missing.");
  return context;
}
