import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode, RefObject, SVGProps } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import ContextMenu from "./ContextMenu";
import { api, errorMessage } from "./api";
import {
  ChevronDown,
  FolderGit2,
  FolderOpen,
  FolderPlus,
  Link,
  Lock,
  Plus,
  Search,
} from "./icons";
import { Modal } from "./ui";
import { currentLocale, msg, t, useT } from "./i18n";
import "./project-sources.css";

export type SourceKind = "new" | "local" | "url" | "github";

export interface GithubStatus {
  available: boolean;
  login?: string | null;
  reason?: "missing" | "signed-out" | string | null;
}

interface GithubRepo {
  fullName: string;
  description?: string | null;
  private: boolean;
  pushedAt?: string | null;
}

// Simple Icons GitHub mark; Lucide no longer ships brand icons.
export function GithubMark(props: SVGProps<SVGSVGElement> & { size?: number }) {
  const { size = 16, ...rest } = props;
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      {...rest}
    >
      <path d="M12 .3a12 12 0 0 0-3.8 23.4c.6.1.8-.3.8-.6v-2c-3.3.7-4-1.6-4-1.6-.6-1.4-1.4-1.8-1.4-1.8-1-.7.1-.7.1-.7 1.2 0 1.9 1.2 1.9 1.2 1 1.8 2.8 1.3 3.5 1 0-.8.4-1.3.7-1.6-2.7-.3-5.5-1.3-5.5-6 0-1.2.5-2.3 1.3-3.1-.2-.4-.6-1.6 0-3.2 0 0 1-.3 3.4 1.2a11.5 11.5 0 0 1 6 0c2.3-1.5 3.3-1.2 3.3-1.2.6 1.6.2 2.8 0 3.2.9.8 1.3 1.9 1.3 3.2 0 4.6-2.8 5.6-5.5 5.9.5.4.9 1 .9 2.2v3.3c0 .3.1.7.8.6A12 12 0 0 0 12 .3" />
    </svg>
  );
}

export const SOURCES: {
  kind: SourceKind;
  label: string;
  text: string;
  Icon: (props: { size?: number }) => ReactNode;
}[] = [
  {
    kind: "new",
    label: msg("New project"),
    text: msg("Start a new Git repository from a name"),
    Icon: FolderGit2,
  },
  {
    kind: "local",
    label: msg("Local folder"),
    text: msg("Browse a folder on disk"),
    Icon: FolderPlus,
  },
  {
    kind: "url",
    label: msg("Git URL"),
    text: msg("Clone from a remote URL"),
    Icon: Link,
  },
  {
    kind: "github",
    label: msg("GitHub repository"),
    text: msg("Clone GitHub owner/repo"),
    Icon: GithubMark,
  },
];

export const githubHint = (status: GithubStatus | null) =>
  !status
    ? t("Checking GitHub…")
    : status.available
      ? t("Signed in as {login}", { login: String(status.login) })
      : status.reason === "missing"
        ? t("Install the GitHub CLI (gh) to list your repositories")
        : t("Run gh auth login in a terminal to connect GitHub");

let statusRequest: Promise<GithubStatus> | null = null;
// One in-flight check shared by Welcome, the project menu and the dialog; each
// new mount starts a fresh check once the previous one settles.
function loadGithubStatus() {
  statusRequest ??= api<GithubStatus>("project_source_github_status")
    .catch((): GithubStatus => ({ available: false, reason: "unavailable" }))
    .finally(() => {
      statusRequest = null;
    });
  return statusRequest;
}

export function useGithubStatus() {
  const [status, setStatus] = useState<GithubStatus | null>(null);
  useEffect(() => {
    let live = true;
    void loadGithubStatus().then((next) => live && setStatus(next));
    return () => {
      live = false;
    };
  }, []);
  return status;
}

// "Add project ▾": one entry point to every source, shared by Welcome and
// the empty Workspaces sidebar.
export function AddProjectButton({
  busy = false,
  className = "button button-primary",
  onSource,
}: {
  busy?: boolean;
  className?: string;
  onSource: (kind: SourceKind) => void;
}) {
  const { t } = useT();
  const button = useRef<HTMLButtonElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  return (
    <>
      <button
        ref={button}
        type="button"
        className={`add-project ${className}`}
        aria-haspopup="menu"
        aria-expanded={!!menu}
        aria-disabled={busy}
        onClick={(event) => {
          if (busy) return;
          const bounds = event.currentTarget.getBoundingClientRect();
          setMenu(menu ? null : { x: bounds.left, y: bounds.bottom + 6 });
        }}
      >
        <Plus size={15} aria-hidden="true" />
        {t("Add project")}
        <ChevronDown size={14} aria-hidden="true" />
      </button>
      {menu && (
        <ProjectSourcesMenu
          {...menu}
          trigger={button}
          onSource={onSource}
          onClose={() => setMenu(null)}
        />
      )}
    </>
  );
}

// Mounted only while open, so GitHub is checked when the menu is shown.
export function ProjectSourcesMenu({
  x,
  y,
  align,
  trigger,
  onSource,
  onClose,
}: {
  x: number;
  y: number;
  align?: "start" | "end";
  trigger: RefObject<HTMLButtonElement | null>;
  onSource: (kind: SourceKind) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const github = useGithubStatus();
  return (
    <ContextMenu
      x={x}
      y={y}
      align={align}
      label={t("Add project")}
      className="project-sources-menu"
      showDescriptions
      trigger={trigger}
      actions={SOURCES.map(({ kind, label, text, Icon }) => {
        const unavailable = kind === "github" && !github?.available;
        return {
          label: t(label),
          description: unavailable ? githubHint(github) : t(text),
          icon: <Icon size={15} />,
          disabled: unavailable,
          run: () => onSource(kind),
        };
      })}
      onClose={onClose}
    />
  );
}

const PARENT_KEY = "lomi.project-parent.v1";
export function rememberedParent() {
  try {
    return localStorage.getItem(PARENT_KEY);
  } catch {
    return null;
  }
}

export function rememberParent(path: string) {
  try {
    localStorage.setItem(PARENT_KEY, path);
  } catch {
    // Only the default location is lost.
  }
}

const separator = (path: string) => (path.includes("\\") ? "\\" : "/");
const parentOf = (path: string) => {
  const index = path.replace(/[\\/]+$/, "").lastIndexOf(separator(path));
  return index > 0 ? path.slice(0, index) : path;
};
const nameFromUrl = (url: string) =>
  url
    .trim()
    .replace(/[\\/]+$/, "")
    .split(/[/:]/)
    .pop()
    ?.replace(/\.git$/, "") ?? "";

function relative(date?: string | null) {
  if (!date) return "";
  const days = Math.round((Date.parse(date) - Date.now()) / 86_400_000);
  if (Number.isNaN(days)) return "";
  const format = new Intl.RelativeTimeFormat(currentLocale(), {
    numeric: "auto",
  });
  if (days > -1) return t("today");
  if (days > -30) return format.format(days, "day");
  if (days > -365) return format.format(Math.round(days / 30), "month");
  return format.format(Math.round(days / 365), "year");
}

const TITLES: Record<Exclude<SourceKind, "local">, string> = {
  new: msg("New project"),
  url: msg("Clone from Git URL"),
  github: msg("Clone from GitHub"),
};

export default function OpenProjectDialog({
  kind,
  home,
  currentProject,
  onOpen,
  onClose,
}: {
  kind: Exclude<SourceKind, "local">;
  home: string;
  currentProject?: string;
  onOpen: (path: string) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const [parent, setParent] = useState(
    () =>
      rememberedParent() ?? (currentProject ? parentOf(currentProject) : home),
  );
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [url, setUrl] = useState("");
  const [query, setQuery] = useState("");
  const [repo, setRepo] = useState("");
  const [repos, setRepos] = useState<GithubRepo[] | null>(null);
  const [reposError, setReposError] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const status = useGithubStatus();
  const first = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (kind !== "github" || !status?.available) return;
    let live = true;
    api<GithubRepo[]>("project_source_github_repos")
      .then((list) => live && setRepos(list))
      .catch((reason) => live && setReposError(errorMessage(reason)));
    return () => {
      live = false;
    };
  }, [kind, status?.available]);

  const filtered = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (repos ?? []).filter(
      (item) =>
        !needle ||
        item.fullName.toLowerCase().includes(needle) ||
        item.description?.toLowerCase().includes(needle),
    );
  }, [repos, query]);

  // Typing an exact owner/repo is a valid choice even when it is not listed.
  const typedRepo = /^[A-Za-z0-9-]+\/[A-Za-z0-9._-]+$/.test(query.trim())
    ? query.trim()
    : "";
  const chosenRepo = repo || typedRepo;
  const suggested =
    kind === "url"
      ? nameFromUrl(url)
      : kind === "github"
        ? (chosenRepo.split("/")[1] ?? "")
        : "";
  const finalName = (nameEdited || kind === "new" ? name : suggested).trim();
  const ready =
    !busy &&
    !!finalName &&
    !!parent &&
    (kind === "new" ||
      (kind === "url" && !!url.trim()) ||
      (kind === "github" && !!chosenRepo && !!status?.available));
  const sep = separator(parent);

  const chooseParent = async () => {
    const picked = await open({
      directory: true,
      multiple: false,
      defaultPath: parent,
      title: t("Choose where to put the project"),
    });
    if (typeof picked === "string") setParent(picked);
  };

  const submit = async () => {
    if (!ready) return;
    setBusy(true);
    setError("");
    try {
      const path =
        kind === "new"
          ? await api<string>("project_source_create", {
              parent,
              name: finalName,
            })
          : await api<string>("project_source_clone", {
              parent,
              name: finalName,
              source:
                kind === "url"
                  ? { kind: "url", url: url.trim() }
                  : { kind: "github", repo: chosenRepo },
            });
      rememberParent(parent);
      onOpen(path);
      onClose();
    } catch (reason) {
      setError(errorMessage(reason));
      setBusy(false);
    }
  };

  const close = () => {
    if (busy && kind !== "new") void api("project_source_cancel");
    else if (!busy) onClose();
  };

  return (
    <Modal
      title={t(TITLES[kind])}
      className="project-source-dialog"
      onClose={close}
      initialFocus={first}
    >
      <form
        className="dialog-form dialog-layout"
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <div className="dialog-content project-source-content">
          {kind === "url" && (
            <label>
              {t("Repository URL")}
              <input
                ref={first}
                value={url}
                disabled={busy}
                placeholder="https://github.com/owner/repo.git"
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
                onChange={(event) => setUrl(event.target.value)}
              />
            </label>
          )}

          {kind === "github" &&
            (!status ? (
              <p className="project-source-status" role="status">
                <span className="project-source-spinner" aria-hidden="true" />
                {t("Checking GitHub…")}
              </p>
            ) : !status.available ? (
              <div className="project-source-empty">
                <GithubMark size={22} />
                <strong>{t("GitHub is not connected")}</strong>
                <p>
                  {status.reason === "missing"
                    ? t(
                        "Install the gh CLI, then run gh auth login in a terminal.",
                      )
                    : t(
                        "Run gh auth login in a terminal, then open this dialog again.",
                      )}
                </p>
              </div>
            ) : (
              <>
                <label className="project-source-search">
                  <Search size={14} aria-hidden="true" />
                  <input
                    ref={first}
                    aria-label={t("Search or type owner/repo")}
                    autoFocus
                    value={query}
                    disabled={busy}
                    placeholder={t(
                      "Search {login}’s repositories or type owner/repo",
                      { login: String(status.login) },
                    )}
                    spellCheck={false}
                    autoCapitalize="off"
                    onChange={(event) => {
                      setQuery(event.target.value);
                      setRepo("");
                    }}
                  />
                </label>
                <div
                  className="project-source-repos"
                  role="listbox"
                  aria-label={t("Your repositories")}
                  aria-busy={repos === null && !reposError}
                >
                  {reposError ? (
                    <p role="alert">{reposError}</p>
                  ) : repos === null ? (
                    <p className="project-source-status" role="status">
                      <span
                        className="project-source-spinner"
                        aria-hidden="true"
                      />
                      {t("Loading your repositories…")}
                    </p>
                  ) : filtered.length === 0 ? (
                    <p className="project-source-status">
                      {typedRepo
                        ? t("Clone {repo} directly.", { repo: typedRepo })
                        : t("No repositories match.")}
                    </p>
                  ) : (
                    filtered.map((item) => (
                      <button
                        key={item.fullName}
                        type="button"
                        role="option"
                        aria-selected={repo === item.fullName}
                        className="project-source-repo"
                        disabled={busy}
                        onClick={() => setRepo(item.fullName)}
                      >
                        <span>
                          <strong>
                            {item.fullName}
                            {item.private && (
                              <Lock size={11} aria-label={t("Private")} />
                            )}
                          </strong>
                          {item.description && (
                            <small>{item.description}</small>
                          )}
                        </span>
                        <time dateTime={item.pushedAt ?? undefined}>
                          {relative(item.pushedAt)}
                        </time>
                      </button>
                    ))
                  )}
                </div>
              </>
            ))}

          {(kind !== "github" || status?.available) && (
            <div className="project-source-fields">
              <label>
                {kind === "new" ? t("Project name") : t("Folder name")}
                <input
                  ref={kind === "new" ? first : undefined}
                  value={nameEdited || kind === "new" ? name : suggested}
                  disabled={busy}
                  placeholder={kind === "new" ? "my-app" : t("repository")}
                  spellCheck={false}
                  autoCapitalize="off"
                  maxLength={255}
                  onChange={(event) => {
                    setName(event.target.value);
                    setNameEdited(true);
                  }}
                />
              </label>
              <div className="project-source-location">
                <span>{t("Location")}</span>
                <button
                  type="button"
                  className="project-source-path"
                  disabled={busy}
                  title={parent}
                  onClick={() => void chooseParent()}
                >
                  <FolderOpen size={14} aria-hidden="true" />
                  <span>
                    {parent}
                    {finalName && (
                      <b>
                        {sep}
                        {finalName}
                      </b>
                    )}
                  </span>
                  <em>{t("Change")}</em>
                </button>
              </div>
            </div>
          )}

          {busy && kind !== "new" && (
            <p className="project-source-status" role="status">
              <span className="project-source-spinner" aria-hidden="true" />
              {kind === "github" && chosenRepo
                ? t("Cloning {repo}…", { repo: chosenRepo })
                : t("Cloning…")}
            </p>
          )}
          {error && <p role="alert">{error}</p>}
        </div>
        <div className="dialog-actions">
          <button type="button" className="button" onClick={close}>
            {busy && kind !== "new" ? t("Cancel clone") : t("Cancel")}
          </button>
          <button className="button button-primary" disabled={!ready}>
            {kind === "new"
              ? busy
                ? t("Creating…")
                : t("Create project")
              : busy
                ? t("Cloning…")
                : t("Clone")}
          </button>
        </div>
      </form>
    </Modal>
  );
}
