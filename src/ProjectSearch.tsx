import ResourceIcon from "./ResourceIcon";
import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  CaseSensitive,
  ChevronDown,
  ChevronRight,
  ChevronsDownUp,
  ChevronsUpDown,
  Ellipsis,
  ListX,
  RefreshCw,
  Regex,
  WholeWord,
  X,
} from "./icons";
import { api, errorMessage } from "./api";
import { basename } from "./model";
import { parentPath } from "./file-path";
import { IconButton } from "./ui";

export interface SearchMatch {
  relative: string;
  line: number;
  column: number;
  length: number;
  preview: string;
  previewStart: number;
}
interface SearchResults {
  matches: SearchMatch[];
  limited: boolean;
  skipped: number;
}

export default function ProjectSearch({
  root,
  relative,
  hidden,
  onClose,
  onScope,
  onOpenFile,
}: {
  root: string;
  relative: string;
  hidden: boolean;
  onClose: () => void;
  onScope: (relative: string) => void;
  onOpenFile: (relative: string, match: SearchMatch) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const resultList = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState("");
  const [options, setOptions] = useState({
    caseSensitive: false,
    wholeWord: false,
    regex: false,
    includeIgnored: false,
    include: "",
    exclude: "",
  });
  const [filtersOpen, setFiltersOpen] = useState(false);
  const [composing, setComposing] = useState(false);
  const [collapsed, setCollapsed] = useState(new Set<string>());
  const [selected, setSelected] = useState("");
  const [results, setResults] = useState<SearchResults>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    if (!hidden) {
      input.current?.focus();
      input.current?.select();
    }
  }, [hidden]);
  useEffect(() => {
    let current = true;
    let started = false;
    if (!query) setResults(undefined);
    setError("");
    setBusy(!!query);
    if (composing) return;
    const timer = setTimeout(() => {
      if (!query) return;
      started = true;
      void api<SearchResults>("search_project", {
        root,
        relative,
        query,
        options,
      })
        .then((result) => {
          if (current) {
            setResults(result);
            setCollapsed(new Set());
            setSelected("");
          }
        })
        .catch((error) => {
          if (current) {
            setResults(undefined);
            setError(errorMessage(error));
          }
        })
        .finally(() => {
          if (current) setBusy(false);
        });
    }, 250);
    return () => {
      current = false;
      clearTimeout(timer);
      if (started) void api("cancel_project_search").catch(() => {});
    };
  }, [root, relative, query, options, revision, composing]);
  const groups = new Map<string, SearchMatch[]>();
  for (const match of results?.matches ?? []) {
    const group = groups.get(match.relative) ?? [];
    group.push(match);
    groups.set(match.relative, group);
  }
  const allCollapsed = groups.size > 0 && collapsed.size === groups.size;
  const toggleGroup = (path: string) =>
    setCollapsed((previous) => {
      const next = new Set(previous);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  const refresh = () => setRevision((revision) => revision + 1);
  return (
    <div className="sidebar-panel project-search" hidden={hidden}>
      <header className="sidebar-heading">
        <span>SEARCH</span>
        <div>
          <IconButton
            title="Refresh search"
            disabled={!query || composing}
            onClick={refresh}
          >
            <RefreshCw size={14} />
          </IconButton>
          <IconButton
            title="Clear search results"
            disabled={!query}
            onClick={() => {
              setQuery("");
              input.current?.focus();
            }}
          >
            <ListX size={14} />
          </IconButton>
          <IconButton
            title={allCollapsed ? "Expand all results" : "Collapse all results"}
            disabled={!groups.size || busy}
            onClick={() =>
              setCollapsed(allCollapsed ? new Set() : new Set(groups.keys()))
            }
          >
            {allCollapsed ? (
              <ChevronsUpDown size={14} />
            ) : (
              <ChevronsDownUp size={14} />
            )}
          </IconButton>
          <IconButton title="Back to Explorer" onClick={onClose}>
            <ArrowLeft size={14} />
          </IconButton>
        </div>
      </header>
      <form
        className="project-search-form"
        onSubmit={(event) => {
          event.preventDefault();
          if (!composing) refresh();
        }}
      >
        <div className="project-search-input">
          <input
            ref={input}
            aria-label="Search in files"
            title="Search saved file contents"
            placeholder="Search"
            value={query}
            spellCheck={false}
            autoComplete="off"
            onChange={(event) => setQuery(event.target.value)}
            onCompositionStart={() => setComposing(true)}
            onCompositionEnd={() => setComposing(false)}
            onKeyDown={(event) => {
              if (event.nativeEvent.isComposing || composing) return;
              if (
                event.key === "ArrowDown" &&
                !event.altKey &&
                !event.ctrlKey &&
                !event.metaKey &&
                !event.shiftKey
              ) {
                const first =
                  resultList.current?.querySelector<HTMLButtonElement>(
                    "button:not(:disabled)",
                  );
                if (first) {
                  event.preventDefault();
                  first.focus();
                }
              }
            }}
          />
          <div className="project-search-options">
            {(
              [
                ["caseSensitive", "Match case", CaseSensitive],
                ["wholeWord", "Match whole word", WholeWord],
                ["regex", "Use regular expression", Regex],
              ] as const
            ).map(([key, title, Icon]) => (
              <IconButton
                key={key}
                title={title}
                aria-pressed={options[key]}
                onClick={() => setOptions({ ...options, [key]: !options[key] })}
              >
                <Icon size={16} />
              </IconButton>
            ))}
          </div>
        </div>
        <div className="project-search-scope">
          {relative && (
            <>
              <span title={`${root}/${relative}`}>{relative}</span>
              <IconButton
                title="Search entire project"
                onClick={() => onScope("")}
              >
                <X size={12} />
              </IconButton>
            </>
          )}
          <IconButton
            title="Toggle search details"
            aria-expanded={filtersOpen}
            className={`icon-button search-details-toggle${options.include || options.exclude || options.includeIgnored ? " is-active" : ""}`}
            onClick={() => setFiltersOpen(!filtersOpen)}
          >
            <Ellipsis size={16} />
          </IconButton>
        </div>
        {filtersOpen && (
          <div className="project-search-filters">
            <label>
              Files to include
              <input
                placeholder="e.g. *.ts, src/**"
                title="Comma-separated glob patterns, relative to the project. Use ./ to anchor a path."
                value={options.include}
                spellCheck={false}
                onChange={(event) =>
                  setOptions({ ...options, include: event.target.value })
                }
              />
            </label>
            <label>
              Files to exclude
              <input
                placeholder="e.g. dist, **/*.test.ts"
                title="Comma-separated glob patterns, relative to the project."
                value={options.exclude}
                spellCheck={false}
                onChange={(event) =>
                  setOptions({ ...options, exclude: event.target.value })
                }
              />
            </label>
            <label className="search-ignore-option">
              <input
                type="checkbox"
                checked={options.includeIgnored}
                onChange={(event) =>
                  setOptions({
                    ...options,
                    includeIgnored: event.target.checked,
                  })
                }
              />
              Include ignored files
            </label>
          </div>
        )}
      </form>
      <div
        ref={resultList}
        className="project-search-results"
        aria-label="Search results"
        aria-busy={busy}
        onKeyDown={(event) => {
          if (
            event.nativeEvent.isComposing ||
            event.altKey ||
            event.ctrlKey ||
            event.metaKey ||
            event.shiftKey
          )
            return;
          const buttons = [
            ...event.currentTarget.querySelectorAll<HTMLButtonElement>(
              "button:not(:disabled)",
            ),
          ];
          const index = buttons.indexOf(event.target as HTMLButtonElement);
          if (index < 0) return;
          const group = (event.target as HTMLElement).closest(
            ".search-file-group",
          )!;
          const heading = group.querySelector<HTMLButtonElement>(
            ".search-file-heading",
          )!;
          if (
            [
              "ArrowDown",
              "ArrowUp",
              "Home",
              "End",
              "Escape",
              "ArrowLeft",
              "ArrowRight",
            ].includes(event.key)
          ) {
            event.preventDefault();
            event.stopPropagation();
            if (
              event.key === "Escape" ||
              (event.key === "ArrowUp" && index === 0)
            )
              input.current?.focus();
            else if (event.key === "ArrowLeft") {
              if (event.target !== heading) heading.focus();
              else if (heading.getAttribute("aria-expanded") === "true")
                heading.click();
            } else if (event.key === "ArrowRight") {
              if (event.target === heading) {
                if (heading.getAttribute("aria-expanded") === "false")
                  heading.click();
                else buttons[index + 1]?.focus();
              }
            } else {
              const next =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? buttons.length - 1
                    : index + (event.key === "ArrowDown" ? 1 : -1);
              buttons[next]?.focus();
            }
          }
        }}
      >
        {(busy || (!!query && results?.matches.length === 0)) && (
          <p className="tree-message" role="status">
            {busy ? "Searching…" : `No results for “${query}”.`}
          </p>
        )}
        {error && (
          <p className="tree-message text-error" role="alert">
            {error}
          </p>
        )}
        {[...groups]
          .sort(([a], [b]) => a.localeCompare(b))
          .map(([path, matches]) => (
            <section key={path} className="search-file-group">
              <button
                type="button"
                className="search-file-heading"
                title={path}
                aria-label={`${path}, ${matches.length} ${matches.length === 1 ? "result" : "results"}`}
                aria-expanded={!collapsed.has(path)}
                disabled={busy}
                onClick={() => toggleGroup(path)}
              >
                {collapsed.has(path) ? (
                  <ChevronRight size={12} />
                ) : (
                  <ChevronDown size={12} />
                )}
                <ResourceIcon path={path} size={14} />
                <span className="search-file-name">{basename(path, root)}</span>
                <span className="search-file-directory">
                  {parentPath(path, root)}
                </span>
                <span className="search-file-count">{matches.length}</span>
              </button>
              {!collapsed.has(path) &&
                matches.map((match) => {
                  const start = match.column - 1 - match.previewStart;
                  const key = `${path}:${match.line}:${match.column}`;
                  return (
                    <button
                      key={key}
                      type="button"
                      className={`search-match${selected === key ? " is-selected" : ""}`}
                      title={`${path}:${match.line}:${match.column}\n${match.preview}`}
                      aria-label={`${path}, line ${match.line}, column ${match.column}`}
                      aria-current={selected === key ? "true" : undefined}
                      disabled={busy}
                      onClick={() => {
                        setSelected(key);
                        onOpenFile(path, match);
                      }}
                    >
                      <span className="search-line-number">{match.line}</span>
                      <code>
                        {match.previewStart > 0 && "…"}
                        {match.preview.slice(0, start)}
                        <mark>
                          {match.preview.slice(start, start + match.length)}
                        </mark>
                        {match.preview.slice(start + match.length)}
                      </code>
                    </button>
                  );
                })}
            </section>
          ))}
      </div>
    </div>
  );
}
