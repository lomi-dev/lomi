import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import {
  ArrowLeft,
  ArrowRight,
  ExternalLink,
  Globe,
  Search,
  RotateCw,
  X,
} from "./icons";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { BrowserTab } from "./model";
import { browserAddress } from "./browser-url";
import {
  browserAction,
  browserSnapshot,
  localServersSnapshot,
  mountBrowser,
  refreshLocalServers,
  subscribeBrowsers,
  subscribeLocalServers,
} from "./browser-runtime";
import { errorMessage } from "./api";
import { IconButton } from "./ui";

export default function BrowserPane({
  tab,
  onFocus,
  onClose,
  overview = false,
}: {
  tab: BrowserTab;
  onFocus?: () => void;
  onClose?: () => void;
  overview?: boolean;
}) {
  const page = useSyncExternalStore(subscribeBrowsers, () =>
    browserSnapshot(tab),
  );
  const host = useRef<HTMLDivElement>(null);
  const address = useRef<HTMLInputElement>(null);
  const suggestions = useRef<HTMLDivElement>(null);
  const suggestionsId = useId();
  const find = useRef<HTMLInputElement>(null);
  const handlers = useRef({ onFocus, onClose });
  handlers.current = { onFocus, onClose };
  const [value, setValue] = useState(tab.url === "about:blank" ? "" : tab.url);
  const [error, setError] = useState("");
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const [suggesting, setSuggesting] = useState(false);
  const { urls: servers, error: serverError } = useSyncExternalStore(
    subscribeLocalServers,
    localServersSnapshot,
  );
  const [filter, setFilter] = useState("");
  const [activeServer, setActiveServer] = useState("");
  const matches = (servers ?? []).filter((url) =>
    url.toLowerCase().includes(filter.toLowerCase()),
  );
  const activeIndex = matches.indexOf(activeServer);
  const closeSuggestions = () => suggestions.current?.hidePopover();
  const showSuggestions = () => {
    if (overview || suggestions.current?.matches(":popover-open")) return;
    setFilter("");
    setActiveServer("");
    suggestions.current?.showPopover();
  };
  const focusAddress = () => {
    address.current?.focus();
    address.current?.select();
    showSuggestions();
  };
  useEffect(() => {
    if (!suggesting) return;
    void refreshLocalServers();
    const timer = window.setInterval(() => void refreshLocalServers(), 5000);
    window.addEventListener("blur", closeSuggestions);
    return () => {
      clearInterval(timer);
      window.removeEventListener("blur", closeSuggestions);
    };
  }, [suggesting]);
  useLayoutEffect(() => {
    if (!suggesting) return;
    const popup = suggestions.current!;
    const input = address.current!;
    const position = () => {
      const rect = input.getBoundingClientRect();
      popup.style.left = `${rect.left}px`;
      popup.style.top = `${rect.bottom + 4}px`;
      popup.style.width = `${rect.width}px`;
      popup.style.maxHeight = `${Math.max(0, Math.min(280, innerHeight - rect.bottom - 12))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    observer.observe(input);
    window.addEventListener("resize", position);
    document.addEventListener("scroll", position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      document.removeEventListener("scroll", position, true);
    };
  }, [suggesting]);
  useLayoutEffect(() => {
    if (!suggesting || activeIndex < 0) return;
    const popup = suggestions.current!;
    const item = popup.querySelector<HTMLElement>('[aria-selected="true"]');
    if (!item) return;
    if (item.offsetTop < popup.scrollTop) popup.scrollTop = item.offsetTop;
    else if (
      item.offsetTop + item.offsetHeight >
      popup.scrollTop + popup.clientHeight
    )
      popup.scrollTop = item.offsetTop + item.offsetHeight - popup.clientHeight;
  }, [suggesting, activeIndex]);
  useEffect(() => {
    setValue(page.url === "about:blank" ? "" : page.url);
  }, [page.url]);
  useEffect(() => {
    if (tab.url === "about:blank") focusAddress();
  }, [tab.id]);
  useEffect(() => {
    if (searching) find.current?.focus();
  }, [searching]);
  useLayoutEffect(() => {
    if (overview) return;
    return mountBrowser(tab, host.current!, (signal) => {
      handlers.current.onFocus?.();
      if (signal === "address") focusAddress();
      if (signal === "close") handlers.current.onClose?.();
      if (signal === "find") setSearching(true);
    });
  }, [tab.id, overview]);
  const run = (action: Parameters<typeof browserAction>[1]) => {
    setError("");
    closeSuggestions();
    void browserAction(tab, action)
      .then(() =>
        action.type === "navigate" && action.url !== "about:blank"
          ? browserAction(tab, { type: "focus" })
          : undefined,
      )
      .catch((error) => setError(errorMessage(error)));
  };
  return (
    <section
      className="browser-pane"
      aria-label="Browser"
      data-browser-pane-id={tab.id}
      onPointerDownCapture={onFocus}
      onFocusCapture={onFocus}
      onKeyDown={(event) => {
        if (
          (event.ctrlKey || event.metaKey) &&
          event.key.toLowerCase() === "l"
        ) {
          event.preventDefault();
          focusAddress();
        }
        if (event.key === "F5") {
          event.preventDefault();
          if (tab.url !== "about:blank") run({ type: "reload" });
        }
      }}
    >
      <form
        className="browser-toolbar"
        data-pane-drag-handle
        aria-label="Browser navigation"
        onSubmit={(event) => {
          event.preventDefault();
          try {
            const url = browserAddress(
              suggesting && activeIndex >= 0 ? activeServer : value,
            );
            setValue(url === "about:blank" ? "" : url);
            run({ type: "navigate", url });
          } catch (error) {
            setError(errorMessage(error));
          }
        }}
      >
        {page.agentControlled && (
          <button
            type="button"
            className="browser-control"
            onClick={() => run({ type: "takeControl" })}
            title="End agent access and keep browsing in this isolated profile."
          >
            Agent · Take control
          </button>
        )}
        <IconButton
          title="Back"
          disabled={tab.url === "about:blank"}
          onClick={() => run({ type: "back" })}
        >
          <ArrowLeft size={14} />
        </IconButton>
        <IconButton
          title="Forward"
          disabled={tab.url === "about:blank"}
          onClick={() => run({ type: "forward" })}
        >
          <ArrowRight size={14} />
        </IconButton>
        <IconButton
          title={page.loading ? "Stop loading" : "Reload page"}
          disabled={tab.url === "about:blank"}
          onClick={() => run({ type: page.loading ? "stop" : "reload" })}
        >
          {page.loading ? <X size={14} /> : <RotateCw size={14} />}
        </IconButton>
        <input
          ref={address}
          aria-label="Web address"
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={suggesting}
          aria-controls={suggestionsId}
          aria-activedescendant={
            suggesting && activeIndex >= 0
              ? `${suggestionsId}-${activeIndex}`
              : undefined
          }
          placeholder="Search or enter address"
          value={value}
          spellCheck={false}
          autoCapitalize="off"
          autoComplete="off"
          onFocus={showSuggestions}
          onClick={showSuggestions}
          onBlur={closeSuggestions}
          onChange={(event) => {
            showSuggestions();
            setValue(event.target.value);
            setFilter(event.target.value.trim());
            setActiveServer("");
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing) return;
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              if (suggesting) closeSuggestions();
              else {
                setValue(page.url === "about:blank" ? "" : page.url);
                address.current?.blur();
              }
            } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              showSuggestions();
              if (matches.length) {
                const index =
                  activeIndex < 0
                    ? event.key === "ArrowDown"
                      ? 0
                      : matches.length - 1
                    : (activeIndex +
                        (event.key === "ArrowDown" ? 1 : -1) +
                        matches.length) %
                      matches.length;
                setActiveServer(matches[index]);
              }
            }
          }}
        />
        <div
          ref={suggestions}
          id={suggestionsId}
          popover="auto"
          className="select-menu browser-servers"
          role="listbox"
          aria-label="Local web servers"
          aria-busy={servers === null}
          onBeforeToggle={(event) => setSuggesting(event.newState === "open")}
          onPointerDown={(event) => event.preventDefault()}
        >
          <div className="browser-message" role="presentation">
            Local web servers
          </div>
          {matches.map((url, index) => (
            <div
              key={url}
              id={`${suggestionsId}-${index}`}
              className={`select-option${activeServer === url ? " is-highlighted" : ""}`}
              role="option"
              aria-selected={activeServer === url}
              onPointerMove={() => setActiveServer(url)}
              onClick={() => {
                setValue(url);
                run({ type: "navigate", url: browserAddress(url) });
              }}
            >
              <Globe size={14} aria-hidden="true" />
              <span>{url}</span>
            </div>
          ))}
          {!matches.length && (
            <div className="browser-message" role="status">
              {serverError ||
                (servers === null
                  ? "Looking for local servers…"
                  : servers.length
                    ? "No matching servers."
                    : "No local HTTP servers found.")}
            </div>
          )}
        </div>
        <IconButton
          title="Open in default browser"
          disabled={tab.url === "about:blank"}
          onClick={() => {
            void openUrl(tab.url).catch((error) =>
              setError(errorMessage(error)),
            );
          }}
        >
          <ExternalLink size={14} />
        </IconButton>
        {onClose && (
          <IconButton title="Close browser panel" onClick={onClose}>
            <X size={14} />
          </IconButton>
        )}
      </form>
      {searching && (
        <form
          className="browser-search"
          onSubmit={(event) => {
            event.preventDefault();
            run({ type: "find", text: query, backwards: false });
          }}
        >
          <Search size={14} />
          <input
            ref={find}
            aria-label="Find in page"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                setSearching(false);
              }
              if (event.key === "Enter" && event.shiftKey) {
                event.preventDefault();
                run({ type: "find", text: query, backwards: true });
              }
            }}
          />
          <IconButton
            title="Find previous"
            onClick={() => run({ type: "find", text: query, backwards: true })}
          >
            <ArrowLeft size={14} />
          </IconButton>
          <IconButton
            title="Find next"
            onClick={() => run({ type: "find", text: query, backwards: false })}
          >
            <ArrowRight size={14} />
          </IconButton>
          <IconButton
            title="Close page search"
            onClick={() => setSearching(false)}
          >
            <X size={14} />
          </IconButton>
        </form>
      )}
      {(error || page.error) && (
        <div className="browser-message" role="alert">
          {error || page.error}
        </div>
      )}
      {page.download && (
        <div className="browser-message" role="status">
          {page.download}
        </div>
      )}
      <div
        className="browser-viewport"
        ref={host}
        aria-label="Web page"
        aria-busy={page.loading}
      >
        {(tab.url === "about:blank" || overview) && (
          <div className="empty-message">
            <Globe size={28} />
            <h2>{overview ? tab.title : "Browse the web"}</h2>
            <p>{overview ? tab.url : "Enter a web address or search above."}</p>
          </div>
        )}
      </div>
    </section>
  );
}
