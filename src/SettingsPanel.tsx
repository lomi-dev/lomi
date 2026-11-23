import {
  lazy,
  Suspense,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { X } from "./icons";
import { useT } from "./i18n";
import SettingsNavigation, { settingsPageLabel } from "./SettingsNavigation";
import AboutSettingsPage from "./AboutSettingsPage";
import GeneralSettingsPage from "./GeneralSettingsPage";
import TerminalSettingsPage from "./TerminalSettingsPage";
import KeybindsSettingsPage from "./KeybindsSettingsPage";
import { useProtectedTheme } from "./useProtectedTheme";
import { defaultSettingsPage } from "./settings-pages";
import type { SettingsPageId } from "./settings-pages";
import { SettingsPage } from "./settings-ui";
import {
  findRevealTarget,
  showRevealTarget,
  type SettingsSearchEntry,
} from "./settings-search";

const PluginsPage = lazy(() => import("./plugins/PluginsPage"));
const ProvidersSettingsPage = lazy(
  () => import("./ai-chat/settings/ProvidersSettingsPage"),
);
const DefaultsSettingsPage = lazy(
  () => import("./ai-chat/settings/DefaultsSettingsPage"),
);
const ThemesPage = lazy(() => import("./ThemesPage"));
const NotificationSettingsPage = lazy(
  () => import("./auth/NotificationSettingsPage"),
);
const RemoteSettingsPage = lazy(() => import("./RemoteSettingsPage"));
const AccountSettingsPage = lazy(() => import("./auth/AccountSettingsPage"));
const AndroidSettingsPage = lazy(() => import("./android/AndroidSettingsPage"));
const AgentControlSettingsPage = lazy(
  () => import("./AgentControlSettingsPage"),
);

function SettingsLoading({ title }: { title: string }) {
  const { t } = useT();
  return <SettingsPage title={title} status={t("Loading…")} />;
}

export interface SettingsPanelProps {
  active: boolean;
  requestedPage: SettingsPageId | null;
  requestRevision: number;
  remoteAvailable: boolean;
  remoteDisabledReason?: string;
  onClose?: () => void;
}

export default function SettingsPanel({
  active,
  requestedPage,
  requestRevision,
  remoteAvailable,
  remoteDisabledReason,
  onClose,
}: SettingsPanelProps) {
  const { t } = useT();
  const disabledReason =
    remoteDisabledReason ?? t("Remote is disabled in this version of Lomi.");
  const [page, setPage] = useState<SettingsPageId>(
    requestedPage ?? defaultSettingsPage,
  );
  useProtectedTheme(
    active &&
      (page === "agent-control" ||
        page === "account" ||
        page === "remote" ||
        page === "notifications" ||
        page === "chat-providers"),
  );
  const [error, setError] = useState("");
  const panel = useRef<HTMLDivElement>(null);
  const revealFrame = useRef(0);
  useEffect(() => () => cancelAnimationFrame(revealFrame.current), []);
  // Lazy pages and tabs mount later, so poll for the target for up to 2s.
  const reveal = (entry: SettingsSearchEntry) => {
    cancelAnimationFrame(revealFrame.current);
    setPage(entry.page);
    const started = performance.now();
    const attempt = () => {
      const elapsed = performance.now() - started;
      const root = panel.current?.querySelector<HTMLElement>(".settings-page");
      if (root && entry.tab) {
        const tab = root.querySelector<HTMLElement>(
          `#agent-control-tab-${entry.tab}`,
        );
        if (tab && tab.getAttribute("aria-selected") !== "true") tab.click();
      }
      const target = root && findRevealTarget(root, entry, elapsed > 800);
      if (target) showRevealTarget(target);
      else if (elapsed < 2000)
        revealFrame.current = requestAnimationFrame(attempt);
    };
    // Wait one frame so the new page replaces the old one first.
    revealFrame.current = requestAnimationFrame(attempt);
  };
  useLayoutEffect(() => {
    if (requestedPage !== null) setPage(requestedPage);
  }, [requestedPage, requestRevision]);

  return (
    <div className="settings-panel" ref={panel}>
      <div className="settings-layout">
        <SettingsNavigation
          active={active}
          page={page}
          requestRevision={requestRevision}
          remoteAvailable={remoteAvailable}
          remoteDisabledReason={disabledReason}
          onSelect={setPage}
          onReveal={reveal}
        />
        <div className="settings-main">
          <header className="settings-topbar">
            <p className="settings-breadcrumb">
              <span>{t("Settings")}</span>
              <span aria-hidden="true">/</span>
              <span aria-current="page">{t(settingsPageLabel(page))}</span>
            </p>
            {onClose && (
              <button
                type="button"
                className="icon-button settings-close"
                aria-label={t("Close settings")}
                title={t("Close settings")}
                onClick={onClose}
              >
                <X size={16} aria-hidden="true" />
              </button>
            )}
          </header>
          {page === "notifications" ? (
            <Suspense fallback={<SettingsLoading title={t("Notifications")} />}>
              <NotificationSettingsPage />
            </Suspense>
          ) : page === "remote" ? (
            <Suspense fallback={<SettingsLoading title={t("Remote")} />}>
              <RemoteSettingsPage
                available={remoteAvailable}
                disabledReason={disabledReason}
              />
            </Suspense>
          ) : page === "account" ? (
            <Suspense fallback={<SettingsLoading title={t("Account")} />}>
              <AccountSettingsPage
                remoteAvailable={remoteAvailable}
                onNavigate={setPage}
              />
            </Suspense>
          ) : page === "agent-control" ? (
            <Suspense fallback={<SettingsLoading title={t("Agent control")} />}>
              <AgentControlSettingsPage active={active} />
            </Suspense>
          ) : page === "android" ? (
            <Suspense fallback={<SettingsLoading title={t("Android")} />}>
              <AndroidSettingsPage />
            </Suspense>
          ) : page === "chat-providers" ? (
            <Suspense fallback={<SettingsLoading title={t("Providers")} />}>
              <ProvidersSettingsPage />
            </Suspense>
          ) : page === "chat-defaults" ? (
            <Suspense fallback={<SettingsLoading title={t("Defaults")} />}>
              <DefaultsSettingsPage />
            </Suspense>
          ) : page === "about" ? (
            <AboutSettingsPage
              error={error}
              onError={setError}
              onClearError={() => setError("")}
            />
          ) : page === "plugins" ? (
            <Suspense fallback={<SettingsLoading title={t("Plugins")} />}>
              <PluginsPage />
            </Suspense>
          ) : page === "general" ? (
            <GeneralSettingsPage />
          ) : page === "terminal" ? (
            <TerminalSettingsPage />
          ) : page === "themes" ? (
            <Suspense fallback={<SettingsLoading title={t("Themes")} />}>
              <ThemesPage />
            </Suspense>
          ) : (
            <KeybindsSettingsPage active={active} />
          )}
        </div>
      </div>
    </div>
  );
}
